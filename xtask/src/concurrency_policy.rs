// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Concurrency-policy check.
//!
//! `splot` uses exactly one data-parallel engine (Rayon, via a *local* owned
//! worker pool) and exactly one coarse-pipeline queue primitive
//! (`crossbeam-channel`, bounded only). Both restricted crates may be depended
//! on **only** by `splot-parallel`; every other workspace crate must reach
//! parallelism through `splot-parallel`'s API. No crate may pull in an async
//! runtime or a competing thread/channel library, and codec source must not
//! initialize a global Rayon pool, open an unbounded channel, build a
//! `std::sync::mpsc` pipeline, or spawn ad-hoc OS threads outside tests. Aliased
//! imports that could hide such a call (for example `use std::thread as t;` or
//! `use crossbeam_channel as cc;`) are flagged at the rename declaration. The source
//! scan is a line-based defense-in-depth check: it does not resolve multi-hop
//! re-exports, so the dependency-direction gate and code review remain the backstop.
//!
//! This module is a thin IO wrapper around a pure
//! [`evaluate_concurrency_policy`] evaluator so the rules can be unit-tested
//! against synthetic fixtures.

use std::path::Path;

use anyhow::{Context as _, Result, bail};

/// Crates that only `splot-parallel` may depend on directly: the single data-parallel
/// engine (`rayon`) and the single coarse-pipeline queue primitive (`crossbeam-channel`).
/// Every other workspace crate must route parallelism through `splot-parallel`'s API.
const RESTRICTED_PARALLEL_CRATES: &[&str] = &["rayon", "crossbeam-channel"];

/// Runtime/concurrency crates no workspace crate may depend on directly — async
/// runtimes, alternative thread pools, and competing channel libraries. Banning
/// them keeps the concurrency surface to the two approved primitives in
/// `splot-parallel` and keeps the codec runtime-free of async executors.
const BANNED_RUNTIME_CRATES: &[&str] = &[
    "tokio",
    "async-std",
    "futures",
    "futures-core",
    "futures-util",
    "futures-executor",
    "threadpool",
    "scoped_threadpool",
    "flume",
    "async-channel",
];

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

/// Unbounded channels are banned: only bounded crossbeam queues are permitted.
const UNBOUNDED: &str = concat!("crossbeam_channel::", "unbounded");

/// Identifier form of an unbounded queue helper (also banned).
const UNBOUNDED_QUEUE: &str = "unbounded_queue";

/// `std::sync::mpsc` pipelines are banned: use a bounded crossbeam queue instead.
const STD_MPSC: &str = concat!("std::sync::", "mpsc");

/// Ad-hoc OS-thread spawning is banned outside tests: use the local worker pool.
const THREAD_SPAWN: &str = concat!("thread::", "spawn");

/// Aliased import of `std::thread` (`use std::thread as t;`) would let `t::spawn()`
/// slip past [`THREAD_SPAWN`]. The rename declaration is flagged instead; like the
/// thread-spawn rule it is test-exempt. Scoped to the full path so numeric casts
/// such as `worker_thread as u32` are never matched.
const THREAD_ALIAS: &str = concat!("std::thread", " as ");

/// Aliased import of `crossbeam_channel` (`use crossbeam_channel as cc;`) would let
/// `cc::unbounded()` slip past [`UNBOUNDED`]. The rename declaration is flagged
/// (everywhere, matching the unbounded rule). Only `splot-parallel` may import
/// `crossbeam_channel` at all, and it never aliases it.
const CROSSBEAM_ALIAS: &str = concat!("crossbeam_channel", " as ");

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
            // runtime crate (async runtimes, alternative pools, rival channels).
            if BANNED_RUNTIME_CRATES.contains(&dep.as_str()) {
                violations.push(format!(
                    "{}: must not depend on banned runtime crate `{}` (no async runtime or competing thread/channel library)",
                    krate.name, dep
                ));
            }

            // Rule 3: `splot-core` must remain runtime-free.
            if krate.name == CORE_CRATE
                && (dep == PARALLEL_CRATE
                    || RESTRICTED_PARALLEL_CRATES.contains(&dep.as_str())
                    || BANNED_RUNTIME_CRATES.contains(&dep.as_str()))
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

        // Rule 6: banned unbounded channels (call form or helper identifier).
        if line.text.contains(UNBOUNDED) || line.text.contains(UNBOUNDED_QUEUE) {
            violations.push(format!(
                "{where_at}: unbounded channels are banned; use a bounded crossbeam queue"
            ));
        }

        // Rule 7: banned `std::sync::mpsc` pipelines.
        if line.text.contains(STD_MPSC) {
            violations.push(format!(
                "{where_at}: `{STD_MPSC}` pipelines are banned; use a bounded crossbeam queue"
            ));
        }

        // Rule 8: ad-hoc thread spawning is banned outside tests; test lines are exempt.
        if !line.in_test && line.text.contains(THREAD_SPAWN) {
            violations.push(format!(
                "{where_at}: ad-hoc thread spawning (`{THREAD_SPAWN}`) is banned outside tests; use the local worker pool"
            ));
        }

        // Rule 9: aliased imports of a sensitive module defeat the qualified needles
        // above; flag the rename declaration itself (the alias cannot be used without
        // first being declared). The `std::thread` alias is test-exempt, matching Rule 8.
        if !line.in_test && line.text.contains(THREAD_ALIAS) {
            violations.push(format!(
                "{where_at}: aliasing `std::thread` (`{THREAD_ALIAS}…`) is banned outside tests; it can hide an aliased thread spawn from the policy scanner"
            ));
        }
        if line.text.contains(CROSSBEAM_ALIAS) {
            violations.push(format!(
                "{where_at}: aliasing `crossbeam_channel` (`{CROSSBEAM_ALIAS}…`) is banned; it can hide an aliased unbounded channel from the policy scanner"
            ));
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
fn collect_crate_manifest_info(root: &Path) -> Result<Vec<CrateManifestInfo>> {
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
        let direct_deps = direct_dependency_names(&manifest);
        crates.push(CrateManifestInfo { name, direct_deps });
    }
    Ok(crates)
}

/// Collects the real crate names of every direct dependency across the
/// `[dependencies]`, `[dev-dependencies]`, `[build-dependencies]`, and any
/// `[target.*.dependencies]` tables, deduplicated and sorted.
fn direct_dependency_names(manifest: &toml::Table) -> Vec<String> {
    let mut names = Vec::new();
    collect_dependency_names(manifest, &mut names);
    // Platform-specific `[target.'cfg(...)'.dependencies]` tables count as direct deps.
    if let Some(targets) = manifest.get("target").and_then(toml::Value::as_table) {
        for target in targets.values() {
            if let Some(table) = target.as_table() {
                collect_dependency_names(table, &mut names);
            }
        }
    }
    names.sort();
    names.dedup();
    names
}

/// Appends the resolved crate name of each entry in the dependency tables found
/// directly under `parent`.
fn collect_dependency_names(parent: &toml::Table, names: &mut Vec<String>) {
    for table_name in ["dependencies", "dev-dependencies", "build-dependencies"] {
        if let Some(table) = parent.get(table_name).and_then(toml::Value::as_table) {
            for (key, value) in table {
                names.push(resolved_crate_name(key, value));
            }
        }
    }
}

/// Resolves a dependency entry's real crate name: a `package = "real"` rename when
/// the value is a table, otherwise the table key itself (which covers
/// `x.workspace = true` and plain version strings).
fn resolved_crate_name(key: &str, value: &toml::Value) -> String {
    if let Some(package) = value
        .as_table()
        .and_then(|table| table.get("package"))
        .and_then(toml::Value::as_str)
    {
        return package.to_owned();
    }
    key.to_owned()
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
            violations.iter().any(|v| v.contains(STD_MPSC)),
            "expected a std-mpsc violation, got {violations:?}"
        );
    }

    #[test]
    fn thread_spawn_is_a_violation_outside_tests_but_exempt_in_tests() {
        let token = concat!("thread::", "spawn");
        let code = format!("    {token}(|| do_work());");

        let outside = [line(&code, false)];
        let violations = evaluate_concurrency_policy(&[], &outside);
        assert!(
            violations.iter().any(|v| v.contains(THREAD_SPAWN)),
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
    fn real_repo_passes_concurrency_policy() {
        // Guards against self-flagging and proves the clean repo passes the gate.
        let root = crate::workspace_root().unwrap();
        check_concurrency_policy(&root)
            .expect("the current repo must satisfy the concurrency policy");
    }
}
