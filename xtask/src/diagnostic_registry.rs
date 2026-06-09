// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! `check-diagnostic-registry`: enforce that `docs/VALIDATOR-DIAGNOSTICS.md` lists
//! exactly the diagnostic rule-id literals present in `crates/splot-validate/src`.
//!
//! Validator diagnostics are built via `Diagnostic::{new,error,warning,info}(rule_id, …)`
//! and a few `&'static str` helper fns; every rule id is a plain string literal (there are
//! no `format!`-built ids). The canonical set of emitted ids is therefore extracted
//! *syntactically*: every `"<ns>/<id>"` literal in non-test, non-comment validator source.
//! That set must equal the ids documented between the
//! `<!-- diagnostics-registry:begin -->` / `:end` markers in the registry doc.
//!
//! This is the full-id superset of the prefix-level `scan_diagnostics` guard in
//! [`crate::feature_status`]; both are kept. Tracked as `XTASK-DIAGNOSTIC-REGISTRY`.

use std::collections::BTreeSet;
use std::path::Path;

use anyhow::{Context as _, Result, anyhow, bail};

use crate::feature_status::{collect_files, display_path, is_diagnostic_id};

/// Validator source root scanned for emitted rule-id literals.
const VALIDATE_SRC: &str = "crates/splot-validate/src";
/// Documentation file that must mirror the emitted rule ids.
const REGISTRY_DOC: &str = "docs/VALIDATOR-DIAGNOSTICS.md";
/// Start of the CI-enforced registry region in [`REGISTRY_DOC`].
const BEGIN_MARKER: &str = "<!-- diagnostics-registry:begin -->";
/// End of the CI-enforced registry region in [`REGISTRY_DOC`].
const END_MARKER: &str = "<!-- diagnostics-registry:end -->";

/// Entry point for `cargo xtask check-diagnostic-registry`.
///
/// Fails when an emitted rule id is undocumented, or the registry documents an id that is
/// not present in non-test validator source.
pub(crate) fn check_diagnostic_registry(root: &Path) -> Result<()> {
    let emitted = emitted_ids(root)?;
    let documented = documented_ids(root)?;
    let (undocumented, unemitted) = classify(&emitted, &documented);

    if undocumented.is_empty() && unemitted.is_empty() {
        eprintln!("check-diagnostic-registry: ok ({} ids)", emitted.len());
        return Ok(());
    }

    for id in &undocumented {
        eprintln!(
            "diagnostic registry: rule id `{id}` is present in {VALIDATE_SRC} but not documented between the registry markers in {REGISTRY_DOC}"
        );
    }
    for id in &unemitted {
        eprintln!(
            "diagnostic registry: rule id `{id}` is documented in {REGISTRY_DOC} but not present in non-test {VALIDATE_SRC} source"
        );
    }
    bail!(
        "diagnostic registry out of sync: {} undocumented, {} unemitted; update the registry tables in {REGISTRY_DOC} (between the markers) or the validator source",
        undocumented.len(),
        unemitted.len()
    )
}

/// Splits `emitted`/`documented` into (emitted-but-undocumented, documented-but-unemitted).
fn classify(
    emitted: &BTreeSet<String>,
    documented: &BTreeSet<String>,
) -> (Vec<String>, Vec<String>) {
    let undocumented = emitted.difference(documented).cloned().collect();
    let unemitted = documented.difference(emitted).cloned().collect();
    (undocumented, unemitted)
}

/// The diagnostic rule-id literals present in non-test, non-comment validator source.
fn emitted_ids(root: &Path) -> Result<BTreeSet<String>> {
    let dir = root.join(VALIDATE_SRC);
    let mut ids = BTreeSet::new();
    for path in collect_files(&dir, &["rs"])? {
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("failed to read {}", display_path(root, &path)))?;
        for literal in string_literals_skipping_comments(strip_test_modules(&text)) {
            if is_registry_id(&literal) {
                ids.insert(literal);
            }
        }
    }
    Ok(ids)
}

/// The rule ids documented inside the enforced registry region of [`REGISTRY_DOC`].
fn documented_ids(root: &Path) -> Result<BTreeSet<String>> {
    let path = root.join(REGISTRY_DOC);
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("failed to read {}", display_path(root, &path)))?;
    let region = registry_region(&text).with_context(|| format!("in {REGISTRY_DOC}"))?;
    Ok(backtick_ids(region))
}

/// `true` if `s` is a registry rule id: a diagnostic id with exactly one `/` separator
/// (`<ns>/<id>`). Slash-less or multi-slash strings are not rule ids.
fn is_registry_id(s: &str) -> bool {
    is_diagnostic_id(s) && s.matches('/').count() == 1
}

/// Returns the slice of `text` before the first top-level `#[cfg(test)]` module.
///
/// Every test module in the validate crate is a single top-level `mod tests` that runs to
/// end of file, so cutting from the first `#[cfg(test)]`-followed-by-`mod` line to EOF
/// removes all test-only literals (assertions, `starts_with` prefixes, fake examples). The
/// cut triggers only when the attribute is followed by a `mod` declaration — a
/// `#[cfg(test)]` on a single `fn` does not truncate the real code after it.
fn strip_test_modules(text: &str) -> &str {
    let lines = line_starts(text);
    for (i, (start, line)) in lines.iter().enumerate() {
        if line.trim() != "#[cfg(test)]" {
            continue;
        }
        // Look past further attributes / blank lines for the item this gates.
        for (_, next) in &lines[i + 1..] {
            let trimmed = next.trim();
            if trimmed.is_empty() || trimmed.starts_with("#[") {
                continue;
            }
            if trimmed.starts_with("mod ") || trimmed.starts_with("pub mod ") {
                return &text[..*start];
            }
            break;
        }
    }
    text
}

/// Returns each line of `text` paired with its starting byte offset.
fn line_starts(text: &str) -> Vec<(usize, &str)> {
    let mut out = Vec::new();
    let mut start = 0usize;
    for line in text.split_inclusive('\n') {
        out.push((start, line));
        start += line.len();
    }
    out
}

/// Extracts double-quoted string-literal contents from Rust `code`, skipping `//` line
/// comments, `/* */` block comments (nestable), and char literals (so `'"'` does not open a
/// string), and honoring `\"` / `\\` escapes. Raw strings are not handled — the validator
/// uses none, and rule ids must be plain string literals to be visible here.
fn string_literals_skipping_comments(code: &str) -> Vec<String> {
    let chars: Vec<char> = code.chars().collect();
    let n = chars.len();
    let mut i = 0usize;
    let mut out = Vec::new();
    while i < n {
        let c = chars[i];
        // Line comment: `// … \n`.
        if c == '/' && i + 1 < n && chars[i + 1] == '/' {
            i += 2;
            while i < n && chars[i] != '\n' {
                i += 1;
            }
            continue;
        }
        // Block comment: `/* … */`, nestable.
        if c == '/' && i + 1 < n && chars[i + 1] == '*' {
            i += 2;
            let mut depth = 1u32;
            while i < n && depth > 0 {
                if chars[i] == '/' && i + 1 < n && chars[i + 1] == '*' {
                    depth += 1;
                    i += 2;
                } else if chars[i] == '*' && i + 1 < n && chars[i + 1] == '/' {
                    depth -= 1;
                    i += 2;
                } else {
                    i += 1;
                }
            }
            continue;
        }
        // Char literal or lifetime.
        if c == '\'' {
            if i + 1 < n && chars[i + 1] == '\\' {
                // Escaped char literal: '\n', '\'', '\u{1F}' …
                i += 2;
                while i < n && chars[i] != '\'' {
                    i += 1;
                }
                if i < n {
                    i += 1;
                }
                continue;
            }
            if i + 2 < n && chars[i + 2] == '\'' {
                // Simple char literal: 'a', '"', '/'.
                i += 3;
                continue;
            }
            // Otherwise a lifetime (`'a`); just advance.
            i += 1;
            continue;
        }
        // String literal.
        if c == '"' {
            i += 1;
            let mut s = String::new();
            while i < n {
                match chars[i] {
                    '\\' => i += 2, // skip the escaped char; exact content is irrelevant
                    '"' => {
                        i += 1;
                        break;
                    }
                    other => {
                        s.push(other);
                        i += 1;
                    }
                }
            }
            out.push(s);
            continue;
        }
        i += 1;
    }
    out
}

/// Returns the slice between the begin/end registry markers. Requires *exactly one* of each
/// marker, so a stray mention of a marker (e.g. in prose) cannot silently shrink the region.
fn registry_region(text: &str) -> Result<&str> {
    let begin_count = text.matches(BEGIN_MARKER).count();
    if begin_count != 1 {
        bail!("expected exactly one `{BEGIN_MARKER}` marker, found {begin_count}");
    }
    let end_count = text.matches(END_MARKER).count();
    if end_count != 1 {
        bail!("expected exactly one `{END_MARKER}` marker, found {end_count}");
    }
    let begin = text
        .find(BEGIN_MARKER)
        .ok_or_else(|| anyhow!("missing `{BEGIN_MARKER}` marker"))?;
    let after_begin = begin + BEGIN_MARKER.len();
    let end_rel = text[after_begin..]
        .find(END_MARKER)
        .ok_or_else(|| anyhow!("`{END_MARKER}` precedes the begin marker"))?;
    Ok(&text[after_begin..after_begin + end_rel])
}

/// Collects backtick-wrapped registry ids (`` `<ns>/<id>` ``) from `region`. Only
/// backtick-delimited tokens are considered, so prose words and section numbers in table
/// cells are never mistaken for ids.
fn backtick_ids(region: &str) -> BTreeSet<String> {
    let mut ids = BTreeSet::new();
    let mut rest = region;
    while let Some(open) = rest.find('`') {
        let after = &rest[open + 1..];
        let Some(close) = after.find('`') else {
            break;
        };
        let token = &after[..close];
        if is_registry_id(token) {
            ids.insert(token.to_owned());
        }
        rest = &after[close + 1..];
    }
    ids
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn ids(code: &str) -> Vec<String> {
        string_literals_skipping_comments(code)
    }

    #[test]
    fn scanner_collects_plain_literals() {
        assert_eq!(
            ids(r#"d("ops/foo"); d("brt/bar");"#),
            ["ops/foo", "brt/bar"]
        );
    }

    #[test]
    fn scanner_ignores_line_and_doc_comments() {
        assert_eq!(ids("// \"ops/foo\"\nd(\"brt/bar\");"), ["brt/bar"]);
        assert_eq!(ids("/// see \"ops/foo\"\nd(\"brt/bar\");"), ["brt/bar"]);
    }

    #[test]
    fn scanner_ignores_nested_block_comment() {
        let code = "/* a \"ops/foo\" /* b \"brt/bar\" */ c */ d(\"qm/baz\");";
        assert_eq!(ids(code), ["qm/baz"]);
    }

    #[test]
    fn scanner_keeps_parity_through_escaped_quote() {
        // The escaped `\"` must not end the literal early (escaped content itself is
        // irrelevant for rule ids), so the following literal is still recovered.
        let found = ids(r#"d("a\"b"); d("ops/foo");"#);
        assert_eq!(found.len(), 2);
        assert!(found.contains(&"ops/foo".to_string()));
    }

    #[test]
    fn scanner_treats_comment_markers_inside_string_as_content() {
        assert_eq!(ids(r#"d("http://x"); d("a/*b");"#), ["http://x", "a/*b"]);
    }

    #[test]
    fn scanner_handles_char_literal_with_quote_or_slash() {
        // The char literals must not open a string or a comment.
        assert_eq!(
            ids(r#"match c { '"' => d("ops/foo"), _ => () }"#),
            ["ops/foo"]
        );
        assert_eq!(ids(r#"if c == '/' { d("ops/foo"); }"#), ["ops/foo"]);
    }

    #[test]
    fn registry_id_grammar() {
        assert!(is_registry_id("ops/inherited-op-index-out-of-range"));
        assert!(is_registry_id("bitstream/parse-error"));
        assert!(!is_registry_id("sequence-header/timing-")); // trailing '-'
        assert!(!is_registry_id("parse-error")); // no slash
        assert!(!is_registry_id("a/b/c")); // two slashes
        assert!(!is_registry_id("Ops/Foo")); // uppercase
        assert!(!is_registry_id("ops//foo")); // empty segment
    }

    #[test]
    fn strip_test_modules_cuts_top_level_test_mod() {
        let src = "fn real() { d(\"ops/foo\"); }\n#[cfg(test)]\nmod tests {\n  fn t() { d(\"ops/test\"); }\n}\n";
        assert_eq!(ids(strip_test_modules(src)), ["ops/foo"]);
    }

    #[test]
    fn strip_test_modules_cuts_after_attributes() {
        let src = "fn real() { d(\"ops/foo\"); }\n#[cfg(test)]\n#[allow(clippy::unwrap_used)]\nmod tests {\n  fn t() { d(\"ops/test\"); }\n}\n";
        assert_eq!(ids(strip_test_modules(src)), ["ops/foo"]);
    }

    #[test]
    fn strip_test_modules_keeps_cfg_test_fn() {
        // `#[cfg(test)]` on a fn (not a mod) must not truncate real code that follows.
        let src = "#[cfg(test)]\nfn helper() {}\nfn real() { d(\"ops/foo\"); }\n";
        assert_eq!(ids(strip_test_modules(src)), ["ops/foo"]);
    }

    #[test]
    fn registry_region_extracts_between_markers() {
        let doc = "x `ops/before`\n<!-- diagnostics-registry:begin -->\n`ops/foo` `brt/bar`\n<!-- diagnostics-registry:end -->\ny `ops/after`\n";
        let region = registry_region(doc).unwrap();
        let found = backtick_ids(region);
        assert!(found.contains("ops/foo") && found.contains("brt/bar"));
        assert!(!found.contains("ops/before") && !found.contains("ops/after"));
    }

    #[test]
    fn registry_region_ignores_non_ids() {
        let doc = "<!-- diagnostics-registry:begin -->\n| `ops/foo` | error | 6.10.2 | syntax thing |\n<!-- diagnostics-registry:end -->\n";
        let found = backtick_ids(registry_region(doc).unwrap());
        assert_eq!(found.len(), 1);
        assert!(found.contains("ops/foo"));
    }

    #[test]
    fn registry_region_missing_marker_errors() {
        assert!(registry_region("no markers").is_err());
        assert!(registry_region("<!-- diagnostics-registry:begin -->\nonly begin").is_err());
    }

    #[test]
    fn registry_region_duplicate_marker_errors() {
        // A second mention of a marker (e.g. in prose) must be rejected, not silently used.
        let doc = "<!-- diagnostics-registry:begin -->\n`a/x`\n<!-- diagnostics-registry:end -->\nprose <!-- diagnostics-registry:begin -->\n";
        assert!(registry_region(doc).is_err());
    }

    #[test]
    fn classify_reports_each_direction() {
        let emitted: BTreeSet<String> = ["a/x", "a/y"].iter().map(|s| s.to_string()).collect();
        let documented: BTreeSet<String> = ["a/y", "a/z"].iter().map(|s| s.to_string()).collect();
        let (undocumented, unemitted) = classify(&emitted, &documented);
        assert_eq!(undocumented, ["a/x"]); // emitted, not documented
        assert_eq!(unemitted, ["a/z"]); // documented, not emitted
    }

    #[test]
    fn classify_clean_when_equal() {
        let set: BTreeSet<String> = ["a/x", "a/y"].iter().map(|s| s.to_string()).collect();
        let (undocumented, unemitted) = classify(&set, &set);
        assert!(undocumented.is_empty() && unemitted.is_empty());
    }

    #[test]
    fn real_validate_source_has_known_anchors() {
        // Resilient: assert membership and a floor, not a frozen count (the registry check
        // itself enforces exactness against the doc).
        let root = repo_root();
        let emitted = emitted_ids(&root).unwrap();
        assert!(emitted.contains("bitstream/parse-error"));
        assert!(emitted.contains("ops/inherited-op-index-out-of-range"));
        assert!(
            emitted.len() >= 120,
            "expected >= 120 ids, got {}",
            emitted.len()
        );
        // Test-only fakes/prefixes must not leak past the test-module cut / grammar.
        assert!(!emitted.contains("obu-header/x"));
        assert!(!emitted.contains("sequence-header/timing-"));
    }

    fn repo_root() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("xtask has a parent dir")
            .to_path_buf()
    }
}
