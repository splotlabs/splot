// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! End-to-end `splot decode` raw output tests.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};
use std::process::Output;
use std::sync::atomic::{AtomicUsize, Ordering};

mod common;
use common::read_dir_names;

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
        "splot-decode-raw-cli-test-{stem}-{}-{nanos}-{id}.{extension}",
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

// The decoded raw planar output for the committed conformant luma-skip fixture
// (luma is an all-zero/skip DC block at flat 128; chroma carries a real coded
// residual). avmdec and dav2d both decode the fixture to these exact bytes (see
// docs/LOCAL-REFERENCE-EVIDENCE.toml); the reference is committed alongside it.
fn expected_minimal_raw() -> Vec<u8> {
    include_bytes!("../../../tests/conformance/vectors/valid/syn-flat-intra-64x64-minimal.raw")
        .to_vec()
}

#[test]
fn decode_explicit_raw_output_format_requires_output_path() {
    let input = temp_input("av2", PLANABLE_CLOSED_LOOP_KEY);

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
fn decode_explicit_raw_success_for_minimal_fixture() {
    let input = conformance_vector("syn-flat-intra-64x64-minimal.ivf");
    let output = temp_output("raw");

    let out = splot(&[
        "decode",
        "--output-format",
        "raw",
        input.to_str().unwrap(),
        "-o",
        output.to_str().unwrap(),
    ]);

    assert_eq!(out.status.code(), Some(0));
    assert!(out.stdout.is_empty(), "stdout was not empty");
    assert!(out.stderr.is_empty(), "stderr was not empty");
    assert_eq!(std::fs::read(&output).unwrap(), expected_minimal_raw());
}

#[test]
fn decode_raw_success_replaces_existing_output_and_cleans_temp_file() {
    let input = conformance_vector("syn-flat-intra-64x64-minimal.ivf");
    let dir = temp_dir("minimal-raw-output");
    let output = dir.join("out.raw");
    std::fs::write(&output, b"old raw bytes").expect("write temporary output sentinel");
    let before_entries = read_dir_names(&dir);

    let out = splot(&[
        "decode",
        "--output-format",
        "raw",
        input.to_str().unwrap(),
        "-o",
        output.to_str().unwrap(),
    ]);

    assert_eq!(out.status.code(), Some(0));
    assert!(out.stdout.is_empty(), "stdout was not empty");
    assert!(out.stderr.is_empty(), "stderr was not empty");
    assert_eq!(std::fs::read(&output).unwrap(), expected_minimal_raw());
    assert_eq!(read_dir_names(&dir), before_entries);
}

#[test]
fn decode_out_of_tier_raw_text_mode_emits_unsupported_feature_without_touching_output() {
    let input = temp_input("av2", PLANABLE_CLOSED_LOOP_KEY);
    let output = temp_output("raw");
    let original_output = b"existing raw output sentinel";
    std::fs::write(&output, original_output).expect("write temporary output sentinel");

    let out = splot(&[
        "decode",
        "--output-format",
        "raw",
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
        "output_format: raw",
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
fn decode_raw_source_error_wins_before_missing_output_parent() {
    let input = temp_input("av2", PLANABLE_CLOSED_LOOP_KEY);
    let root = temp_dir("source-error-before-output-root");
    let missing_parent = root.join("missing");
    let output = missing_parent.join("out.raw");

    let out = splot(&[
        "decode",
        "--json",
        "--output-format",
        "raw",
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
    assert_eq!(json["output_format"], "raw");
}

#[test]
fn decode_raw_outputs_are_thread_deterministic() {
    let input = conformance_vector("syn-flat-intra-64x64-minimal.ivf");
    let decode = |threads| {
        let output = temp_output("raw");
        let out = splot(&[
            "decode",
            "--output-format",
            "raw",
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
    let expected = expected_minimal_raw();

    assert_eq!(decode("1"), expected);
    assert_eq!(decode("auto"), expected);
    assert_eq!(decode("2"), expected);
}

#[test]
fn decode_raw_missing_output_parent_emits_output_error_json() {
    let input = conformance_vector("syn-flat-intra-64x64-minimal.ivf");
    let dir = temp_dir("missing-output-parent-root");
    let missing_parent = dir.join("missing");
    let output = missing_parent.join("out.raw");

    let out = splot(&[
        "decode",
        "--json",
        "--output-format",
        "raw",
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
    assert_eq!(json["matrix_row"], "decode-minimal-raw-runtime-output");
    assert_eq!(json["feature_id"], "DECODE-MINIMAL-RAW-RUNTIME-OUTPUT");
    assert_eq!(json["detail_kind"], "output_error");
    assert_eq!(json["output_format"], "raw");
    assert_eq!(json["output_operation"], "create_raw_temp_file");
    assert_eq!(json["output_source_kind"], "io");
    assert!(
        !json["output_source_message"]
            .as_str()
            .unwrap()
            .contains(".splot-decode-raw-"),
        "diagnostic leaked temp path suffix: {json}"
    );
}

#[test]
fn decode_raw_directory_output_path_emits_output_error_text_and_cleans_temp_file() {
    let input = conformance_vector("syn-flat-intra-64x64-minimal.ivf");
    let parent = temp_dir("directory-output-parent");
    let output = parent.join("already-dir.raw");
    std::fs::create_dir(&output).expect("create directory at output path");
    let before_entries = read_dir_names(&parent);

    let out = splot(&[
        "decode",
        "--output-format",
        "raw",
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
        "matrix_row: decode-minimal-raw-runtime-output",
        "feature_id: DECODE-MINIMAL-RAW-RUNTIME-OUTPUT",
        "detail_kind: output_error",
        "output_operation: rename_raw_output",
        "output_source_kind: io",
        "output_format: raw",
    ] {
        assert!(
            stderr.contains(expected),
            "stderr did not contain {expected:?}: {stderr}"
        );
    }
    assert!(
        !stderr.contains(".splot-decode-raw-"),
        "diagnostic leaked temp path suffix: {stderr}"
    );
    assert!(output.is_dir(), "directory output path was replaced");
    assert_eq!(read_dir_names(&parent), before_entries);
}
