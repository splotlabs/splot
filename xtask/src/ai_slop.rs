// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AI-slop comment gate (`cargo xtask check-ai-slop`).
//!
//! Hard-fails when a tracked, non-generated Rust source comment carries a banned
//! process-history or development-diary phrase. The phrase set is deliberately
//! conservative: established domain vocabulary is left alone (the decoder's
//! partition `frontier`, AV2 "previously decoded" spec language, capability
//! phrasings, control-flow deferral notes). Adding a phrase means cleaning every
//! existing hit in the same change, because this gate enforces zero.

use std::path::Path;

use anyhow::{Context as _, Result, bail};

use crate::comment_density::{
    CommentKind, is_generated_source, rust_source_files, scan_comment_lines,
};
use crate::feature_status::display_path;

/// Banned phrases matched between word boundaries (a leading AND trailing
/// boundary), so suffixing one into a different word does not trip the gate
/// (`old behavioral`, `this refactored`). Lowercase ASCII.
const BANNED_WORD_PHRASES: &[&str] = &[
    "formerly",
    "this refactor",
    "this used to",
    "used to be",
    "old behavior",
    "old behaviour",
    "not yet fixtured",
];

/// Banned phrases matched at a leading word boundary only (no trailing
/// boundary), so plurals and inflections are still caught. Lowercase ASCII.
const BANNED_PREFIX_PHRASES: &[&str] = &["oracle fixture", "verified fixture", "pinned by fixture"];

/// Verifies no banned slop phrase appears in a tracked-source comment.
///
/// # Errors
///
/// Returns an error listing every offending `file:line` when a banned phrase is
/// found, or when a tracked source file cannot be read.
pub(crate) fn check_ai_slop(root: &Path) -> Result<()> {
    let files = rust_source_files(root)?;
    let mut scanned = 0usize;
    let mut violations = Vec::new();

    for path in files {
        let displayed = display_path(root, &path);
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("failed to read {displayed}"))?;
        if is_generated_source(&text) {
            continue;
        }
        scanned += 1;
        scan_comment_lines(&text, |kind, line_no, line| {
            if matches!(kind, CommentKind::Spdx) {
                return;
            }
            if let Some(marker) = banned_marker(line) {
                violations.push(format!(
                    "{displayed}:{line_no}: banned comment phrase `{marker}`"
                ));
            }
        });
    }

    if violations.is_empty() {
        eprintln!("check-ai-slop: ok (0 banned comment phrases in {scanned} tracked Rust files)");
        Ok(())
    } else {
        let count = violations.len();
        bail!(
            "check-ai-slop: {count} banned comment phrase(s); replace history/diary prose with a current invariant, a capability, or an `AV2 §` anchor:\n{}",
            violations.join("\n")
        )
    }
}

/// Returns a label for the first banned phrase in `comment`, or `None`.
fn banned_marker(comment: &str) -> Option<&'static str> {
    let lower = comment.to_ascii_lowercase();
    for phrase in BANNED_WORD_PHRASES {
        if contains_at_word_boundary(&lower, phrase, is_word_boundary_end) {
            return Some(phrase);
        }
    }
    for phrase in BANNED_PREFIX_PHRASES {
        if contains_at_word_boundary(&lower, phrase, |_| true) {
            return Some(phrase);
        }
    }
    if contains_at_word_boundary(&lower, "round-", starts_with_ascii_digit) {
        return Some("round-<n> (development round)");
    }
    if contains_at_word_boundary(&lower, "pr", pr_number_tail) {
        return Some("PR #<n> (pull-request reference)");
    }
    None
}

/// Whether `tail` does not begin with a word character, so a phrase match ends
/// at a word boundary rather than inside a longer word.
fn is_word_boundary_end(tail: &str) -> bool {
    tail.chars()
        .next()
        .is_none_or(|c| !(c.is_alphanumeric() || c == '_'))
}

/// Tests whether `needle` occurs in `haystack_lower` preceded by a word boundary
/// and followed by a `tail_ok` suffix. `needle` must already be lowercase.
fn contains_at_word_boundary(
    haystack_lower: &str,
    needle: &str,
    tail_ok: impl Fn(&str) -> bool,
) -> bool {
    haystack_lower.match_indices(needle).any(|(at, matched)| {
        let preceded_by_word = haystack_lower[..at]
            .chars()
            .next_back()
            .is_some_and(|c| c.is_alphanumeric() || c == '_');
        !preceded_by_word && tail_ok(&haystack_lower[at + matched.len()..])
    })
}

/// Whether `tail` begins with an ASCII digit.
fn starts_with_ascii_digit(tail: &str) -> bool {
    tail.as_bytes().first().is_some_and(u8::is_ascii_digit)
}

/// Whether `tail` is a ` #<digits>` pull-request suffix.
fn pr_number_tail(tail: &str) -> bool {
    let tail = tail.trim_start_matches(' ');
    tail.strip_prefix('#')
        .is_some_and(|rest| starts_with_ascii_digit(rest.trim_start_matches(' ')))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn markers(text: &str) -> Vec<&'static str> {
        let mut found = Vec::new();
        scan_comment_lines(text, |kind, _line_no, line| {
            if !matches!(kind, CommentKind::Spdx)
                && let Some(marker) = banned_marker(line)
            {
                found.push(marker);
            }
        });
        found
    }

    #[test]
    fn flags_process_history_phrases() {
        assert_eq!(markers("// formerly returned None\n"), vec!["formerly"]);
        assert_eq!(
            markers("/// see round-7 F2 regression\n"),
            vec!["round-<n> (development round)"]
        );
        assert_eq!(
            markers("// tracked in PR #1234\n"),
            vec!["PR #<n> (pull-request reference)"]
        );
        assert_eq!(
            markers("/// this refactor preserves bytes\n"),
            vec!["this refactor"]
        );
    }

    #[test]
    fn flags_fixture_diary_phrases() {
        assert_eq!(
            markers("/// admits exactly the verified fixture\n"),
            vec!["verified fixture"]
        );
        assert_eq!(
            markers("/// lacks oracle fixtures\n"),
            vec!["oracle fixture"]
        );
    }

    #[test]
    fn keeps_domain_and_spec_vocabulary() {
        assert!(markers("/// the previously decoded U-plane EOB\n").is_empty());
        assert!(markers("/// positioned at the partition frontier\n").is_empty());
        assert!(markers("/// the subset only supports DC-only blocks\n").is_empty());
        assert!(markers("/// deferred until the borrows release\n").is_empty());
        assert!(markers("/// a round-trip safety check\n").is_empty());
    }

    #[test]
    fn respects_word_boundaries() {
        assert!(markers("/// caused to be slow\n").is_empty());
        assert_eq!(markers("/// this used to work\n"), vec!["this used to"]);
        assert!(markers("/// the expr #1 operand\n").is_empty());
    }

    #[test]
    fn ignores_string_and_code_lines() {
        assert!(markers("let sample = \"formerly banned\";\n").is_empty());
        assert!(markers("const P: &str = \"oracle fixture\";\n").is_empty());
    }

    #[test]
    fn flags_trailing_inline_comments() {
        assert_eq!(
            markers("let x = compute(); // formerly returned None\n"),
            vec!["formerly"]
        );
        assert_eq!(
            markers("fn f() {} // this refactor split it\n"),
            vec!["this refactor"]
        );
    }

    #[test]
    fn ignores_banned_phrase_inside_a_string_with_trailing_slashes() {
        assert!(markers("let u = \"http://oracle fixture\";\n").is_empty());
    }

    #[test]
    fn banned_phrases_are_lowercase_ascii() {
        for phrase in BANNED_WORD_PHRASES.iter().chain(BANNED_PREFIX_PHRASES) {
            assert!(phrase.is_ascii(), "{phrase} must be ASCII");
            assert_eq!(
                *phrase,
                phrase.to_ascii_lowercase(),
                "{phrase} must be lowercase"
            );
        }
    }

    #[test]
    fn word_phrases_require_a_trailing_boundary() {
        assert!(markers("/// the old behavioral model is fine\n").is_empty());
        assert!(markers("/// this refactored cleanly\n").is_empty());
        assert_eq!(
            markers("/// the old behavior was wrong\n"),
            vec!["old behavior"]
        );
    }

    #[test]
    fn prefix_phrases_still_catch_plurals() {
        assert_eq!(
            markers("/// lacks oracle fixtures\n"),
            vec!["oracle fixture"]
        );
    }

    #[test]
    fn flags_trailing_block_comments() {
        assert_eq!(markers("let x = 0; /* formerly 1 */\n"), vec!["formerly"]);
    }

    #[test]
    fn flags_multiline_block_comment_interior() {
        assert_eq!(
            markers("/* note:\nthis used to panic\n*/\n"),
            vec!["this used to"]
        );
    }
}
