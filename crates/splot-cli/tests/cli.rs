// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! End-to-end CLI tests: run the built `splot` binary against the committed
//! fixtures in `tests/fixtures/` and assert on exit codes and output.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};
use std::process::Output;

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name)
}

fn splot(args: &[&str]) -> Output {
    std::process::Command::new(env!("CARGO_BIN_EXE_splot"))
        .args(args)
        .output()
        .expect("failed to run the splot binary")
}

fn validate(fixture_name: &str, extra: &[&str]) -> Output {
    let path = fixture(fixture_name);
    let path = path.to_str().expect("fixture path is valid UTF-8");
    let mut args = vec!["validate"];
    args.extend_from_slice(extra);
    args.push(path);
    splot(&args)
}

#[test]
fn validate_conformant_exits_zero() {
    let out = validate("conformant.av2", &[]);
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("conformant"), "stdout was: {stdout}");
}

#[test]
fn validate_global_xlayer_violation_exits_one() {
    let out = validate("bad-global-xlayer.av2", &[]);
    assert_eq!(out.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("obu-header/global-xlayer-required"),
        "stdout was: {stdout}"
    );
    assert!(stdout.contains("NOT conformant"), "stdout was: {stdout}");
}

#[test]
fn validate_truncated_stream_exits_one() {
    let out = validate("truncated.av2", &[]);
    assert_eq!(out.status.code(), Some(1));
}

#[test]
fn validate_json_emits_structured_diagnostic() {
    let out = validate("bad-global-xlayer.av2", &["--json"]);
    assert_eq!(out.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("\"rule_id\": \"obu-header/global-xlayer-required\""),
        "stdout was: {stdout}"
    );
}

#[test]
fn inspect_lists_obu_headers() {
    let path = fixture("conformant.av2");
    let out = splot(&["inspect", "--headers", path.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("OBU_TEMPORAL_DELIMITER"),
        "stdout was: {stdout}"
    );
    assert!(
        stdout.contains("OBU_SEQUENCE_HEADER"),
        "stdout was: {stdout}"
    );
}

#[test]
fn inspect_prints_valid_prefix_before_a_tail_error() {
    // A valid TemporalDelimiter followed by a truncated OBU: the prefix is shown,
    // and the tail parse error sets a non-zero exit.
    let path = fixture("prefix-then-truncated.av2");
    let out = splot(&["inspect", "--headers", path.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("OBU_TEMPORAL_DELIMITER"),
        "stdout was: {stdout}"
    );
}

#[test]
fn missing_input_file_exits_two() {
    let out = validate("does-not-exist.av2", &[]);
    assert_eq!(out.status.code(), Some(2));
}
