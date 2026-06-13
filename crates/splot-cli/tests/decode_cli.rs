// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! End-to-end `splot decode` CLI contract tests.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};
use std::process::Output;
use std::sync::atomic::{AtomicUsize, Ordering};

use splot_decode::unsupported_feature_diagnostic;

static TEMP_COUNTER: AtomicUsize = AtomicUsize::new(0);

fn splot(args: &[&str]) -> Output {
    std::process::Command::new(env!("CARGO_BIN_EXE_splot"))
        .args(args)
        .output()
        .expect("failed to run the splot binary")
}

fn splot_in(args: &[&str], cwd: &Path) -> Output {
    std::process::Command::new(env!("CARGO_BIN_EXE_splot"))
        .current_dir(cwd)
        .args(args)
        .output()
        .expect("failed to run the splot binary")
}

fn temp_path(stem: &str, extension: &str) -> PathBuf {
    let id = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system time is after Unix epoch")
        .as_nanos();
    std::env::temp_dir().join(format!(
        "splot-decode-cli-test-{stem}-{}-{nanos}-{id}.{extension}",
        std::process::id()
    ))
}

fn temp_input(extension: &str, data: &[u8]) -> PathBuf {
    let path = temp_path("input", extension);
    std::fs::write(&path, data).expect("write temporary input");
    path
}

fn temp_output(extension: &str) -> PathBuf {
    temp_path("output", extension)
}

fn temp_dir(stem: &str) -> PathBuf {
    let path = temp_path(stem, "dir");
    std::fs::create_dir(&path).expect("create temporary directory");
    path
}

fn read_dir_paths(path: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(path)
        .expect("read temporary directory")
        .map(|entry| entry.expect("read temporary directory entry").path())
        .collect()
}

#[test]
fn decode_unsupported_text_mode_emits_stable_diagnostic() {
    let input = temp_input("av2", b"input must not be read");
    let output = temp_output("y4m");
    let original_output = b"existing output must remain untouched";
    std::fs::write(&output, original_output).expect("write temporary output sentinel");

    let out = splot(&[
        "decode",
        input.to_str().unwrap(),
        "-o",
        output.to_str().unwrap(),
    ]);

    let diagnostic = unsupported_feature_diagnostic();
    assert_eq!(out.status.code(), Some(1));
    assert!(out.stdout.is_empty(), "stdout was not empty");
    let stderr = String::from_utf8_lossy(&out.stderr);
    for expected in [
        format!("rule_id: {}", diagnostic.rule_id),
        format!("severity: {}", diagnostic.severity),
        format!(
            "spec_section: {}",
            diagnostic
                .spec_section
                .expect("diagnostic cites a spec section")
        ),
        format!("matrix_row: {}", diagnostic.matrix_row),
        format!("feature_id: {}", diagnostic.feature_id),
        "remediation:".to_string(),
    ] {
        assert!(
            stderr.contains(&expected),
            "stderr did not contain {expected:?}: {stderr}"
        );
    }
    assert_eq!(
        std::fs::read(&output).expect("read temporary output sentinel"),
        original_output
    );
}

#[test]
fn decode_unsupported_json_mode_emits_diagnostic_object() {
    let input = temp_input("av2", b"input must not be read");
    let output = temp_output("y4m");
    let original_output = b"json mode output sentinel";
    std::fs::write(&output, original_output).expect("write temporary output sentinel");

    let out = splot(&[
        "decode",
        "--json",
        input.to_str().unwrap(),
        "-o",
        output.to_str().unwrap(),
    ]);

    assert_eq!(out.status.code(), Some(1));
    assert!(out.stderr.is_empty(), "stderr was not empty");
    let diagnostic = unsupported_feature_diagnostic();
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(json["rule_id"], diagnostic.rule_id);
    assert_eq!(json["severity"], diagnostic.severity.as_str());
    assert_eq!(
        json["spec_section"],
        diagnostic
            .spec_section
            .expect("diagnostic cites a spec section")
    );
    assert_eq!(json["matrix_row"], diagnostic.matrix_row);
    assert_eq!(json["feature_id"], diagnostic.feature_id);
    assert_eq!(json["message"], diagnostic.message);
    assert_eq!(json["remediation"], diagnostic.remediation);
    assert_eq!(
        std::fs::read(&output).expect("read temporary output sentinel"),
        original_output
    );
}

#[test]
fn decode_unsupported_missing_input_does_not_touch_files() {
    let input = temp_path("missing-input", "av2");
    let output = temp_output("y4m");
    assert!(!input.exists(), "temporary input unexpectedly exists");
    assert!(!output.exists(), "temporary output unexpectedly exists");

    let out = splot(&[
        "decode",
        input.to_str().unwrap(),
        "-o",
        output.to_str().unwrap(),
    ]);

    assert_eq!(out.status.code(), Some(1));
    assert!(out.stdout.is_empty(), "stdout was not empty");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("decode/unsupported-feature"),
        "stderr was: {stderr}"
    );
    assert!(!input.exists(), "decode created the missing input path");
    assert!(!output.exists(), "decode created the output path");
}

#[test]
fn decode_hash_output_format_emits_unsupported_text_without_output_path() {
    let input = temp_input("av2", b"input must not be read");

    let out = splot(&["decode", "--output-format", "hash", input.to_str().unwrap()]);

    assert_eq!(out.status.code(), Some(1));
    assert!(out.stdout.is_empty(), "stdout was not empty");
    let stderr = String::from_utf8_lossy(&out.stderr);
    let diagnostic = unsupported_feature_diagnostic();
    for expected in [
        format!("rule_id: {}", diagnostic.rule_id),
        format!("severity: {}", diagnostic.severity),
        format!(
            "spec_section: {}",
            diagnostic
                .spec_section
                .expect("diagnostic cites a spec section")
        ),
        format!("matrix_row: {}", diagnostic.matrix_row),
        format!("feature_id: {}", diagnostic.feature_id),
    ] {
        assert!(
            stderr.contains(&expected),
            "stderr did not contain {expected:?}: {stderr}"
        );
    }
}

#[test]
fn decode_hash_output_format_missing_input_does_not_touch_files() {
    let input = temp_path("missing-input", "av2");
    let cwd = temp_dir("hash-cwd");
    assert!(!input.exists(), "temporary input unexpectedly exists");
    assert!(
        read_dir_paths(&cwd).is_empty(),
        "temporary cwd unexpectedly contains files"
    );

    let out = splot_in(
        &["decode", "--output-format", "hash", input.to_str().unwrap()],
        &cwd,
    );

    assert_eq!(out.status.code(), Some(1));
    assert!(out.stdout.is_empty(), "stdout was not empty");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("decode/unsupported-feature"),
        "stderr was: {stderr}"
    );
    assert!(!input.exists(), "decode created the missing input path");
    assert_eq!(
        read_dir_paths(&cwd),
        Vec::<PathBuf>::new(),
        "decode created an implicit output in the temporary cwd"
    );
}

#[test]
fn decode_hash_output_format_json_emits_same_diagnostic() {
    let input = temp_input("av2", b"input must not be read");

    let out = splot(&[
        "decode",
        "--json",
        "--output-format",
        "hash",
        input.to_str().unwrap(),
    ]);

    assert_eq!(out.status.code(), Some(1));
    assert!(out.stderr.is_empty(), "stderr was not empty");
    let diagnostic = unsupported_feature_diagnostic();
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(json["rule_id"], diagnostic.rule_id);
    assert_eq!(json["severity"], diagnostic.severity.as_str());
    assert_eq!(
        json["spec_section"],
        diagnostic
            .spec_section
            .expect("diagnostic cites a spec section")
    );
    assert_eq!(json["matrix_row"], diagnostic.matrix_row);
    assert_eq!(json["feature_id"], diagnostic.feature_id);
}

#[test]
fn decode_invalid_output_format_is_usage_error() {
    let input = temp_input("av2", b"input must not be read");

    let out = splot(&["decode", "--output-format", "raw", input.to_str().unwrap()]);

    assert_eq!(out.status.code(), Some(2));
    assert!(out.stdout.is_empty(), "stdout was not empty");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !stderr.contains("decode/unsupported-feature"),
        "stderr was: {stderr}"
    );
}

#[test]
fn decode_hash_output_format_with_output_path_does_not_touch_file() {
    let input = temp_input("av2", b"input must not be read");
    let output = temp_output("hashes");
    let original_output = b"hash output sentinel";
    std::fs::write(&output, original_output).expect("write temporary output sentinel");

    let out = splot(&[
        "decode",
        "--output-format",
        "hash",
        input.to_str().unwrap(),
        "-o",
        output.to_str().unwrap(),
    ]);

    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("decode/unsupported-feature"),
        "stderr was: {stderr}"
    );
    assert_eq!(
        std::fs::read(&output).expect("read temporary output sentinel"),
        original_output
    );
}

#[test]
fn decode_without_output_selection_is_usage_error() {
    let input = temp_input("av2", b"input must not be read");

    let out = splot(&["decode", input.to_str().unwrap()]);

    assert_eq!(out.status.code(), Some(2));
    assert!(out.stdout.is_empty(), "stdout was not empty");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !stderr.contains("decode/unsupported-feature"),
        "stderr was: {stderr}"
    );
}

#[test]
fn decode_explicit_y4m_output_format_requires_output_path() {
    let input = temp_input("av2", b"input must not be read");

    let out = splot(&["decode", "--output-format", "y4m", input.to_str().unwrap()]);

    assert_eq!(out.status.code(), Some(2));
    assert!(out.stdout.is_empty(), "stdout was not empty");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !stderr.contains("decode/unsupported-feature"),
        "stderr was: {stderr}"
    );
}

#[test]
fn decode_explicit_y4m_output_format_matches_implicit_no_touch_behavior() {
    let input = temp_input("av2", b"input must not be read");
    let output = temp_output("y4m");
    let original_output = b"explicit y4m output sentinel";
    std::fs::write(&output, original_output).expect("write temporary output sentinel");

    let out = splot(&[
        "decode",
        "--output-format",
        "y4m",
        input.to_str().unwrap(),
        "-o",
        output.to_str().unwrap(),
    ]);

    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("decode/unsupported-feature"),
        "stderr was: {stderr}"
    );
    assert_eq!(
        std::fs::read(&output).expect("read temporary output sentinel"),
        original_output
    );
}
