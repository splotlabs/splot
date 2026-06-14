// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! End-to-end `splot decode` CLI contract tests.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};
use std::process::Output;
use std::sync::atomic::{AtomicUsize, Ordering};

use splot_decode::unsupported_feature_diagnostic;

static TEMP_COUNTER: AtomicUsize = AtomicUsize::new(0);

const PLANABLE_CLOSED_LOOP_KEY: &[u8] = &[0x01, 0x10];
const UNSUPPORTED_OPEN_LOOP_KEY: &[u8] = &[0x01, 0x14];
const MALFORMED_ANNEX_B: &[u8] = &[0x05, 0x10];

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

fn repeated_sequence_header_obus(count: usize) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(count * 2);
    for _ in 0..count {
        bytes.extend_from_slice(&[0x01, 0x08]);
    }
    bytes
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
fn decode_plan_success_text_mode_emits_runtime_unsupported_diagnostic() {
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
        "detail_kind: runtime_unsupported".to_string(),
        "bitstream_format: annex_b".to_string(),
        "input_len_bytes: 2".to_string(),
        "obu_count: 1".to_string(),
        "frame_candidate_count: 1".to_string(),
        "output_format: y4m".to_string(),
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
fn decode_plan_success_json_mode_emits_runtime_unsupported_object() {
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
    assert_eq!(json["detail_kind"], "runtime_unsupported");
    assert_eq!(json["output_format"], "y4m");
    assert_eq!(json["bitstream_format"], "annex_b");
    assert_eq!(json["input_len_bytes"], 2);
    assert_eq!(json["obu_count"], 1);
    assert_eq!(json["frame_candidate_count"], 1);
    assert_eq!(json["source_warning_count"], 0);
    assert_eq!(json["selected_temporal_layer_id"], 0);
    assert_eq!(json["selected_embedded_layer_id"], 0);
    assert_eq!(json["selected_extended_layer_id"], 0);
    assert_eq!(
        std::fs::read(&output).expect("read temporary output sentinel"),
        original_output
    );
}

#[test]
fn decode_missing_input_is_operational_error_and_does_not_touch_files() {
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

    assert_eq!(out.status.code(), Some(2));
    assert!(out.stdout.is_empty(), "stdout was not empty");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("failed to read input file"),
        "stderr was: {stderr}"
    );
    assert!(
        !stderr.contains("decode/"),
        "operational error emitted decode diagnostic: {stderr}"
    );
    assert!(!input.exists(), "decode created the missing input path");
    assert!(!output.exists(), "decode created the output path");
}

#[test]
fn decode_hash_output_format_emits_unsupported_text_without_output_path() {
    let input = temp_input("av2", PLANABLE_CLOSED_LOOP_KEY);

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
        "detail_kind: runtime_unsupported".to_string(),
        "output_format: hash".to_string(),
    ] {
        assert!(
            stderr.contains(&expected),
            "stderr did not contain {expected:?}: {stderr}"
        );
    }
}

#[test]
fn decode_hash_output_format_missing_input_is_operational_error() {
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

    assert_eq!(out.status.code(), Some(2));
    assert!(out.stdout.is_empty(), "stdout was not empty");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("failed to read input file"),
        "stderr was: {stderr}"
    );
    assert!(
        !stderr.contains("decode/"),
        "operational error emitted decode diagnostic: {stderr}"
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
    let input = temp_input("av2", PLANABLE_CLOSED_LOOP_KEY);

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
    assert_eq!(json["detail_kind"], "runtime_unsupported");
    assert_eq!(json["output_format"], "hash");
}

#[test]
fn decode_malformed_source_text_mode_emits_structured_diagnostic() {
    let input = temp_input("av2", MALFORMED_ANNEX_B);
    let output = temp_output("y4m");
    let original_output = b"malformed output sentinel";
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
        "rule_id: decode/malformed-source",
        "severity: Error",
        "spec_section: 5.2.1",
        "matrix_row: decode-byte-stream-planner",
        "feature_id: DECODE-BYTE-STREAM-PLANNER",
        "detail_kind: malformed_source",
        "source_issue_kind: annex_b_parse_error",
        "output_format: y4m",
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
fn decode_malformed_source_json_mode_emits_detail_fields() {
    let input = temp_input("av2", MALFORMED_ANNEX_B);

    let out = splot(&[
        "decode",
        "--json",
        "--output-format",
        "hash",
        input.to_str().unwrap(),
    ]);

    assert_eq!(out.status.code(), Some(1));
    assert!(out.stderr.is_empty(), "stderr was not empty");
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(json["rule_id"], "decode/malformed-source");
    assert_eq!(json["severity"], "Error");
    assert_eq!(json["spec_section"], "5.2.1");
    assert_eq!(json["matrix_row"], "decode-byte-stream-planner");
    assert_eq!(json["feature_id"], "DECODE-BYTE-STREAM-PLANNER");
    assert_eq!(json["detail_kind"], "malformed_source");
    assert_eq!(json["source_issue_kind"], "annex_b_parse_error");
    assert_eq!(json["output_format"], "hash");
    assert!(
        json["byte_offset"].is_u64(),
        "json missing byte_offset: {json}"
    );
    assert!(
        json["parser_message"].is_string(),
        "json missing parser_message: {json}"
    );
}

#[test]
fn decode_unsupported_structure_json_mode_uses_planner_metadata() {
    let input = temp_input("av2", UNSUPPORTED_OPEN_LOOP_KEY);
    let output = temp_output("y4m");
    let original_output = b"unsupported output sentinel";
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
    assert_eq!(json["spec_section"], "5.2.1");
    assert_eq!(json["matrix_row"], "decode-stream-state");
    assert_eq!(json["feature_id"], "DECODE-STREAM-STATE-PLANNER");
    assert_eq!(json["detail_kind"], "unsupported_structure");
    assert_eq!(json["unsupported_reason"], "unsupported_frame_obu");
    assert_eq!(json["obu_type"], "OBU_OPEN_LOOP_KEY");
    assert_eq!(json["byte_offset"], 1);
    assert_eq!(
        std::fs::read(&output).expect("read temporary output sentinel"),
        original_output
    );
}

#[test]
fn decode_resource_limit_json_mode_reports_limit_values() {
    let limit_input = repeated_sequence_header_obus(4_097);
    let input = temp_input("av2", &limit_input);
    let output = temp_output("hashes");
    let original_output = b"resource-limit output sentinel";
    std::fs::write(&output, original_output).expect("write temporary output sentinel");

    let out = splot(&[
        "decode",
        "--json",
        "--output-format",
        "hash",
        input.to_str().unwrap(),
        "-o",
        output.to_str().unwrap(),
    ]);

    assert_eq!(out.status.code(), Some(1));
    assert!(out.stderr.is_empty(), "stderr was not empty");
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(json["rule_id"], "decode/resource-limit");
    assert_eq!(json["severity"], "Error");
    assert_eq!(json["spec_section"], "5.2.1");
    assert_eq!(json["matrix_row"], "decode-limits-budget");
    assert_eq!(json["feature_id"], "DOC-DECODE-LIMITS-CONTRACT");
    assert_eq!(json["detail_kind"], "resource_limit");
    assert_eq!(json["limit_name"], "max_obus");
    assert_eq!(json["limit"], 4_096);
    assert_eq!(json["actual"], 4_097);
    assert_eq!(json["unit"], "count");
    assert_eq!(json["output_format"], "hash");
    assert_eq!(
        std::fs::read(&output).expect("read temporary output sentinel"),
        original_output
    );
}

#[test]
fn decode_invalid_output_format_is_usage_error() {
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
fn decode_hash_output_format_with_output_path_does_not_touch_file() {
    let input = temp_input("av2", PLANABLE_CLOSED_LOOP_KEY);
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
    let input = temp_input("av2", PLANABLE_CLOSED_LOOP_KEY);

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
fn decode_threads_fixed_is_accepted_emits_unsupported() {
    let input = temp_input("av2", PLANABLE_CLOSED_LOOP_KEY);

    let out = splot(&[
        "decode",
        "--threads",
        "8",
        "--output-format",
        "hash",
        input.to_str().unwrap(),
    ]);

    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("decode/unsupported-feature"),
        "stderr was: {stderr}"
    );
}

#[test]
fn decode_threads_auto_is_accepted() {
    let input = temp_input("av2", PLANABLE_CLOSED_LOOP_KEY);

    let out = splot(&[
        "decode",
        "--threads",
        "auto",
        "--output-format",
        "hash",
        input.to_str().unwrap(),
    ]);

    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("decode/unsupported-feature"),
        "stderr was: {stderr}"
    );
}

#[test]
fn decode_thread_policies_emit_same_json_diagnostic() {
    let input = temp_input("av2", PLANABLE_CLOSED_LOOP_KEY);
    let mut outputs = Vec::new();

    for threads in ["auto", "1", "4"] {
        let out = splot(&[
            "decode",
            "--json",
            "--threads",
            threads,
            "--output-format",
            "hash",
            input.to_str().unwrap(),
        ]);

        assert_eq!(out.status.code(), Some(1), "threads={threads}");
        assert!(out.stderr.is_empty(), "stderr was not empty for {threads}");
        outputs.push(out.stdout);
    }

    assert_eq!(outputs[0], outputs[1]);
    assert_eq!(outputs[1], outputs[2]);
}

#[test]
fn decode_threads_invalid_is_usage_error() {
    let input = temp_input("av2", PLANABLE_CLOSED_LOOP_KEY);

    let out = splot(&[
        "decode",
        "--threads",
        "nope",
        "--output-format",
        "hash",
        input.to_str().unwrap(),
    ]);

    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !stderr.contains("decode/unsupported-feature"),
        "stderr was: {stderr}"
    );
}
