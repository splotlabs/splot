// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Zero-copy media-buffer policy check.
//!
//! `splot` defaults to borrowing decoded media (frames, planes, reference-frame
//! storage, pixel/sample buffers) through view types and never duplicates that
//! storage implicitly. This gate is line-based defense-in-depth that flags the
//! patterns which silently copy a whole frame — a `Clone` derive on a
//! media-storage type, an unmarked sample copy (`.to_vec()` /
//! `copy_from_slice` / `extend_from_slice` / `clone_from_slice` /
//! `Vec::from(&…)`), a `.clone()` on a media-named binding, `Arc/Rc::make_mut`,
//! an unmarked `read_from_bytes`, `unsafe` / `transmute` / `from_raw_parts`, an
//! `include!` bypass — plus banned byte/transmute dependencies and a `zerocopy`
//! dependency outside its approved crates. A flagged copy passes only with a
//! nearby specific `splot-copy-ok: <reason>` marker. The policy and the
//! deliberate scope of this gate are documented in
//! [`docs/ZERO_COPY.md`](../../docs/ZERO_COPY.md).
//!
//! This module is a thin IO wrapper around the pure
//! [`evaluate_zero_copy_policy`] evaluator so the rules can be unit-tested
//! against synthetic fixtures.
//!
//! Feature tracking: `INFRA-ZERO-COPY-MEDIA-POLICY`.

use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context as _, Result, bail};

/// Crates allowed to declare a direct `zerocopy` dependency: `splot-core` for
/// fixed-layout container/wire headers and `splot-recon` only with a documented
/// raw-sample view (see `docs/ZERO_COPY.md`). Never the decoder/encoder/validator/
/// CLI/parallel crates.
const ZEROCOPY_APPROVED_CRATES: &[&str] = &["splot-core", "splot-recon"];

/// Byte/transmute/zero-copy alternative crates that would bypass the splot view
/// model or the single approved `zerocopy` wire surface. None may be added.
const BANNED_BYTE_CRATES: &[&str] = &[
    "bytes",
    "bytemuck",
    "safe-transmute",
    "rkyv",
    "memmap2",
    "smallvec",
    "arrayvec",
];

/// Exact type names treated as large media storage: a `Clone` derive/impl on any
/// of these duplicates frame/plane/reference/sample storage. Matched by exact
/// identifier so small-metadata names (`PlaneSize`, `PlaneRect`, `PlaneId`,
/// `DecodedFrameInfo`, `BitDepth`, `PixelFormat`, `ReferenceSlot`, `OutputIndex`)
/// and borrow views (`PlaneRef`, `FrameRef`, …) are never caught.
const MEDIA_TYPE_NAMES: &[&str] = &[
    // Concrete splot-recon storage types.
    "Plane",
    "FramePlanes",
    "DecodedFrame",
    "CurrentFrameWorkspace",
    "CurrentFramePlane",
    "ReferenceFrameStore",
    // Forward-looking generic media-storage names from docs/ZERO_COPY.md.
    "Frame",
    "CurrentFrame",
    "ReferenceFrame",
    "FrameStore",
    "LookaheadFrame",
    "FrameBuffer",
    "SampleBuffer",
    "PixelBuffer",
    "Workspace",
    "Reconstruction",
];

/// Binding names whose `.clone()` is suspicious: cloning one of these likely
/// duplicates a media buffer. Matched against the identifier immediately before
/// `.clone()`, so `frame.rows.clone()` (a `rows` clone) is not flagged.
const SUSPICIOUS_CLONE_BINDINGS: &[&str] = &[
    "frame",
    "ref_frame",
    "reference",
    "plane",
    "samples",
    "pixels",
    "buffer",
    "workspace",
    "lookahead",
    "current",
    "decoded",
    "recon",
];

/// Bulk sample-copy needles flagged in `splot-recon/src` unless a nearby specific
/// `splot-copy-ok:` marker names the materialization boundary. Each entry is
/// `(needle, human-readable form)`.
const SAMPLE_COPY_NEEDLES: &[(&str, &str)] = &[
    (".to_vec(", ".to_vec()"),
    ("Vec::from(&", "Vec::from(&…)"),
    ("extend_from_slice", "extend_from_slice"),
    ("copy_from_slice", "copy_from_slice"),
    ("clone_from_slice", "clone_from_slice"),
];

/// Vague `splot-copy-ok:` reasons the gate rejects: a reviewer cannot tell which
/// boundary forced the copy. Compared case-insensitively against the trimmed
/// reason.
const VAGUE_MARKER_REASONS: &[&str] = &["temporary", "fix", "needed", "convenience", "todo"];

/// The `splot-copy-ok:` marker token.
const MARKER_TOKEN: &str = "splot-copy-ok";

/// One workspace crate reduced to its name and resolved direct dependency names.
pub(crate) struct ZcCrate {
    /// The crate's `[package].name`.
    pub name: String,
    /// Direct dependency crate names (real names; `package = "…"` resolved).
    pub deps: Vec<String>,
}

/// One scanned source line: display path (forward-slash normalized), 1-based line
/// number, and text (trailing newline stripped).
pub(crate) struct ZcSourceLine {
    /// Display path of the source file.
    pub path: String,
    /// 1-based line number within the file.
    pub line_no: usize,
    /// The line's text.
    pub text: String,
}

/// Marker presence/quality for a copy site.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MarkerStatus {
    /// No `splot-copy-ok` marker on the line or the preceding two lines.
    Missing,
    /// A marker is present but its reason is empty or vague.
    Vague,
    /// A marker with a specific boundary reason is present.
    Specific,
}

/// Returns whether `path` (forward-slash, repo-relative) is under `crate_name`'s
/// `src/` directory.
fn is_src_of(path: &str, crate_name: &str) -> bool {
    path.starts_with(&format!("crates/{crate_name}/src/"))
}

/// `splot-recon/src` — the only crate that owns decoded sample buffers today, so
/// the bulk sample-copy needles are scanned here.
fn is_recon_src(path: &str) -> bool {
    is_src_of(path, "splot-recon")
}

/// Media crates whose `src` is scanned for the `include!` bypass: `splot-core`
/// legitimately `include!`s test modules under `src/write/`, so it is excluded.
fn is_media_crate_src(path: &str) -> bool {
    is_recon_src(path) || is_src_of(path, "splot-decode") || is_src_of(path, "splot-encode")
}

/// Crates scanned for `Clone`/`make_mut`/`unsafe`/`read_from_bytes` and the
/// suspicious `.clone()` rule: the media crates plus `splot-core` (compressed
/// payload/container code).
fn is_clone_scan_src(path: &str) -> bool {
    is_media_crate_src(path) || is_src_of(path, "splot-core")
}

/// Classifies the `splot-copy-ok` marker (if any) in `text`.
fn marker_in_text(text: &str) -> Option<MarkerStatus> {
    let pos = text.find(MARKER_TOKEN)?;
    let after = &text[pos + MARKER_TOKEN.len()..];
    match after.strip_prefix(':') {
        None => Some(MarkerStatus::Vague), // `splot-copy-ok` with no reason
        Some(reason) => {
            let reason = reason.trim();
            let lower = reason.to_ascii_lowercase();
            if reason.is_empty()
                || VAGUE_MARKER_REASONS.contains(&lower.as_str())
                || lower.starts_with("todo")
            {
                Some(MarkerStatus::Vague)
            } else {
                Some(MarkerStatus::Specific)
            }
        }
    }
}

/// Returns the best marker status on `sources[i]` or the preceding two lines of
/// the same file (a specific marker anywhere in that window wins).
fn marker_status(sources: &[ZcSourceLine], i: usize) -> MarkerStatus {
    let target = &sources[i];
    let mut best = MarkerStatus::Missing;
    let mut j = i;
    loop {
        let line = &sources[j];
        if line.path != target.path || target.line_no.saturating_sub(line.line_no) > 2 {
            break;
        }
        if let Some(status) = marker_in_text(&line.text) {
            match status {
                MarkerStatus::Specific => return MarkerStatus::Specific,
                MarkerStatus::Vague => best = MarkerStatus::Vague,
                MarkerStatus::Missing => {}
            }
        }
        if j == 0 {
            break;
        }
        j -= 1;
    }
    best
}

/// Records a violation for a marker-suppressible copy at `sources[i]`: passes on a
/// specific marker, flags a vague marker, and flags an unmarked copy with `unmarked`.
fn check_marked_copy(sources: &[ZcSourceLine], i: usize, what: &str, violations: &mut Vec<String>) {
    let line = &sources[i];
    let where_at = format!("{}:{}", line.path, line.line_no);
    match marker_status(sources, i) {
        MarkerStatus::Specific => {}
        MarkerStatus::Vague => violations.push(format!(
            "{where_at}: vague `{MARKER_TOKEN}` marker for {what}; name the specific materialization boundary"
        )),
        MarkerStatus::Missing => violations.push(format!(
            "{where_at}: {what}; add a specific `{MARKER_TOKEN}: <reason>` marker only if this is an intentional materialization boundary"
        )),
    }
}

/// If `text` declares a type, returns its identifier.
fn declared_type_name(text: &str) -> Option<&str> {
    let mut t = text.trim_start();
    for prefix in ["pub(crate) ", "pub(super) ", "pub ", "pub(in crate) "] {
        if let Some(rest) = t.strip_prefix(prefix) {
            t = rest.trim_start();
            break;
        }
    }
    for keyword in ["struct ", "enum ", "union "] {
        if let Some(rest) = t.strip_prefix(keyword) {
            let end = rest
                .find(|c: char| !(c.is_alphanumeric() || c == '_'))
                .unwrap_or(rest.len());
            if end > 0 {
                return Some(&rest[..end]);
            }
        }
    }
    None
}

/// Returns the identifier immediately preceding the `.clone(` at byte `dot_pos`.
fn clone_receiver(text: &str, dot_pos: usize) -> &str {
    let bytes = text.as_bytes();
    let mut start = dot_pos;
    while start > 0 {
        let c = bytes[start - 1];
        if c.is_ascii_alphanumeric() || c == b'_' {
            start -= 1;
        } else {
            break;
        }
    }
    &text[start..dot_pos]
}

/// Returns whether `text` (a non-comment code line) uses the `unsafe` keyword.
fn uses_unsafe_keyword(text: &str) -> bool {
    text.contains("unsafe ") || text.contains("unsafe{") || text.trim() == "unsafe"
}

/// Evaluates the zero-copy policy against owned manifest and source inputs,
/// returning sorted, human-readable violation strings.
///
/// This is the pure core of the check: it performs no IO, so it can be exercised
/// directly with synthetic fixtures. See the module docs for the rule set and
/// `docs/ZERO_COPY.md` for the policy.
pub(crate) fn evaluate_zero_copy_policy(
    crates: &[ZcCrate],
    sources: &[ZcSourceLine],
) -> Vec<String> {
    let mut violations: Vec<String> = Vec::new();

    // Dependency rules (all manifests).
    for krate in crates {
        for dep in &krate.deps {
            if dep == "zerocopy" && !ZEROCOPY_APPROVED_CRATES.contains(&krate.name.as_str()) {
                violations.push(format!(
                    "{}: `zerocopy` may only be a direct dependency of {} (private fixed-layout wire views only)",
                    krate.name,
                    ZEROCOPY_APPROVED_CRATES.join(" or ")
                ));
            }
            if BANNED_BYTE_CRATES.contains(&dep.as_str()) {
                violations.push(format!(
                    "{}: banned byte/transmute crate `{}`; use splot views or the approved `zerocopy` wire surface",
                    krate.name, dep
                ));
            }
        }
    }

    // Source rules.
    for (i, line) in sources.iter().enumerate() {
        let text = &line.text;
        let trimmed = text.trim_start();
        // Comment-only lines are never copies; they may carry `splot-copy-ok`
        // markers (used by the lookback) or prose naming a banned construct.
        if trimmed.starts_with("//") {
            continue;
        }
        let where_at = format!("{}:{}", line.path, line.line_no);

        if is_clone_scan_src(&line.path) {
            // Rule: `Clone` derive on a media-storage type.
            if let Some(name) = declared_type_name(text)
                && MEDIA_TYPE_NAMES.contains(&name)
                && preceding_clone_derive(sources, i)
            {
                violations.push(format!(
                    "{where_at}: `Clone` derive on media-storage type `{name}`; remove it (borrow a view or share via `SharedFrame` instead)"
                ));
            }
            // Rule: `impl Clone for` a media-storage type.
            if let Some(name) = clone_impl_target(text)
                && MEDIA_TYPE_NAMES.contains(&name)
            {
                violations.push(format!(
                    "{where_at}: `impl Clone for {name}` duplicates media storage; remove it (share via `SharedFrame` instead)"
                ));
            }
            // Rule: suspicious `.clone()` on a media-named binding.
            for (pos, _) in text.match_indices(".clone(") {
                let receiver = clone_receiver(text, pos);
                if SUSPICIOUS_CLONE_BINDINGS.contains(&receiver) {
                    check_marked_copy(
                        sources,
                        i,
                        &format!("suspicious media copy: `{receiver}.clone()`"),
                        &mut violations,
                    );
                }
            }
            // Rule: copy-on-write on shared frame storage.
            if text.contains("make_mut") {
                violations.push(format!(
                    "{where_at}: `make_mut` copy-on-write on shared storage is banned; never mutate shared frame storage in place"
                ));
            }
            // Rule: no unsafe byte reinterpretation.
            if uses_unsafe_keyword(text) {
                violations.push(format!(
                    "{where_at}: `unsafe` is banned (workspace `unsafe_code = \"forbid\"`); do not reinterpret bytes as samples"
                ));
            }
            if text.contains("transmute") {
                violations.push(format!(
                    "{where_at}: `transmute` is banned; convert through a validated domain type"
                ));
            }
            if text.contains("from_raw_parts") {
                violations.push(format!(
                    "{where_at}: `from_raw_parts` is banned; build views from safe slices"
                ));
            }
            // Rule: `read_from_bytes` copies unless marked a tiny wire-header copy.
            if text.contains("read_from_bytes") {
                check_marked_copy(
                    sources,
                    i,
                    "`read_from_bytes` copy (prefer a `ref_from_*` borrow)",
                    &mut violations,
                );
            }
        }

        // Rule: bulk sample copies (splot-recon/src only).
        if is_recon_src(&line.path) {
            for (needle, shown) in SAMPLE_COPY_NEEDLES {
                if text.contains(needle) {
                    check_marked_copy(
                        sources,
                        i,
                        &format!("suspicious media copy: `{shown}`"),
                        &mut violations,
                    );
                }
            }
        }

        // Rule: `include!` bypass in the media crates (could hide a copy).
        if is_media_crate_src(&line.path) && text.contains("include!(") {
            violations.push(format!(
                "{where_at}: `include!` is banned in media crates (it can hide a copy from this scan)"
            ));
        }
    }

    violations.sort();
    violations.dedup();
    violations
}

/// Returns whether the attribute block immediately above `sources[i]` contains a
/// `#[derive(... Clone ...)]`.
fn preceding_clone_derive(sources: &[ZcSourceLine], i: usize) -> bool {
    let path = &sources[i].path;
    let mut j = i;
    while j > 0 {
        j -= 1;
        let prev = &sources[j];
        if &prev.path != path {
            return false;
        }
        let trimmed = prev.text.trim_start();
        if trimmed.starts_with("#[") {
            if trimmed.contains("derive(") && trimmed.contains("Clone") {
                return true;
            }
            continue;
        }
        // Doc comments and blank lines may separate the derive from the item.
        if trimmed.is_empty() || trimmed.starts_with("//") {
            continue;
        }
        return false;
    }
    false
}

/// If `text` is an `impl Clone for <Type>` line, returns the target type name.
fn clone_impl_target(text: &str) -> Option<&str> {
    let trimmed = text.trim_start();
    if !trimmed.starts_with("impl") {
        return None;
    }
    let pos = text.find("Clone for ")?;
    let rest = text[pos + "Clone for ".len()..].trim_start();
    let end = rest
        .find(|c: char| !(c.is_alphanumeric() || c == '_'))
        .unwrap_or(rest.len());
    (end > 0).then(|| &rest[..end])
}

/// Verifies the workspace honors the zero-copy media-buffer policy.
///
/// Reads every workspace member's manifest for direct dependencies and walks the
/// `splot-core`/`splot-recon`/`splot-decode`/`splot-encode` source trees, then
/// applies [`evaluate_zero_copy_policy`]. Fails the gate on any violation.
///
/// # Errors
/// Returns an error if a manifest or source file cannot be read/parsed, or if the
/// evaluator reports one or more policy violations.
pub(crate) fn check_zero_copy_policy(root: &Path) -> Result<()> {
    let crates = collect_crate_deps(root)?;
    let sources = collect_source_lines(root)?;
    let violations = evaluate_zero_copy_policy(&crates, &sources);

    if violations.is_empty() {
        eprintln!("check-zero-copy-policy: ok");
        Ok(())
    } else {
        for violation in &violations {
            eprintln!("{violation}");
        }
        bail!("check-zero-copy-policy: {} violation(s)", violations.len())
    }
}

/// Builds the per-crate resolved direct-dependency view from every workspace
/// member's manifest (resolving `package = "…"` renames and `x.workspace = true`
/// aliases, like the dependency-direction and concurrency gates).
fn collect_crate_deps(root: &Path) -> Result<Vec<ZcCrate>> {
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
        let deps = direct_dependency_names(&manifest, &workspace_deps);
        crates.push(ZcCrate { name, deps });
    }
    Ok(crates)
}

/// Resolved direct dependency names across the dependency tables (deduplicated).
fn direct_dependency_names(
    manifest: &toml::Table,
    workspace_deps: &HashMap<String, String>,
) -> Vec<String> {
    let mut names = Vec::new();
    for table_name in ["dependencies", "dev-dependencies", "build-dependencies"] {
        if let Some(table) = manifest.get(table_name).and_then(toml::Value::as_table) {
            for (key, value) in table {
                names.push(crate::resolved_dep_name(key, value, workspace_deps));
            }
        }
    }
    if let Some(targets) = manifest.get("target").and_then(toml::Value::as_table) {
        for target in targets.values() {
            if let Some(table) = target.as_table() {
                for inner in ["dependencies", "dev-dependencies", "build-dependencies"] {
                    if let Some(deps) = table.get(inner).and_then(toml::Value::as_table) {
                        for (key, value) in deps {
                            names.push(crate::resolved_dep_name(key, value, workspace_deps));
                        }
                    }
                }
            }
        }
    }
    names.sort();
    names.dedup();
    names
}

/// Walks the scanned crate `src` trees and returns every line, sorted by
/// `(path, line_no)`. All lines (including comments) are kept so the marker
/// lookback can see `splot-copy-ok` comments above a copy.
fn collect_source_lines(root: &Path) -> Result<Vec<ZcSourceLine>> {
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
                if !matches!(
                    path.file_name().and_then(|name| name.to_str()),
                    Some("target" | ".git")
                ) {
                    stack.push(path);
                }
            } else if path.extension().and_then(|ext| ext.to_str()) == Some("rs") {
                let display = path
                    .strip_prefix(root)
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .replace('\\', "/");
                if !is_clone_scan_src(&display) {
                    continue;
                }
                let contents = std::fs::read_to_string(&path)
                    .with_context(|| format!("failed to read {}", path.display()))?;
                for (index, raw) in contents.lines().enumerate() {
                    sources.push(ZcSourceLine {
                        path: display.clone(),
                        line_no: index + 1,
                        text: raw.to_owned(),
                    });
                }
            }
        }
    }
    sources.sort_by(|a, b| (a.path.as_str(), a.line_no).cmp(&(b.path.as_str(), b.line_no)));
    Ok(sources)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    fn krate(name: &str, deps: &[&str]) -> ZcCrate {
        ZcCrate {
            name: name.to_owned(),
            deps: deps.iter().map(|d| (*d).to_owned()).collect(),
        }
    }

    /// Builds source lines for one synthetic file, numbering from 1.
    fn file(path: &str, lines: &[&str]) -> Vec<ZcSourceLine> {
        lines
            .iter()
            .enumerate()
            .map(|(i, text)| ZcSourceLine {
                path: path.to_owned(),
                line_no: i + 1,
                text: (*text).to_owned(),
            })
            .collect()
    }

    fn run_src(sources: &[ZcSourceLine]) -> Vec<String> {
        evaluate_zero_copy_policy(&[], sources)
    }

    const RECON: &str = "crates/splot-recon/src/x.rs";
    const CORE: &str = "crates/splot-core/src/x.rs";
    const DECODE: &str = "crates/splot-decode/src/x.rs";

    #[test]
    fn zerocopy_in_approved_crate_is_ok() {
        assert!(
            evaluate_zero_copy_policy(&[krate("splot-core", &["zerocopy", "thiserror"])], &[])
                .is_empty()
        );
        assert!(evaluate_zero_copy_policy(&[krate("splot-recon", &["zerocopy"])], &[]).is_empty());
    }

    #[test]
    fn zerocopy_in_disallowed_crate_is_a_violation() {
        let v = evaluate_zero_copy_policy(&[krate("splot-decode", &["zerocopy"])], &[]);
        assert!(
            v.iter()
                .any(|m| m.contains("zerocopy") && m.contains("splot-decode")),
            "got {v:?}"
        );
    }

    #[test]
    fn banned_byte_crate_is_a_violation() {
        for dep in ["bytemuck", "bytes", "rkyv", "smallvec"] {
            let v = evaluate_zero_copy_policy(&[krate("splot-core", &[dep])], &[]);
            assert!(
                v.iter().any(|m| m.contains(dep)),
                "expected `{dep}` flagged, got {v:?}"
            );
        }
    }

    #[test]
    fn media_clone_derive_is_a_violation() {
        let src = file(
            RECON,
            &[
                "#[derive(Clone, Debug, Eq, PartialEq)]",
                "pub struct DecodedFrame<T> {",
            ],
        );
        let v = run_src(&src);
        assert!(
            v.iter()
                .any(|m| m.contains("Clone` derive on media-storage type `DecodedFrame")),
            "got {v:?}"
        );
    }

    #[test]
    fn small_metadata_clone_derive_is_ok() {
        // `DecodedFrameInfo` is small metadata: exact-name matching must not flag it.
        let src = file(
            RECON,
            &[
                "#[derive(Clone, Copy, Debug)]",
                "pub struct DecodedFrameInfo {",
            ],
        );
        assert!(
            run_src(&src).is_empty(),
            "DecodedFrameInfo must not be flagged"
        );
    }

    #[test]
    fn view_type_clone_derive_is_ok() {
        // Borrow views are cheap and may be Copy/Clone; their names are not media names.
        let src = file(
            RECON,
            &[
                "#[derive(Clone, Copy, Debug)]",
                "pub struct PlaneRef<'a, T> {",
            ],
        );
        assert!(run_src(&src).is_empty(), "PlaneRef must not be flagged");
    }

    #[test]
    fn impl_clone_for_media_type_is_a_violation() {
        let src = file(RECON, &["impl<T> Clone for Plane<T> {"]);
        let v = run_src(&src);
        assert!(
            v.iter().any(|m| m.contains("impl Clone for Plane")),
            "got {v:?}"
        );
    }

    #[test]
    fn suspicious_clone_on_media_binding_is_a_violation() {
        let v = run_src(&file(RECON, &["    let copy = frame.clone();"]));
        assert!(v.iter().any(|m| m.contains("frame.clone()")), "got {v:?}");
    }

    #[test]
    fn clone_on_non_media_binding_is_ok() {
        // `frame.rows.clone()` clones `rows`, not `frame`; `digest.clone()` is unrelated.
        let src = file(
            CORE,
            &[
                "    let a = frame.rows.clone();",
                "    let b = digest.clone();",
            ],
        );
        assert!(
            run_src(&src).is_empty(),
            "non-media clones must not be flagged"
        );
    }

    #[test]
    fn marked_clone_is_ok_with_specific_reason() {
        let src = file(
            RECON,
            &[
                "    // splot-copy-ok: materialize external encoder input; lookahead retains it",
                "    let retained = frame.clone();",
            ],
        );
        assert!(
            run_src(&src).is_empty(),
            "a specifically-marked clone must pass"
        );
    }

    #[test]
    fn vague_marker_is_a_violation() {
        let src = file(
            RECON,
            &[
                "    // splot-copy-ok: convenience",
                "    let v = samples.to_vec();",
            ],
        );
        let v = run_src(&src);
        assert!(v.iter().any(|m| m.contains("vague")), "got {v:?}");
    }

    #[test]
    fn bare_marker_without_reason_is_vague() {
        let src = file(RECON, &["    let v = samples.to_vec(); // splot-copy-ok"]);
        let v = run_src(&src);
        assert!(v.iter().any(|m| m.contains("vague")), "got {v:?}");
    }

    #[test]
    fn unmarked_sample_copy_in_recon_is_a_violation() {
        for code in [
            "    let v = samples.to_vec();",
            "    dst.copy_from_slice(&src);",
            "    out.extend_from_slice(&row);",
            "    out.clone_from_slice(&row);",
            "    let v = Vec::from(&samples[..]);",
        ] {
            let v = run_src(&file(RECON, &[code]));
            assert!(!v.is_empty(), "expected `{code}` flagged in recon");
        }
    }

    #[test]
    fn sample_copy_outside_recon_is_not_flagged() {
        // splot-decode/core copy compressed bytes, not samples; not scanned for these.
        for path in [CORE, DECODE] {
            let src = file(
                path,
                &[
                    "    let bytes = payload.to_vec();",
                    "    a.extend_from_slice(&b);",
                ],
            );
            assert!(
                run_src(&src).is_empty(),
                "{path} bulk copies must not be flagged"
            );
        }
    }

    #[test]
    fn marker_on_same_line_suppresses() {
        let src = file(
            RECON,
            &["    out.extend_from_slice(&row); // splot-copy-ok: serialize output bytes"],
        );
        assert!(run_src(&src).is_empty());
    }

    #[test]
    fn marker_three_lines_away_does_not_suppress() {
        let src = file(
            RECON,
            &[
                "    // splot-copy-ok: serialize output",
                "    let _pad = 0;",
                "    let _pad2 = 0;",
                "    let v = samples.to_vec();",
            ],
        );
        let v = run_src(&src);
        assert!(!v.is_empty(), "a marker 3 lines above must not suppress");
    }

    #[test]
    fn make_mut_is_always_a_violation() {
        let v = run_src(&file(RECON, &["    Arc::make_mut(&mut frame_arc);"]));
        assert!(v.iter().any(|m| m.contains("make_mut")), "got {v:?}");
    }

    #[test]
    fn unsafe_transmute_and_from_raw_parts_are_violations() {
        assert!(!run_src(&file(RECON, &["    unsafe { do_it(); }"])).is_empty());
        assert!(!run_src(&file(RECON, &["    let x = core::mem::transmute(y);"])).is_empty());
        assert!(!run_src(&file(RECON, &["    let s = slice::from_raw_parts(p, n);"])).is_empty());
    }

    #[test]
    fn read_from_bytes_is_marker_suppressible() {
        let unmarked = run_src(&file(CORE, &["    let h = Header::read_from_bytes(b)?;"]));
        assert!(
            !unmarked.is_empty(),
            "unmarked read_from_bytes must be flagged"
        );
        let marked = run_src(&file(
            CORE,
            &[
                "    // splot-copy-ok: tiny IVF wire-header copy",
                "    let h = Header::read_from_bytes(b)?;",
            ],
        ));
        assert!(
            marked.is_empty(),
            "a marked tiny wire-header copy must pass"
        );
    }

    #[test]
    fn include_in_media_crate_is_a_violation_but_core_is_ok() {
        let recon = run_src(&file(RECON, &["include!(\"x_tests.rs\");"]));
        assert!(
            recon.iter().any(|m| m.contains("include!")),
            "got {recon:?}"
        );
        // splot-core legitimately includes test modules under src/write/.
        let core = run_src(&file(CORE, &["include!(\"x_tests.rs\");"]));
        assert!(core.is_empty(), "core include! must not be flagged");
    }

    #[test]
    fn comment_prose_naming_a_pattern_is_not_flagged() {
        let src = file(
            RECON,
            &["    // never call samples.to_vec() or frame.clone() on media buffers"],
        );
        assert!(
            run_src(&src).is_empty(),
            "comment prose must not be flagged"
        );
    }

    #[test]
    fn real_repo_passes_zero_copy_policy() {
        // Guards against self-flagging and proves the clean repo passes the gate.
        let root = crate::workspace_root().unwrap();
        check_zero_copy_policy(&root).expect("the current repo must satisfy the zero-copy policy");
    }
}
