// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! End-to-end `splot decode` Y4M output tests.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};
use std::process::Output;
use std::sync::atomic::{AtomicUsize, Ordering};

static TEMP_COUNTER: AtomicUsize = AtomicUsize::new(0);

const PLANABLE_CLOSED_LOOP_KEY: &[u8] = &[0x01, 0x10];

fn splot(args: &[&str]) -> Output {
    std::process::Command::new(env!("CARGO_BIN_EXE_splot"))
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
        "splot-decode-y4m-cli-test-{stem}-{}-{nanos}-{id}.{extension}",
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

fn conformance_vector(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/conformance/vectors/valid")
        .join(name)
}

fn read_dir_names(path: &Path) -> Vec<String> {
    let mut entries = std::fs::read_dir(path)
        .expect("read temporary directory")
        .map(|entry| {
            entry
                .expect("read temporary directory entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect::<Vec<_>>();
    entries.sort();
    entries
}

fn expected_minimal_y4m() -> Vec<u8> {
    let mut bytes = b"YUV4MPEG2 W64 H64 F30:1 Ip A0:0 C420\nFRAME\n".to_vec();
    bytes.extend(core::iter::repeat_n(128, 64 * 64));
    bytes.extend(core::iter::repeat_n(129, 32 * 32 + 32 * 32));
    bytes
}

#[test]
fn decode_out_of_tier_y4m_text_mode_emits_unsupported_feature_without_touching_output() {
    let input = temp_input("av2", PLANABLE_CLOSED_LOOP_KEY);
    let output = temp_output("y4m");
    let original_output = b"existing output must remain untouched";
    std::fs::write(&output, original_output).expect("write temporary output sentinel");

    let out = splot(&[
        "decode",
        input.to_str().unwrap(),
        "-o",
        output.to_str().unwrap(),
    ]);

    assert_eq!(out.status.code(), Some(1));
    assert!(out.stdout.is_empty(), "stdout was not empty");
    let stderr = String::from_utf8_lossy(&out.stderr);
    for expected in [
        "rule_id: decode/unsupported-feature",
        "severity: Error",
        "spec_section: 7.1",
        "matrix_row: minimal-decode-tier-contract",
        "feature_id: DECODE-MINIMAL-TIER-RUNTIME-SUCCESS",
        "detail_kind: unsupported_feature",
        "unsupported_reason: unexpected_planned_stream_shape",
        "tier_id: minimal-intra-8bit420-hash-v1",
        "output_format: y4m",
        "remediation:",
    ] {
        assert!(
            stderr.contains(expected),
            "stderr did not contain {expected:?}: {stderr}"
        );
    }
    assert_eq!(
        std::fs::read(&output).expect("read temporary output sentinel"),
        original_output
    );
}

#[test]
fn decode_out_of_tier_y4m_json_mode_emits_unsupported_feature_object() {
    let input = temp_input("av2", PLANABLE_CLOSED_LOOP_KEY);
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
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(json["rule_id"], "decode/unsupported-feature");
    assert_eq!(json["severity"], "Error");
    assert_eq!(json["spec_section"], "7.1");
    assert_eq!(json["matrix_row"], "minimal-decode-tier-contract");
    assert_eq!(json["feature_id"], "DECODE-MINIMAL-TIER-RUNTIME-SUCCESS");
    assert_eq!(json["detail_kind"], "unsupported_feature");
    assert_eq!(
        json["unsupported_reason"],
        "unexpected_planned_stream_shape"
    );
    assert_eq!(json["tier_id"], "minimal-intra-8bit420-hash-v1");
    assert_eq!(json["output_format"], "y4m");
    assert_eq!(
        std::fs::read(&output).expect("read temporary output sentinel"),
        original_output
    );
}

#[test]
fn decode_y4m_source_error_wins_before_missing_output_parent() {
    let input = temp_input("av2", PLANABLE_CLOSED_LOOP_KEY);
    let root = temp_dir("source-error-before-output-root");
    let missing_parent = root.join("missing");
    let output = missing_parent.join("out.y4m");

    let out = splot(&[
        "decode",
        "--json",
        "--output-format",
        "y4m",
        input.to_str().unwrap(),
        "-o",
        output.to_str().unwrap(),
    ]);

    assert_eq!(out.status.code(), Some(1));
    assert!(out.stderr.is_empty(), "stderr was not empty");
    assert!(
        !missing_parent.exists(),
        "decode created the missing parent"
    );
    assert!(!output.exists(), "decode created the requested output");
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(json["rule_id"], "decode/unsupported-feature");
    assert_eq!(json["severity"], "Error");
    assert_eq!(json["spec_section"], "7.1");
    assert_eq!(json["matrix_row"], "minimal-decode-tier-contract");
    assert_eq!(json["feature_id"], "DECODE-MINIMAL-TIER-RUNTIME-SUCCESS");
    assert_eq!(json["detail_kind"], "unsupported_feature");
    assert_eq!(
        json["unsupported_reason"],
        "unexpected_planned_stream_shape"
    );
    assert_eq!(json["output_format"], "y4m");
}

#[test]
fn decode_explicit_y4m_output_format_requires_output_path() {
    let input = temp_input("av2", PLANABLE_CLOSED_LOOP_KEY);

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
    let input = temp_input("av2", PLANABLE_CLOSED_LOOP_KEY);
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

#[test]
fn decode_explicit_y4m_success_for_minimal_fixture() {
    let input = conformance_vector("syn-flat-intra-64x64-minimal.ivf");
    let output = temp_output("y4m");

    let out = splot(&[
        "decode",
        "--output-format",
        "y4m",
        input.to_str().unwrap(),
        "-o",
        output.to_str().unwrap(),
    ]);

    assert_eq!(out.status.code(), Some(0));
    assert!(out.stdout.is_empty(), "stdout was not empty");
    assert!(out.stderr.is_empty(), "stderr was not empty");
    assert_eq!(std::fs::read(&output).unwrap(), expected_minimal_y4m());
}

#[test]
fn decode_implicit_y4m_success_matches_explicit_output() {
    let input = conformance_vector("syn-flat-intra-64x64-minimal.ivf");
    let implicit = temp_output("y4m");
    let explicit = temp_output("y4m");

    let implicit_out = splot(&[
        "decode",
        input.to_str().unwrap(),
        "-o",
        implicit.to_str().unwrap(),
    ]);
    let explicit_out = splot(&[
        "decode",
        "--output-format",
        "y4m",
        input.to_str().unwrap(),
        "-o",
        explicit.to_str().unwrap(),
    ]);

    assert_eq!(implicit_out.status.code(), Some(0));
    assert_eq!(explicit_out.status.code(), Some(0));
    assert!(
        implicit_out.stdout.is_empty(),
        "implicit stdout was not empty"
    );
    assert!(
        implicit_out.stderr.is_empty(),
        "implicit stderr was not empty"
    );
    assert!(
        explicit_out.stdout.is_empty(),
        "explicit stdout was not empty"
    );
    assert!(
        explicit_out.stderr.is_empty(),
        "explicit stderr was not empty"
    );
    assert_eq!(std::fs::read(&implicit).unwrap(), expected_minimal_y4m());
    assert_eq!(
        std::fs::read(&implicit).unwrap(),
        std::fs::read(&explicit).unwrap()
    );
}

#[test]
fn decode_y4m_success_replaces_existing_output_and_cleans_temp_file() {
    let input = conformance_vector("syn-flat-intra-64x64-minimal.ivf");
    let dir = temp_dir("minimal-y4m-output");
    let output = dir.join("out.y4m");
    std::fs::write(&output, b"old y4m bytes").expect("write temporary output sentinel");
    let before_entries = read_dir_names(&dir);

    let out = splot(&[
        "decode",
        "--output-format",
        "y4m",
        input.to_str().unwrap(),
        "-o",
        output.to_str().unwrap(),
    ]);

    assert_eq!(out.status.code(), Some(0));
    assert!(out.stdout.is_empty(), "stdout was not empty");
    assert!(out.stderr.is_empty(), "stderr was not empty");
    assert_eq!(std::fs::read(&output).unwrap(), expected_minimal_y4m());
    assert_eq!(read_dir_names(&dir), before_entries);
}

#[test]
fn decode_y4m_skips_temp_name_that_matches_requested_output() {
    let input = conformance_vector("syn-flat-intra-64x64-minimal.ivf");
    let dir = temp_dir("temp-name-collision");
    let selected_name_path = dir.join("selected-output-name.txt");
    let script = r#"out=".splot-decode-y4m-$$-0-0.tmp"
printf "%s" "$out" > "$SPLOT_SELECTED"
exec "$SPLOT_BIN" decode --output-format y4m "$SPLOT_INPUT" -o "$out"
"#;

    let out = std::process::Command::new("sh")
        .arg("-c")
        .arg(script)
        .current_dir(&dir)
        .env("SPLOT_BIN", env!("CARGO_BIN_EXE_splot"))
        .env("SPLOT_INPUT", input)
        .env("SPLOT_SELECTED", &selected_name_path)
        .output()
        .expect("failed to run shell-wrapped splot command");

    assert_eq!(out.status.code(), Some(0));
    assert!(out.stdout.is_empty(), "stdout was not empty");
    assert!(
        out.stderr.is_empty(),
        "stderr was: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let selected_name = std::fs::read_to_string(&selected_name_path).unwrap();
    let output = dir.join(selected_name);
    assert_eq!(std::fs::read(&output).unwrap(), expected_minimal_y4m());
    let names = read_dir_names(&dir);
    assert!(names.contains(&"selected-output-name.txt".to_string()));
    assert_eq!(
        names.iter().filter(|name| name.ends_with(".tmp")).count(),
        1
    );
}

#[test]
fn decode_y4m_outputs_are_thread_deterministic() {
    let input = conformance_vector("syn-flat-intra-64x64-minimal.ivf");
    let decode = |threads| {
        let output = temp_output("y4m");
        let out = splot(&[
            "decode",
            "--output-format",
            "y4m",
            "--threads",
            threads,
            input.to_str().unwrap(),
            "-o",
            output.to_str().unwrap(),
        ]);
        assert_eq!(out.status.code(), Some(0), "threads={threads}");
        assert!(out.stdout.is_empty(), "stdout was not empty for {threads}");
        assert!(out.stderr.is_empty(), "stderr was not empty for {threads}");
        std::fs::read(&output).unwrap()
    };
    let expected = expected_minimal_y4m();

    assert_eq!(decode("1"), expected);
    assert_eq!(decode("auto"), expected);
    assert_eq!(decode("2"), expected);
}

#[test]
fn decode_y4m_missing_output_parent_emits_output_error_json() {
    let input = conformance_vector("syn-flat-intra-64x64-minimal.ivf");
    let dir = temp_dir("missing-output-parent-root");
    let missing_parent = dir.join("missing");
    let output = missing_parent.join("out.y4m");

    let out = splot(&[
        "decode",
        "--json",
        "--output-format",
        "y4m",
        input.to_str().unwrap(),
        "-o",
        output.to_str().unwrap(),
    ]);

    assert_eq!(out.status.code(), Some(1));
    assert!(out.stderr.is_empty(), "stderr was not empty");
    assert!(
        !missing_parent.exists(),
        "decode created the missing parent"
    );
    assert!(!output.exists(), "decode created the requested output");
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(json["rule_id"], "decode/output-error");
    assert_eq!(json["severity"], "Error");
    assert_eq!(json["spec_section"], "");
    assert_eq!(json["matrix_row"], "decode-y4m-runtime-output");
    assert_eq!(json["feature_id"], "DECODE-Y4M-RUNTIME-OUTPUT");
    assert_eq!(json["detail_kind"], "output_error");
    assert_eq!(json["output_format"], "y4m");
    assert_eq!(json["output_operation"], "create_y4m_temp_file");
    assert_eq!(json["output_source_kind"], "io");
    assert!(
        !json["output_source_message"]
            .as_str()
            .unwrap()
            .contains(".splot-decode-y4m-"),
        "diagnostic leaked temp path suffix: {json}"
    );
}

#[test]
fn decode_y4m_directory_output_path_emits_output_error_text_and_cleans_temp_file() {
    let input = conformance_vector("syn-flat-intra-64x64-minimal.ivf");
    let parent = temp_dir("directory-output-parent");
    let output = parent.join("already-dir.y4m");
    std::fs::create_dir(&output).expect("create directory at output path");
    let before_entries = read_dir_names(&parent);

    let out = splot(&[
        "decode",
        "--output-format",
        "y4m",
        input.to_str().unwrap(),
        "-o",
        output.to_str().unwrap(),
    ]);

    assert_eq!(out.status.code(), Some(1));
    assert!(out.stdout.is_empty(), "stdout was not empty");
    let stderr = String::from_utf8_lossy(&out.stderr);
    for expected in [
        "rule_id: decode/output-error",
        "severity: Error",
        "spec_section: ",
        "matrix_row: decode-y4m-runtime-output",
        "feature_id: DECODE-Y4M-RUNTIME-OUTPUT",
        "detail_kind: output_error",
        "output_operation: rename_y4m_output",
        "output_source_kind: io",
        "output_format: y4m",
    ] {
        assert!(
            stderr.contains(expected),
            "stderr did not contain {expected:?}: {stderr}"
        );
    }
    assert!(
        !stderr.contains(".splot-decode-y4m-"),
        "diagnostic leaked temp path suffix: {stderr}"
    );
    assert!(output.is_dir(), "directory output path was replaced");
    assert_eq!(read_dir_names(&parent), before_entries);
}
