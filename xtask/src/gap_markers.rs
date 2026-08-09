// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Marker check for the unified decode engine.
//!
//! The decode engine flags every unimplemented feature at the op that consumes it
//! with a typed `decode/unsupported-feature` marker. `gap!` is the unified marker;
//! the tier-specific families ([`MARKER_MACROS`] — `inter_cap!`, `general_intra_at!`,
//! …) are ergonomic wrappers that funnel to the same carrier. Their first `"reason"`
//! literals form one global inventory of what the engine still misses, and the
//! decoder-output oracle asserts a fixture's recorded `reason` against it. This
//! check protects that inventory two ways:
//!
//! 1. **Uniqueness** — no two marker sites share a `reason` id, across every family.
//!    A collision would let the oracle satisfy one feature's `xfail` assertion with a
//!    different feature's marker, silently accepting a regression under a
//!    wrong-but-expected reason.
//! 2. **Count floor** — the marker count may not drop below [`GAP_MARKER_FLOOR`], so
//!    removing a guard (which would let a stream decode to wrong pixels instead of
//!    failing closed) cannot pass unnoticed. Raise the floor in the same commit that
//!    adds markers; lowering it is a reviewed edit.
//!
//! The scan is a line/char lexer over production decode source (inline
//! `#[cfg(test)]` modules and `*_tests.rs` files excluded), so a marker mentioned in
//! a comment or string does not count.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::Path;

use anyhow::{Context as _, Result, bail};

use crate::diagnostic_registry::strip_test_modules;
use crate::feature_status::collect_files;

/// Decode-crate source root scanned for `gap!` markers.
const DECODE_SRC: &str = "crates/splot-decode/src";

/// Lowest permitted count of unimplemented-feature markers in production decode
/// source. Raised as markers are added; a count below this floor fails the check,
/// so removing a guard (which would let a stream decode to wrong pixels instead of
/// failing closed) cannot pass unnoticed. Lowering it is a reviewed edit.
const GAP_MARKER_FLOOR: usize = 169;

/// One `gap!("reason", …)` marker site: its reason id and the file it lives in.
struct GapSite {
    reason: String,
    file: String,
}

struct MarkerScan {
    reasons: Vec<String>,
    non_literal_sites: Vec<NonLiteralMarker>,
}

struct NonLiteralMarker {
    macro_name: &'static str,
    line: usize,
}

/// Extracts the `reason` literal of every `gap!("…", …)` call in `code`, skipping
/// `//` line comments, `/* */` block comments, char literals, and string literals
/// (so a `gap!` inside a comment or string is not matched). Marker calls whose
/// first argument is not a literal are returned separately so the convention can
/// be enforced instead of silently omitting them from the inventory.
fn scan_gap_reasons(code: &str) -> MarkerScan {
    let chars: Vec<char> = code.chars().collect();
    let n = chars.len();
    let mut i = 0usize;
    let mut prev_ident = false;
    let mut reasons = Vec::new();
    let mut non_literal_sites = Vec::new();
    while i < n {
        let c = chars[i];
        if c == '/' && i + 1 < n && chars[i + 1] == '/' {
            i += 2;
            while i < n && chars[i] != '\n' {
                i += 1;
            }
            prev_ident = false;
            continue;
        }
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
            prev_ident = false;
            continue;
        }
        if c == '\'' {
            // A lifetime (`'a`/`'_`) is an unclosed apostrophe; skip only the `'` so a later marker stays visible.
            let is_lifetime = i + 1 < n
                && (chars[i + 1].is_alphabetic() || chars[i + 1] == '_')
                && !(i + 2 < n && chars[i + 2] == '\'');
            if is_lifetime {
                prev_ident = false;
                i += 1;
                continue;
            }
            i += 1;
            if i < n && chars[i] == '\\' {
                i += 2;
            } else {
                i += 1;
            }
            while i < n && chars[i] != '\'' {
                i += 1;
            }
            if i < n {
                i += 1;
            }
            prev_ident = false;
            continue;
        }
        if c == '"' {
            i = skip_string_literal(&chars, i).0;
            prev_ident = false;
            continue;
        }
        if !prev_ident && let Some((macro_name, bang_len)) = matches_marker_bang(&chars, i) {
            if let Some((reason, next)) = gap_reason_after(&chars, i + bang_len) {
                reasons.push(reason);
                i = next;
            } else {
                non_literal_sites.push(NonLiteralMarker {
                    macro_name,
                    line: chars[..i].iter().filter(|&&ch| ch == '\n').count() + 1,
                });
                i += bang_len;
            }
            prev_ident = false;
            continue;
        }
        prev_ident = c.is_alphanumeric() || c == '_';
        i += 1;
    }
    MarkerScan {
        reasons,
        non_literal_sites,
    }
}

/// Every unimplemented-feature marker macro. Their first `"reason"` literals share
/// one global namespace regardless of decode tier, so a fixture's recorded reason
/// maps to exactly one marker site. `gap!` is the unified target; the tier-specific
/// families are ergonomic wrappers that all funnel to the same diagnostic carrier.
const MARKER_MACROS: &[&str] = &[
    "gap",
    "inter_cap",
    "inter_missing",
    "inter_diag",
    "compound_cap",
    "compound_missing",
    "general_intra_at",
];

/// When `chars[i..]` begins one of [`MARKER_MACROS`] immediately followed by `!`,
/// returns its name and the length of that `name!` token; otherwise `None`.
fn matches_marker_bang(chars: &[char], i: usize) -> Option<(&'static str, usize)> {
    for &name in MARKER_MACROS {
        let name_chars: Vec<char> = name.chars().collect();
        let end = i + name_chars.len();
        if end < chars.len() && chars[i..end] == name_chars[..] && chars[end] == '!' {
            return Some((name, name_chars.len() + 1));
        }
    }
    None
}

/// Given `chars` positioned just past `gap!`, returns the reason literal and the
/// index after it when the call opens as `(  "reason"`, else `None`.
fn gap_reason_after(chars: &[char], mut j: usize) -> Option<(String, usize)> {
    let n = chars.len();
    while j < n && chars[j].is_whitespace() {
        j += 1;
    }
    if j >= n || chars[j] != '(' {
        return None;
    }
    j += 1;
    while j < n && chars[j].is_whitespace() {
        j += 1;
    }
    if j >= n || chars[j] != '"' {
        return None;
    }
    let (next, reason) = skip_string_literal(chars, j);
    Some((reason, next))
}

/// Consumes the string literal starting at `chars[start] == '"'`; returns the index
/// after the closing quote and the (unescaped) contents.
fn skip_string_literal(chars: &[char], start: usize) -> (usize, String) {
    let n = chars.len();
    let mut i = start + 1;
    let mut s = String::new();
    while i < n {
        match chars[i] {
            '\\' => {
                if i + 1 < n {
                    s.push(chars[i + 1]);
                }
                i += 2;
            }
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
    (i, s)
}

/// Fails when a reason id is shared by two `gap!` sites or the marker count drops
/// below `floor`.
fn evaluate(files: &[(String, String)], floor: usize) -> Result<usize> {
    let mut sites: Vec<GapSite> = Vec::new();
    let mut non_literals = String::new();
    for (path, text) in files {
        let scan = scan_gap_reasons(strip_test_modules(text));
        for site in scan.non_literal_sites {
            let _ = write!(
                non_literals,
                "\n  {path}:{}: `{}!` reason must be a string literal",
                site.line, site.macro_name
            );
        }
        for reason in scan.reasons {
            sites.push(GapSite {
                reason,
                file: path.clone(),
            });
        }
    }

    if !non_literals.is_empty() {
        bail!("gap marker reason ids must be string literals:{non_literals}");
    }

    let mut by_reason: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for site in &sites {
        by_reason
            .entry(site.reason.as_str())
            .or_default()
            .push(site.file.as_str());
    }
    let mut collisions = String::new();
    for (reason, files) in &by_reason {
        if files.len() > 1 {
            let _ = write!(collisions, "\n  {reason} in {}", files.join(", "));
        }
    }
    if !collisions.is_empty() {
        bail!("gap! reason ids must be globally unique; duplicates:{collisions}");
    }

    if sites.len() < floor {
        bail!(
            "gap! marker count {} dropped below floor {floor}; a removed marker un-guards an \
             unimplemented feature. Restore the marker or lower GAP_MARKER_FLOOR in a reviewed \
             commit.",
            sites.len()
        );
    }
    Ok(sites.len())
}

/// Verifies `gap!` reason-id uniqueness and the monotonic marker-count floor over
/// production decode source.
pub(crate) fn check_gap_markers(root: &Path) -> Result<()> {
    let dir = root.join(DECODE_SRC);
    let mut files: Vec<(String, String)> = Vec::new();
    for path in collect_files(&dir, &["rs"])? {
        if path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.ends_with("_tests.rs"))
        {
            continue;
        }
        let rel = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .display()
            .to_string();
        let text =
            std::fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
        files.push((rel, text));
    }
    let count = evaluate(&files, GAP_MARKER_FLOOR)?;
    eprintln!(
        "check-gap-markers: {count} unimplemented-feature markers, reason ids unique \
         (floor {GAP_MARKER_FLOOR})"
    );
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::{evaluate, scan_gap_reasons};

    #[test]
    fn scans_literal_reasons_and_skips_comments_and_strings() {
        let code = r#"
            let a = gap!("inter_use_global_motion", Some(off), "msg", "7.11");
            // gap!("commented_out", ...) must not count
            let s = "gap!(\"in_a_string\", ...)";
            let b = gap!(
                "intra_missing_edge",
                None,
                "msg",
                SPEC,
            );
            // A lifetime must not be mistaken for a char literal that swallows a marker.
            fn f<'a>(e: Foo<'_>) { general_intra_at!("intra_after_lifetime", off, "m", "s"); }
        "#;
        let scan = scan_gap_reasons(code);
        assert!(scan.non_literal_sites.is_empty());
        assert_eq!(
            scan.reasons,
            vec![
                "inter_use_global_motion",
                "intra_missing_edge",
                "intra_after_lifetime"
            ]
        );
    }

    #[test]
    fn reports_non_literal_reason_and_ignores_identifier_prefix() {
        let code = r#"
            let x = mygap!("not_a_marker", a, b, c);
            let y = gap!(REASON_CONST, a, b, c);
        "#;
        let scan = scan_gap_reasons(code);
        assert!(scan.reasons.is_empty());
        assert_eq!(scan.non_literal_sites.len(), 1);
        assert_eq!(scan.non_literal_sites[0].macro_name, "gap");
        assert_eq!(scan.non_literal_sites[0].line, 3);
    }

    #[test]
    fn non_literal_reason_fails_with_file_and_line() {
        let files = vec![(
            "a.rs".to_string(),
            "let x = 1;\nlet y = gap!(REASON_CONST, a, b, c);".to_string(),
        )];
        let err = evaluate(&files, 0).unwrap_err().to_string();
        assert!(err.contains("a.rs:2"), "{err}");
        assert!(
            err.contains("`gap!` reason must be a string literal"),
            "{err}"
        );
    }

    #[test]
    fn duplicate_reason_fails() {
        let files = vec![
            (
                "a.rs".to_string(),
                r#"gap!("dup", o, "m", "s");"#.to_string(),
            ),
            (
                "b.rs".to_string(),
                r#"gap!("dup", o, "m", "s");"#.to_string(),
            ),
        ];
        let err = evaluate(&files, 0).unwrap_err().to_string();
        assert!(err.contains("dup"), "{err}");
        assert!(err.contains("a.rs") && err.contains("b.rs"), "{err}");
    }

    #[test]
    fn count_below_floor_fails() {
        let files = vec![(
            "a.rs".to_string(),
            r#"gap!("only", o, "m", "s");"#.to_string(),
        )];
        assert!(evaluate(&files, 2).is_err());
        assert_eq!(evaluate(&files, 1).unwrap(), 1);
    }

    #[test]
    fn empty_corpus_passes_zero_floor() {
        assert_eq!(evaluate(&[], 0).unwrap(), 0);
    }

    #[test]
    fn scans_every_marker_family_reason() {
        let code = r#"
            let a = inter_cap!("inter_x", o, "cap", "s");
            let b = general_intra_at!("intra_y", o, "m", "s");
            let c = compound_missing!("compound_z", o, "in", "s");
            let d = inter_diag!("diag_w", o, "m", "s");
        "#;
        let scan = scan_gap_reasons(code);
        assert!(scan.non_literal_sites.is_empty());
        let mut reasons = scan.reasons;
        reasons.sort();
        assert_eq!(reasons, vec!["compound_z", "diag_w", "inter_x", "intra_y"]);
    }
}
