// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Golden snapshots of the `splot validate` output controls (CLI-VALIDATE-OUTPUT-CONTROLS).
//!
//! Freezes the text and JSON shape of `--max-diagnostics` (capped list + a
//! truncation notice / `truncation` object) and `--summary-only` (counts +
//! conformance line / a `summary` object with an empty `diagnostics` array). The
//! output is deterministic for a committed fixture (byte offsets, messages, counts;
//! no paths/timestamps), and these flags are presentation-only — the exit code is
//! asserted to stay identical to the uncapped run.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::Path;
use std::process::Command;

/// Runs `splot validate <extra...> <fixture>` and returns `(exit_code, stdout)`.
fn validate(fixture: &str, extra: &[&str]) -> (Option<i32>, String) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(fixture);
    let mut args = vec!["validate"];
    args.extend_from_slice(extra);
    let path = path.to_str().expect("fixture path is UTF-8");
    args.push(path);
    let out = Command::new(env!("CARGO_BIN_EXE_splot"))
        .args(&args)
        .output()
        .expect("failed to run the splot binary");
    (
        out.status.code(),
        String::from_utf8(out.stdout).expect("validate stdout is valid UTF-8"),
    )
}

#[test]
fn validate_max_diagnostics_text() {
    let (code, stdout) = validate("bad-global-xlayer.av2", &["--max-diagnostics", "1"]);
    assert_eq!(code, Some(1));
    insta::assert_snapshot!("validate_max_diagnostics_text", stdout);
}

#[test]
fn validate_max_diagnostics_json() {
    let (code, stdout) = validate(
        "bad-global-xlayer.av2",
        &["--max-diagnostics", "1", "--json"],
    );
    assert_eq!(code, Some(1));
    insta::assert_snapshot!("validate_max_diagnostics_json", stdout);
}

#[test]
fn validate_summary_only_text() {
    let (code, stdout) = validate("bad-global-xlayer.av2", &["--summary-only"]);
    assert_eq!(code, Some(1));
    insta::assert_snapshot!("validate_summary_only_text", stdout);
}

#[test]
fn validate_summary_only_json() {
    let (code, stdout) = validate("bad-global-xlayer.av2", &["--summary-only", "--json"]);
    assert_eq!(code, Some(1));
    insta::assert_snapshot!("validate_summary_only_json", stdout);
}

#[test]
fn validate_summary_only_clean_text() {
    let (code, stdout) = validate("operating-point-set.av2", &["--summary-only"]);
    assert_eq!(code, Some(0));
    insta::assert_snapshot!("validate_summary_only_clean_text", stdout);
}
