// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Concurrency-policy check.
//!
//! `splot` uses exactly one data-parallel engine (Rayon, via a *local* owned
//! worker pool) and exactly one coarse-pipeline queue primitive
//! (`crossbeam-channel`, bounded only). Both restricted crates may be depended
//! on **only** by `splot-parallel`; every other workspace crate must reach
//! parallelism through `splot-parallel`'s API. No crate may pull in an async
//! runtime or a competing thread/channel library — the whole `futures` family and
//! every `crossbeam` crate except `crossbeam-channel` are banned by prefix,
//! `rayon`/`rayon-core` are restricted to `splot-parallel`, and dependency names are
//! resolved through `package`/`workspace` aliases so a banned crate cannot hide
//! behind a rename. Codec source must not initialize or use the global Rayon pool
//! (`build_global`, or the `rayon::spawn` / `rayon::join` / `rayon::scope` free
//! functions), open an unbounded channel (any import or call form), build a
//! `std::sync::mpsc` pipeline (any import or call form), or spawn ad-hoc OS threads
//! (`thread::spawn`, `thread::Builder`, or a `std::thread` alias) outside tests.
//! Aliased imports that could hide such a call (for example `use std::thread as t;`
//! or `use crossbeam_channel as cc;`) are flagged at the rename declaration. And
//! outside `splot-parallel`, a Rayon parallel-iteration call (`par_iter`,
//! `par_chunks`, `par_bridge`) must sit inside a `WorkerPool::install` closure: a
//! call outside any `install` closure is flagged, since it would run on Rayon's
//! global pool and would not scale with the configured thread count. The
//! source scan is a line-based defense-in-depth check: it does not resolve multi-hop
//! re-exports, so the dependency-direction gate and code review remain the backstop.
//!
//! This module is a thin IO wrapper around a pure
//! [`evaluate_concurrency_policy`] evaluator so the rules can be unit-tested
//! against synthetic fixtures.

use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context as _, Result, bail};

/// Crates that only `splot-parallel` may depend on directly: the single data-parallel
/// engine (`rayon`, plus its lower-level `rayon-core` so the umbrella cannot be
/// bypassed) and the single coarse-pipeline queue primitive (`crossbeam-channel`).
/// Every other workspace crate must route parallelism through `splot-parallel`'s API.
const RESTRICTED_PARALLEL_CRATES: &[&str] = &["rayon", "rayon-core", "crossbeam-channel"];

/// Runtime/concurrency crates no workspace crate may depend on directly — async
/// runtimes, alternative thread pools, and competing channel libraries. Banning
/// them keeps the concurrency surface to the two approved primitives in
/// `splot-parallel` and keeps the codec runtime-free of async executors. The whole
/// `futures` family is banned by prefix (see [`is_banned_runtime_crate`]) rather
/// than by an exhaustive list, so `futures-channel` / `futures-task` / `futures-io`
/// / `futures-sink` / etc. cannot slip in.
const BANNED_RUNTIME_CRATES: &[&str] = &[
    "tokio",
    "async-std",
    "threadpool",
    "scoped_threadpool",
    "flume",
    "async-channel",
];

/// Returns `true` if `name` is a banned runtime/channel crate: an exact entry in
/// [`BANNED_RUNTIME_CRATES`]; any crate in the `futures` family (`futures` itself or a
/// `futures-*` crate); or any `crossbeam` crate **other than** the approved
/// `crossbeam-channel` (which is restricted to `splot-parallel` via
/// [`RESTRICTED_PARALLEL_CRATES`]) — e.g. `crossbeam-utils`, `crossbeam-queue`,
/// `crossbeam-deque`, `crossbeam-epoch`, or the `crossbeam` umbrella.
fn is_banned_runtime_crate(name: &str) -> bool {
    if BANNED_RUNTIME_CRATES.contains(&name) {
        return true;
    }
    if name == "futures" || name.starts_with("futures-") {
        return true;
    }
    if (name == "crossbeam" || name.starts_with("crossbeam-")) && name != "crossbeam-channel" {
        return true;
    }
    false
}

/// The one crate allowed to depend on [`RESTRICTED_PARALLEL_CRATES`]: the approved
/// concurrency-primitives crate that wraps Rayon and bounded crossbeam channels.
const PARALLEL_CRATE: &str = "splot-parallel";

/// The runtime-free core crate: it must not gain any concurrency dependency.
const CORE_CRATE: &str = "splot-core";

/// The validator crate: parser-driven and single-threaded; it must not depend on
/// `splot-parallel` or any restricted parallel crate.
const VALIDATE_CRATE: &str = "splot-validate";

// The banned *source* needles below are assembled with `concat!` from fragments so
// the literal token never appears verbatim in this file. The source scanner only
// walks `crates/`, never `xtask/`, so this is belt-and-suspenders: it guarantees
// that even if the scan root ever widened to include this module, the gate would
// not flag its own constant definitions as policy violations.

/// Global Rayon pool initialization is banned: splot uses a local owned pool only.
const BUILD_GLOBAL: &str = concat!("build", "_global");

/// Unbounded-channel needles — banned anywhere under `crates/`. Covers the qualified
/// path, the bare call (any import form, e.g. `use crossbeam_channel::{unbounded};`
/// then `unbounded()`), the aliased import (`unbounded as ub`), and the helper
/// identifier. Only `splot-parallel` may depend on `crossbeam-channel`, and it must
/// stay bounded-only, so none of these belongs in any crate.
const UNBOUNDED_NEEDLES: &[&str] = &[
    concat!("crossbeam_channel::", "unbounded"),
    concat!("unbounded", "("),
    concat!("unbounded", " as "),
    "unbounded_queue",
];

/// `std::sync::mpsc` pipeline needles — banned anywhere. Covers the qualified path,
/// the braced import (`use std::sync::{mpsc};`), the aliased import (`mpsc as`), and
/// the channel constructors (`mpsc::channel` / `mpsc::sync_channel`) regardless of how
/// `mpsc` is imported. Use a bounded crossbeam queue instead.
const STD_MPSC_NEEDLES: &[&str] = &[
    concat!("std::sync::", "mpsc"),
    concat!("std::sync::{", "mpsc"),
    concat!("mpsc::", "channel"),
    concat!("mpsc::", "sync_channel"),
    concat!("mpsc", " as "),
];

/// Ad-hoc OS-thread-spawn needles — banned outside tests (the local `WorkerPool`
/// owns every worker thread). Covers the bare `spawn` call (catches
/// `std::thread::spawn` and `use std::thread::spawn; spawn(...)`), the
/// `thread::Builder` form (`Builder::new().spawn(...)`), and the two `std::thread`
/// alias forms (`use std::thread as t;`, `use std::thread::{self as t};`) that would
/// otherwise hide an aliased `t::spawn`. Scoped to full paths so numeric casts such
/// as `worker_thread as u32` are never matched.
const THREAD_SPAWN_NEEDLES: &[&str] = &[
    concat!("thread::", "spawn"),
    concat!("thread::", "Builder"),
    concat!("std::thread", " as "),
    concat!("std::thread::{", "self"),
];

/// Aliasing the `crossbeam_channel` crate (`use crossbeam_channel as cc;`) — flagged
/// everywhere as an extra guard against hiding `cc::unbounded()`.
const CROSSBEAM_ALIAS: &str = concat!("crossbeam_channel", " as ");

/// Rayon global-pool entry points — the free functions that run on Rayon's implicit
/// **global** registry instead of a local pool. Banned everywhere under `crates/`
/// (including `splot-parallel`): the scoped equivalents are methods on the owned
/// `WorkerPool`/`ThreadPool` (`pool.install`, `inner.join`, `inner.scope`, …), never
/// these `rayon::` free functions.
const RAYON_GLOBAL_NEEDLES: &[&str] = &[
    concat!("rayon::", "spawn"),
    concat!("rayon::", "join"),
    concat!("rayon::", "scope"),
    concat!("rayon::", "in_place_scope"),
    concat!("rayon::", "broadcast"),
    concat!("rayon::", "yield_now"),
    concat!("rayon::", "yield_local"),
];

/// Rayon parallel-iteration entry points. Calling one of these *outside*
/// `WorkerPool::install` silently runs on Rayon's implicit **global** pool, so the
/// work would not scale with the context's configured worker count. The substring
/// `par_iter` also matches `into_par_iter` / `par_iter_mut`.
const PAR_ITER_TOKENS: &[&str] = &[
    concat!("par", "_iter"),
    concat!("par", "_chunks"),
    concat!("par", "_bridge"),
];

/// The pool-scoping call that binds parallel iteration to the local worker pool.
/// A file that uses a [`PAR_ITER_TOKENS`] entry but never calls this is presumed to
/// run on the global pool. Matched leniently (substring) to minimize false positives.
const INSTALL_CALL: &str = concat!("install", "(");

/// Path prefix of the one crate exempt from the par-iter-outside-`install` rule:
/// `splot-parallel` is the trusted wrapper that owns Rayon and may use parallel
/// iterators in helpers whose `install` scoping the file-level heuristic cannot see.
const PARALLEL_CRATE_PREFIX: &str = "crates/splot-parallel/";

/// Files exempt from the par-iter-outside-`install` rule (Rule 10) — for the rare
/// legitimate case where the `install` scoping lives in a different file. Empty by
/// default; add a path with a documented reason rather than weakening the rule.
const PAR_ITER_RULE_ALLOWLIST: &[&str] = &[];

/// One workspace crate's manifest, reduced to its direct dependency crate names
/// (resolved real names, deduplicated across dependency tables).
pub(crate) struct CrateManifestInfo {
    /// The crate's `[package].name`.
    pub name: String,
    /// Direct dependency crate names (real names; `package = "..."` resolved).
    pub direct_deps: Vec<String>,
}

/// One source line scanned from a `crates/**/*.rs` file, with enough context to
/// apply the test-aware source rules.
pub(crate) struct SourceLine {
    /// Display path of the source file (forward-slash normalized).
    pub path: String,
    /// 1-based line number within the file.
    pub line_no: usize,
    /// The line's text (trailing newline stripped).
    pub text: String,
    /// `true` if the line lies in test code (a `/tests/` file or a `#[cfg(test)]`
    /// module), which exempts it from the thread-spawn source rule.
    pub in_test: bool,
}

/// Evaluates the concurrency policy against owned manifest and source inputs,
/// returning sorted, human-readable violation strings.
///
/// This is the pure core of the check: it performs no IO, so it can be exercised
/// directly with synthetic fixtures. See the module docs for the full rule set.
pub(crate) fn evaluate_concurrency_policy(
    crates: &[CrateManifestInfo],
    sources: &[SourceLine],
) -> Vec<String> {
    let mut violations: Vec<String> = Vec::new();

    for krate in crates {
        for dep in &krate.direct_deps {
            // Rule 1: only `splot-parallel` may depend on a restricted parallel crate.
            if RESTRICTED_PARALLEL_CRATES.contains(&dep.as_str()) && krate.name != PARALLEL_CRATE {
                violations.push(format!(
                    "{}: must not depend on restricted parallel crate `{}` (only {} may); route parallelism through {}",
                    krate.name, dep, PARALLEL_CRATE, PARALLEL_CRATE
                ));
            }

            // Rule 2: no crate (including `splot-parallel`) may depend on a banned
            // runtime crate (async runtimes, alternative pools, rival channels, the
            // whole `futures` family).
            if is_banned_runtime_crate(dep) {
                violations.push(format!(
                    "{}: must not depend on banned runtime crate `{}` (no async runtime or competing thread/channel library)",
                    krate.name, dep
                ));
            }

            // Rule 3: `splot-core` must remain runtime-free.
            if krate.name == CORE_CRATE
                && (dep == PARALLEL_CRATE
                    || RESTRICTED_PARALLEL_CRATES.contains(&dep.as_str())
                    || is_banned_runtime_crate(dep))
            {
                violations.push(format!(
                    "{}: must remain runtime-free but depends on `{}`",
                    krate.name, dep
                ));
            }

            // Rule 4: `splot-validate` stays single-threaded — no `splot-parallel`
            // or restricted parallel crate.
            if krate.name == VALIDATE_CRATE
                && (dep == PARALLEL_CRATE || RESTRICTED_PARALLEL_CRATES.contains(&dep.as_str()))
            {
                violations.push(format!(
                    "{}: must stay single-threaded but depends on `{}`",
                    krate.name, dep
                ));
            }
        }
    }

    for line in sources {
        let where_at = format!("{}:{}", line.path, line.line_no);

        // Rule 5: banned global Rayon pool initialization.
        if line.text.contains(BUILD_GLOBAL) {
            violations.push(format!(
                "{where_at}: global Rayon pool init (`{BUILD_GLOBAL}`) is banned; use a local owned worker pool"
            ));
        }

        // Rule 6: banned unbounded channels — any import/call/alias/helper form.
        if let Some(needle) = UNBOUNDED_NEEDLES.iter().find(|n| line.text.contains(**n)) {
            violations.push(format!(
                "{where_at}: unbounded channels (`{needle}`) are banned; use a bounded crossbeam queue"
            ));
        }

        // Rule 7: banned `std::sync::mpsc` pipelines (any import/alias/call form).
        if let Some(needle) = STD_MPSC_NEEDLES.iter().find(|n| line.text.contains(**n)) {
            violations.push(format!(
                "{where_at}: `std::sync::mpsc` pipelines (`{needle}`) are banned; use a bounded crossbeam queue"
            ));
        }

        // Rule 8: ad-hoc OS-thread spawning (call, `thread::Builder`, or `std::thread`
        // alias forms) is banned outside tests; the local `WorkerPool` owns every
        // worker thread. Test lines are exempt.
        if !line.in_test
            && let Some(needle) = THREAD_SPAWN_NEEDLES
                .iter()
                .find(|n| line.text.contains(**n))
        {
            violations.push(format!(
                "{where_at}: ad-hoc OS-thread spawning (`{needle}`) is banned outside tests; use the local `WorkerPool`"
            ));
        }

        // Rule 9: aliasing the `crossbeam_channel` crate (everywhere) — an extra guard
        // against hiding `cc::unbounded()` behind a rename declaration.
        if line.text.contains(CROSSBEAM_ALIAS) {
            violations.push(format!(
                "{where_at}: aliasing `crossbeam_channel` (`{CROSSBEAM_ALIAS}…`) is banned; it can hide an unbounded channel from the policy scanner"
            ));
        }

        // Rule 11: Rayon global-pool entry points (free functions) — banned everywhere,
        // including `splot-parallel`. They run on Rayon's implicit global registry; use
        // the owned `WorkerPool`/`ThreadPool` scoped methods (e.g. `install`) instead.
        if let Some(needle) = RAYON_GLOBAL_NEEDLES
            .iter()
            .find(|n| line.text.contains(**n))
        {
            violations.push(format!(
                "{where_at}: Rayon global-pool entry point (`{needle}`) is banned; run work through the local `WorkerPool`, not Rayon's global pool"
            ));
        }
    }

    // Rule 10: outside `splot-parallel`, a Rayon parallel-iteration call must run
    // *inside* a `WorkerPool::install` closure, or it runs on Rayon's global pool and
    // will not scale with `--threads`. `sources` is sorted by (path, line), so we scan
    // each file in order and track whether the current line is lexically inside an
    // `install(...)` closure with a brace-depth tracker (same heuristic as the
    // cfg(test) detector): an `install(` that opens a net brace pushes a scope until
    // the matching `}`; a multi-line `install(` whose `{` lands on a later line arms a
    // pending scope. `splot-parallel`, test lines, and allowlisted files are exempt; a
    // par-iter on the same line as the `install(` counts as inside.
    let mut current_path: &str = "";
    let mut depth: i32 = 0;
    let mut install_stack: Vec<i32> = Vec::new();
    let mut pending_install = false;
    for line in sources {
        if line.path.as_str() != current_path {
            current_path = line.path.as_str();
            depth = 0;
            install_stack.clear();
            pending_install = false;
        }
        let opens_install = line.text.contains(INSTALL_CALL);
        let inside_install = !install_stack.is_empty() || opens_install || pending_install;
        if !line.in_test
            && !inside_install
            && !current_path.starts_with(PARALLEL_CRATE_PREFIX)
            && !PAR_ITER_RULE_ALLOWLIST.contains(&current_path)
            && let Some(token) = PAR_ITER_TOKENS.iter().find(|t| line.text.contains(**t))
        {
            violations.push(format!(
                "{}:{}: parallel iteration (`{token}`) must run inside `WorkerPool::install` so it uses the configured worker pool, not Rayon's global pool",
                line.path, line.line_no
            ));
        }
        let pre_depth = depth;
        depth += brace_delta(&line.text);
        if opens_install {
            if depth > pre_depth {
                install_stack.push(pre_depth);
                pending_install = false;
            } else {
                // A multi-line `install(` whose closure `{` lands on a later line.
                pending_install = true;
            }
        } else if pending_install {
            if depth > pre_depth {
                install_stack.push(pre_depth);
            }
            pending_install = false;
        }
        while let Some(&entry) = install_stack.last() {
            if depth <= entry {
                install_stack.pop();
            } else {
                break;
            }
        }
    }

    violations.sort();
    violations.dedup();
    violations
}

/// Verifies the workspace honors the Rayon + crossbeam-channel concurrency policy.
///
/// Reads every workspace member's manifest for direct dependencies and walks every
/// `crates/**/*.rs` source file, then applies [`evaluate_concurrency_policy`]. Fails
/// the gate (non-zero exit) on any violation.
///
/// # Errors
/// Returns an error if a manifest or source file cannot be read/parsed, or if the
/// evaluator reports one or more policy violations.
pub(crate) fn check_concurrency_policy(root: &Path) -> Result<()> {
    let crates = collect_crate_manifest_info(root)?;
    let sources = collect_source_lines(root)?;
    let violations = evaluate_concurrency_policy(&crates, &sources);

    if violations.is_empty() {
        eprintln!("check-concurrency-policy: ok");
        Ok(())
    } else {
        for violation in &violations {
            eprintln!("{violation}");
        }
        bail!(
            "check-concurrency-policy: {} violation(s)",
            violations.len()
        )
    }
}

/// Builds the per-crate direct-dependency view from every workspace member's manifest.
///
/// Dependency names are resolved to real package names, including `package = "..."`
/// renames AND `x.workspace = true` entries whose rename lives in the root
/// `[workspace.dependencies]` map — so a banned crate cannot hide behind a workspace
/// alias (e.g. root `rt = { package = "tokio" }` + member `rt.workspace = true`).
fn collect_crate_manifest_info(root: &Path) -> Result<Vec<CrateManifestInfo>> {
    let root_manifest = crate::read_manifest(&root.join("Cargo.toml"))?;
    let workspace_deps = crate::workspace_dep_names(&root_manifest);
    let mut crates = Vec::new();
    for member in crate::workspace_members(root)? {
        let manifest_path = root.join(&member).join("Cargo.toml");
        let manifest = crate::read_manifest(&manifest_path)?;
        let name = manifest
            .get("package")
            .and_then(|package| package.get("name"))
            .and_then(toml::Value::as_str)
            .map(str::to_owned)
            .with_context(|| format!("{} has no [package].name", manifest_path.display()))?;
        let direct_deps = direct_dependency_names(&manifest, &workspace_deps);
        crates.push(CrateManifestInfo { name, direct_deps });
    }
    Ok(crates)
}

/// Collects the real crate names of every direct dependency across the
/// `[dependencies]`, `[dev-dependencies]`, `[build-dependencies]`, and any
/// `[target.*.dependencies]` tables, deduplicated and sorted. `workspace_deps` maps a
/// workspace-dependency alias to its real package name (from the root manifest).
fn direct_dependency_names(
    manifest: &toml::Table,
    workspace_deps: &HashMap<String, String>,
) -> Vec<String> {
    let mut names = Vec::new();
    collect_dependency_names(manifest, workspace_deps, &mut names);
    // Platform-specific `[target.'cfg(...)'.dependencies]` tables count as direct deps.
    if let Some(targets) = manifest.get("target").and_then(toml::Value::as_table) {
        for target in targets.values() {
            if let Some(table) = target.as_table() {
                collect_dependency_names(table, workspace_deps, &mut names);
            }
        }
    }
    names.sort();
    names.dedup();
    names
}

/// Appends the resolved real crate name of each entry in the dependency tables found
/// directly under `parent`, resolving `package = "..."` renames and `workspace = true`
/// aliases through `workspace_deps` (shared with `check-dependency-direction`).
fn collect_dependency_names(
    parent: &toml::Table,
    workspace_deps: &HashMap<String, String>,
    names: &mut Vec<String>,
) {
    for table_name in ["dependencies", "dev-dependencies", "build-dependencies"] {
        if let Some(table) = parent.get(table_name).and_then(toml::Value::as_table) {
            for (key, value) in table {
                names.push(crate::resolved_dep_name(key, value, workspace_deps));
            }
        }
    }
}

/// Walks every `.rs` file under `crates/` (never `xtask/`, `fuzz/`, `target/`, or
/// `.git/`) and returns each line tagged with whether it is test code.
fn collect_source_lines(root: &Path) -> Result<Vec<SourceLine>> {
    let crates_dir = root.join("crates");
    let mut sources = Vec::new();
    if !crates_dir.is_dir() {
        return Ok(sources);
    }
    let mut stack = vec![crates_dir];
    while let Some(dir) = stack.pop() {
        let entries = std::fs::read_dir(&dir)
            .with_context(|| format!("failed to read directory {}", dir.display()))?;
        for entry in entries {
            let entry =
                entry.with_context(|| format!("failed to read an entry in {}", dir.display()))?;
            let path = entry.path();
            let file_type = entry
                .file_type()
                .with_context(|| format!("failed to stat {}", path.display()))?;
            if file_type.is_dir() {
                if !is_skipped_source_dir(&path) {
                    stack.push(path);
                }
            } else if path.extension().and_then(|ext| ext.to_str()) == Some("rs") {
                let display = path
                    .strip_prefix(root)
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .replace('\\', "/");
                let file_in_tests = display.contains("/tests/");
                let contents = std::fs::read_to_string(&path)
                    .with_context(|| format!("failed to read {}", path.display()))?;
                scan_source_text(&display, &contents, file_in_tests, &mut sources);
            }
        }
    }
    sources.sort_by(|a, b| (a.path.as_str(), a.line_no).cmp(&(b.path.as_str(), b.line_no)));
    Ok(sources)
}

/// Directories never scanned for source: build output and VCS metadata. `crates/` is
/// the only scan root, so `xtask/` and `fuzz/` are already excluded by construction.
fn is_skipped_source_dir(path: &Path) -> bool {
    matches!(
        path.file_name().and_then(|name| name.to_str()),
        Some("target" | ".git")
    )
}

/// Splits `contents` into [`SourceLine`]s, marking lines as test code and skipping
/// comment-only lines from the banned-needle scan.
///
/// `in_test` is set when the whole file is a test file (`file_in_tests`) or the line
/// lies inside a `#[cfg(test)]`-annotated module. The cfg(test) region is found with a
/// deliberately simple brace-depth tracker (see [`scan_source_lines`]). Comment-only
/// lines (trimmed text starting with `//`) are dropped entirely: the policy bans the
/// concurrency *construct in code*, not prose discussing it (e.g. doc comments in
/// `splot-parallel` that state `build_global` is never used).
fn scan_source_text(
    display: &str,
    contents: &str,
    file_in_tests: bool,
    sources: &mut Vec<SourceLine>,
) {
    for line in scan_source_lines(contents, file_in_tests) {
        let ScannedLine {
            line_no,
            text,
            in_test,
        } = line;
        // Skip comment-only lines so policy prose (doc comments, `//` notes that
        // *name* a banned construct to forbid it) is never flagged as a use.
        if text.trim_start().starts_with("//") {
            continue;
        }
        sources.push(SourceLine {
            path: display.to_owned(),
            line_no,
            text,
            in_test,
        });
    }
}

/// One scanned line: 1-based number, owned text, and computed `in_test` flag.
struct ScannedLine {
    line_no: usize,
    text: String,
    in_test: bool,
}

/// Tags each line of `contents` with `in_test`, detecting `#[cfg(test)]` modules.
///
/// Heuristic: when a `#[cfg(test)]` attribute is seen and the next meaningful line
/// opens a `mod ... {`, every line through the matching closing brace is treated as
/// test code (brace-depth tracked across lines). A line is also test code when the
/// whole file is a test file (`file_in_tests`). This is intentionally simple and
/// slightly conservative; it only needs to keep the thread-spawn source rule from
/// flagging legitimate test code.
fn scan_source_lines(contents: &str, file_in_tests: bool) -> Vec<ScannedLine> {
    let mut out = Vec::new();
    // cfg(test) region tracker state:
    let mut pending_cfg_test = false; // saw `#[cfg(test)]`, awaiting its `mod ... {`
    let mut in_cfg_test = false; // currently inside a cfg(test) module body
    let mut depth: i32 = 0; // running brace depth since the region opener

    for (index, raw) in contents.lines().enumerate() {
        let line = raw.to_owned();
        let trimmed = line.trim();

        if in_cfg_test {
            // Already inside a test module: this line is test code. Maintain the brace
            // depth and close the region when it returns to zero (the matching `}`).
            depth += brace_delta(&line);
            out.push(ScannedLine {
                line_no: index + 1,
                text: line,
                in_test: true,
            });
            if depth <= 0 {
                in_cfg_test = false;
                depth = 0;
            }
            continue;
        }

        // A `#[cfg(test)]` attribute arms the next `mod ... {` to open a test region.
        if trimmed.starts_with("#[cfg(test)]") {
            pending_cfg_test = true;
        } else if pending_cfg_test {
            if trimmed.starts_with("mod ") && trimmed.contains('{') {
                // The armed `mod ... {` line is the region opener: it and the lines
                // through its matching `}` are test code. Seed the depth with its delta.
                pending_cfg_test = false;
                depth = brace_delta(&line);
                let one_line_mod = depth <= 0; // a self-closing `mod tests { ... }`
                in_cfg_test = !one_line_mod;
                if one_line_mod {
                    depth = 0;
                }
                out.push(ScannedLine {
                    line_no: index + 1,
                    text: line,
                    in_test: true,
                });
                continue;
            }
            // An armed attribute followed by a non-`mod` meaningful line disarms it
            // (e.g. `#[cfg(test)] fn helper()` — not a module). Blank lines and further
            // attributes keep it armed.
            if !trimmed.is_empty() && !trimmed.starts_with("#[") {
                pending_cfg_test = false;
            }
        }

        out.push(ScannedLine {
            line_no: index + 1,
            text: line,
            in_test: file_in_tests,
        });
    }
    out
}

/// Net brace delta of a line: `{` count minus `}` count. A coarse, comment/string
/// unaware measure — adequate for the conservative cfg(test) region heuristic.
fn brace_delta(line: &str) -> i32 {
    let opens = line.matches('{').count() as i32;
    let closes = line.matches('}').count() as i32;
    opens - closes
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    /// Builds a `CrateManifestInfo` from a name and a list of direct dep names.
    fn krate(name: &str, deps: &[&str]) -> CrateManifestInfo {
        CrateManifestInfo {
            name: name.to_owned(),
            direct_deps: deps.iter().map(|d| (*d).to_owned()).collect(),
        }
    }

    /// Builds a `SourceLine` with the given text and test flag.
    fn line(text: &str, in_test: bool) -> SourceLine {
        SourceLine {
            path: "crates/x/src/lib.rs".to_owned(),
            line_no: 1,
            text: text.to_owned(),
            in_test,
        }
    }

    /// Builds a `SourceLine` with an explicit path/line for multi-file Rule-10 tests.
    fn line_at(path: &str, line_no: usize, text: &str, in_test: bool) -> SourceLine {
        SourceLine {
            path: path.to_owned(),
            line_no,
            text: text.to_owned(),
            in_test,
        }
    }

    #[test]
    fn parallel_crate_may_use_restricted_parallel_deps() {
        let crates = [krate(
            PARALLEL_CRATE,
            &["rayon", "crossbeam-channel", "thiserror"],
        )];
        assert!(evaluate_concurrency_policy(&crates, &[]).is_empty());
    }

    #[test]
    fn validate_with_rayon_is_a_violation() {
        let crates = [krate(VALIDATE_CRATE, &["splot-core", "rayon"])];
        let violations = evaluate_concurrency_policy(&crates, &[]);
        assert!(!violations.is_empty());
    }

    #[test]
    fn tokio_dependency_is_a_violation() {
        let crates = [krate("splot-decode", &["tokio"])];
        let violations = evaluate_concurrency_policy(&crates, &[]);
        assert!(
            violations.iter().any(|v| v.contains("tokio")),
            "expected a tokio violation, got {violations:?}"
        );
    }

    #[test]
    fn threadpool_dependency_is_a_violation() {
        let crates = [krate("splot-encode", &["threadpool"])];
        let violations = evaluate_concurrency_policy(&crates, &[]);
        assert!(
            violations.iter().any(|v| v.contains("threadpool")),
            "expected a threadpool violation, got {violations:?}"
        );
    }

    #[test]
    fn non_parallel_crate_with_rayon_is_a_violation_rule_one() {
        let crates = [krate("splot-decode", &["rayon"])];
        let violations = evaluate_concurrency_policy(&crates, &[]);
        assert!(
            violations.iter().any(|v| v.contains("rayon")),
            "expected a rule-1 rayon violation, got {violations:?}"
        );
    }

    #[test]
    fn core_depending_on_parallel_is_a_violation_rule_three() {
        let crates = [krate(CORE_CRATE, &[PARALLEL_CRATE])];
        let violations = evaluate_concurrency_policy(&crates, &[]);
        assert!(
            violations.iter().any(|v| v.contains("runtime-free")),
            "expected a rule-3 runtime-free violation, got {violations:?}"
        );
    }

    #[test]
    fn clean_manifest_set_has_no_violations() {
        let crates = [
            krate(CORE_CRATE, &["thiserror", "serde"]),
            krate(PARALLEL_CRATE, &["rayon", "crossbeam-channel", "thiserror"]),
            krate(
                "splot-encode",
                &["splot-core", "splot-parallel", "thiserror"],
            ),
        ];
        assert!(evaluate_concurrency_policy(&crates, &[]).is_empty());
    }

    #[test]
    fn build_global_source_line_is_a_violation() {
        // Build the needle the same `concat!` way so the literal never appears here.
        let token = concat!("build", "_global");
        let src = [line(&format!("    pool.{token}();"), false)];
        let violations = evaluate_concurrency_policy(&[], &src);
        assert!(
            violations.iter().any(|v| v.contains(BUILD_GLOBAL)),
            "expected a build-global violation, got {violations:?}"
        );
    }

    #[test]
    fn unbounded_source_line_is_a_violation() {
        let token = concat!("crossbeam_channel::", "unbounded");
        let src = [line(&format!("    let (tx, rx) = {token}();"), false)];
        let violations = evaluate_concurrency_policy(&[], &src);
        assert!(
            violations.iter().any(|v| v.contains("unbounded")),
            "expected an unbounded violation, got {violations:?}"
        );
    }

    #[test]
    fn std_mpsc_source_line_is_a_violation() {
        let token = concat!("std::sync::", "mpsc");
        let src = [line(&format!("    use {token}::channel;"), false)];
        let violations = evaluate_concurrency_policy(&[], &src);
        assert!(
            violations.iter().any(|v| v.contains("mpsc")),
            "expected a std-mpsc violation, got {violations:?}"
        );
    }

    #[test]
    fn braced_and_aliased_mpsc_imports_are_violations() {
        // `use std::sync::{mpsc};` / `{mpsc as chan};` then `mpsc::channel()` /
        // `chan::sync_channel()` must be caught despite the missing qualified path.
        for code in [
            "    use std::sync::{mpsc};",
            "    use std::sync::{mpsc as chan};",
            "    let (tx, rx) = mpsc::channel();",
            "    let (tx, rx) = mpsc::sync_channel(4);",
        ] {
            let violations = evaluate_concurrency_policy(&[], &[line(code, false)]);
            assert!(
                violations.iter().any(|v| v.contains("mpsc")),
                "expected an mpsc violation for `{code}`, got {violations:?}"
            );
        }
    }

    #[test]
    fn rayon_core_is_restricted_to_splot_parallel() {
        // Only splot-parallel may depend on rayon-core (like rayon itself).
        let offender = [krate("splot-decode", &["rayon-core"])];
        assert!(
            evaluate_concurrency_policy(&offender, &[])
                .iter()
                .any(|v| v.contains("rayon-core")),
            "a non-parallel crate depending on rayon-core must be flagged"
        );
        assert!(
            evaluate_concurrency_policy(&[krate(PARALLEL_CRATE, &["rayon-core"])], &[]).is_empty(),
            "splot-parallel may depend on rayon-core"
        );
    }

    #[test]
    fn adjacent_crossbeam_crates_are_banned_but_channel_is_allowed() {
        assert!(is_banned_runtime_crate("crossbeam-utils"));
        assert!(is_banned_runtime_crate("crossbeam-queue"));
        assert!(is_banned_runtime_crate("crossbeam-deque"));
        assert!(is_banned_runtime_crate("crossbeam"));
        assert!(!is_banned_runtime_crate("crossbeam-channel"));

        let offender = [krate("splot-decode", &["crossbeam-utils"])];
        assert!(
            evaluate_concurrency_policy(&offender, &[])
                .iter()
                .any(|v| v.contains("crossbeam-utils")),
            "a crate depending on crossbeam-utils must be flagged"
        );
        // splot-parallel may still depend on the approved crossbeam-channel.
        assert!(
            evaluate_concurrency_policy(&[krate(PARALLEL_CRATE, &["crossbeam-channel"])], &[])
                .is_empty(),
            "splot-parallel's crossbeam-channel dependency must remain allowed"
        );
    }

    #[test]
    fn par_iter_after_install_closure_closes_is_a_violation() {
        // install closure on lines 10-12, then a top-level par_iter at line 20 — the
        // later iterator is outside install and must be flagged (file-level `has_install`
        // would have missed it).
        let src = [
            line_at(
                "crates/splot-encode/src/x.rs",
                10,
                "    pool.install(|| {",
                false,
            ),
            line_at("crates/splot-encode/src/x.rs", 11, "        work();", false),
            line_at("crates/splot-encode/src/x.rs", 12, "    });", false),
            line_at(
                "crates/splot-encode/src/x.rs",
                20,
                "    let v: Vec<_> = items.par_iter().collect();",
                false,
            ),
        ];
        let violations = evaluate_concurrency_policy(&[], &src);
        assert!(
            violations.iter().any(|v| v.contains("WorkerPool::install")),
            "a par-iter after the install closure closes must be flagged, got {violations:?}"
        );
    }

    #[test]
    fn par_iter_inside_install_closure_is_ok() {
        let src = [
            line_at(
                "crates/splot-encode/src/x.rs",
                10,
                "    pool.install(|| {",
                false,
            ),
            line_at(
                "crates/splot-encode/src/x.rs",
                11,
                "        let v: Vec<_> = items.par_iter().collect();",
                false,
            ),
            line_at("crates/splot-encode/src/x.rs", 12, "    });", false),
        ];
        assert!(
            evaluate_concurrency_policy(&[], &src).is_empty(),
            "a par-iter inside the install closure must be accepted"
        );
    }

    #[test]
    fn thread_spawn_is_a_violation_outside_tests_but_exempt_in_tests() {
        let token = concat!("thread::", "spawn");
        let code = format!("    {token}(|| do_work());");

        let outside = [line(&code, false)];
        let violations = evaluate_concurrency_policy(&[], &outside);
        assert!(
            violations.iter().any(|v| v.contains(token)),
            "expected a thread-spawn violation outside tests, got {violations:?}"
        );

        let inside = [line(&code, true)];
        assert!(
            evaluate_concurrency_policy(&[], &inside).is_empty(),
            "thread-spawn inside tests must be exempt"
        );
    }

    #[test]
    fn cfg_test_region_marks_lines_as_test() {
        let token = concat!("thread::", "spawn");
        let contents = format!(
            "fn prod() {{}}\n#[cfg(test)]\nmod tests {{\n    fn helper() {{ {token}(|| ()); }}\n}}\n"
        );
        let scanned = scan_source_lines(&contents, false);
        // Line 4 holds the thread-spawn call inside the cfg(test) module.
        let spawn_line = scanned
            .iter()
            .find(|l| l.text.contains(token))
            .expect("scanned lines include the spawn call");
        assert!(
            spawn_line.in_test,
            "thread-spawn line inside a #[cfg(test)] module should be in_test"
        );
    }

    #[test]
    fn comment_lines_naming_banned_tokens_are_not_flagged() {
        // A doc comment that *names* build_global to forbid it must not be a use.
        let token = concat!("build", "_global");
        let mut sources = Vec::new();
        scan_source_text(
            "crates/x/src/lib.rs",
            &format!("//! The global pool and `{token}` are never used.\n"),
            false,
            &mut sources,
        );
        let violations = evaluate_concurrency_policy(&[], &sources);
        assert!(
            violations.is_empty(),
            "comment prose naming a banned token must not be flagged, got {violations:?}"
        );
    }

    #[test]
    fn unbounded_queue_identifier_is_a_violation() {
        // The `unbounded_queue` helper-identifier needle (Rule 6, second form).
        let src = [line("    let queue = unbounded_queue();", false)];
        let violations = evaluate_concurrency_policy(&[], &src);
        assert!(
            violations.iter().any(|v| v.contains("unbounded")),
            "expected an unbounded-queue identifier violation, got {violations:?}"
        );
    }

    #[test]
    fn aliased_thread_import_is_a_violation_outside_tests_but_exempt_in_tests() {
        // `use std::thread as t;` then `t::spawn()` would evade THREAD_SPAWN; the
        // rename declaration is caught instead, and is test-exempt like Rule 8.
        let code = "    use std::thread as t;";
        let outside = [line(code, false)];
        assert!(
            !evaluate_concurrency_policy(&[], &outside).is_empty(),
            "aliasing std::thread outside tests must be flagged"
        );
        let inside = [line(code, true)];
        assert!(
            evaluate_concurrency_policy(&[], &inside).is_empty(),
            "aliasing std::thread inside tests is exempt (matches the thread-spawn rule)"
        );
    }

    #[test]
    fn aliased_crossbeam_import_is_a_violation() {
        // `use crossbeam_channel as cc;` then `cc::unbounded()` would evade UNBOUNDED.
        let src = [line("    use crossbeam_channel as cc;", false)];
        let violations = evaluate_concurrency_policy(&[], &src);
        assert!(
            !violations.is_empty(),
            "aliasing crossbeam_channel must be flagged, got {violations:?}"
        );
    }

    #[test]
    fn par_iter_outside_install_is_a_violation() {
        let src = [line_at(
            "crates/splot-encode/src/x.rs",
            10,
            "    let v: Vec<_> = (0..n).into_par_iter().map(f).collect();",
            false,
        )];
        let violations = evaluate_concurrency_policy(&[], &src);
        assert!(
            violations.iter().any(|v| v.contains("WorkerPool::install")),
            "expected a par-iter-outside-install violation, got {violations:?}"
        );
    }

    #[test]
    fn par_iter_inside_install_is_ok() {
        let src = [
            line_at(
                "crates/splot-encode/src/x.rs",
                10,
                "        pool.install(|| {",
                false,
            ),
            line_at(
                "crates/splot-encode/src/x.rs",
                11,
                "            (0..n).into_par_iter().for_each(g);",
                false,
            ),
        ];
        assert!(
            evaluate_concurrency_policy(&[], &src).is_empty(),
            "par-iter in a file that calls install must be accepted"
        );
    }

    #[test]
    fn par_iter_in_splot_parallel_is_exempt() {
        let src = [line_at(
            "crates/splot-parallel/src/x.rs",
            10,
            "    let v: Vec<_> = (0..n).into_par_iter().collect();",
            false,
        )];
        assert!(
            evaluate_concurrency_policy(&[], &src).is_empty(),
            "splot-parallel is the trusted Rayon wrapper and is exempt from Rule 10"
        );
    }

    #[test]
    fn par_iter_on_test_line_is_exempt_from_rule_ten() {
        let src = [line_at(
            "crates/splot-encode/src/x.rs",
            10,
            "    let v: Vec<_> = (0..n).into_par_iter().collect();",
            true,
        )];
        assert!(
            evaluate_concurrency_policy(&[], &src).is_empty(),
            "par-iter on a test line must not trigger Rule 10"
        );
    }

    #[test]
    fn unbounded_bare_call_is_a_violation() {
        // `use crossbeam_channel::{unbounded}; let (s, r) = unbounded();` — the call
        // form must be caught regardless of how `unbounded` was imported.
        let src = [line("    let (s, r) = unbounded();", false)];
        let violations = evaluate_concurrency_policy(&[], &src);
        assert!(
            violations.iter().any(|v| v.contains("unbounded")),
            "expected an unbounded bare-call violation, got {violations:?}"
        );
    }

    #[test]
    fn unbounded_aliased_import_is_a_violation() {
        let src = [line(
            "    use crossbeam_channel::{bounded, unbounded as ub};",
            false,
        )];
        let violations = evaluate_concurrency_policy(&[], &src);
        assert!(
            violations.iter().any(|v| v.contains("unbounded")),
            "expected an aliased-unbounded-import violation, got {violations:?}"
        );
    }

    #[test]
    fn thread_builder_spawn_is_a_violation_outside_tests_but_exempt_in_tests() {
        let code = "    std::thread::Builder::new().spawn(work)?;";
        assert!(
            !evaluate_concurrency_policy(&[], &[line(code, false)]).is_empty(),
            "thread::Builder spawn outside tests must be flagged"
        );
        assert!(
            evaluate_concurrency_policy(&[], &[line(code, true)]).is_empty(),
            "thread::Builder spawn inside tests is exempt"
        );
    }

    #[test]
    fn thread_self_alias_import_is_a_violation_outside_tests() {
        let src = [line("    use std::thread::{self as t};", false)];
        assert!(
            !evaluate_concurrency_policy(&[], &src).is_empty(),
            "aliasing std::thread via {{self as t}} must be flagged"
        );
    }

    #[test]
    fn rayon_global_entry_points_are_violations() {
        for call in [
            "    rayon::join(a, b);",
            "    rayon::spawn(|| work());",
            "    rayon::scope(|s| s.spawn(|_| work()));",
        ] {
            let violations = evaluate_concurrency_policy(&[], &[line(call, false)]);
            assert!(
                violations.iter().any(|v| v.contains("global-pool")),
                "expected a Rayon global-pool violation for `{call}`, got {violations:?}"
            );
        }
    }

    #[test]
    fn futures_family_crates_are_banned_by_prefix() {
        assert!(is_banned_runtime_crate("futures"));
        assert!(is_banned_runtime_crate("futures-channel"));
        assert!(is_banned_runtime_crate("futures-sink"));
        assert!(is_banned_runtime_crate("futures-task"));
        assert!(!is_banned_runtime_crate("futuristic"));
        assert!(!is_banned_runtime_crate("future-proof"));

        let crates = [krate("splot-decode", &["futures-channel"])];
        let violations = evaluate_concurrency_policy(&crates, &[]);
        assert!(
            violations.iter().any(|v| v.contains("futures-channel")),
            "expected a futures-channel ban, got {violations:?}"
        );
    }

    #[test]
    fn workspace_aliased_banned_dependency_is_resolved() {
        // A member inheriting a workspace dependency aliased to a banned crate
        // (`rt.workspace = true`, root maps `rt = { package = "tokio" }`) must resolve
        // to the real package name, not the alias.
        let manifest: toml::Table = toml::from_str(
            "[package]\nname = \"splot-decode\"\n[dependencies]\nrt.workspace = true\n",
        )
        .unwrap();
        let mut workspace_deps: std::collections::HashMap<String, String> =
            std::collections::HashMap::new();
        workspace_deps.insert("rt".to_owned(), "tokio".to_owned());

        let deps = direct_dependency_names(&manifest, &workspace_deps);
        assert!(
            deps.contains(&"tokio".to_owned()),
            "workspace alias must resolve to the real package name, got {deps:?}"
        );

        let crates = [CrateManifestInfo {
            name: "splot-decode".to_owned(),
            direct_deps: deps,
        }];
        let violations = evaluate_concurrency_policy(&crates, &[]);
        assert!(
            violations.iter().any(|v| v.contains("tokio")),
            "the resolved banned crate must be flagged, got {violations:?}"
        );
    }

    #[test]
    fn real_repo_passes_concurrency_policy() {
        // Guards against self-flagging and proves the clean repo passes the gate.
        let root = crate::workspace_root().unwrap();
        check_concurrency_policy(&root)
            .expect("the current repo must satisfy the concurrency policy");
    }
}
