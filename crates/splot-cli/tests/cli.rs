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
fn inspect_json_includes_payload_status_without_dropping_header_fields() {
    let path = fixture("conformant.av2");
    let out = splot(&["inspect", "--json", path.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(0));
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let records = json.as_array().expect("inspect output is an array");
    assert!(
        records.len() >= 2,
        "stdout was: {}",
        String::from_utf8_lossy(&out.stdout)
    );

    let temporal_delimiter = &records[0];
    assert_eq!(temporal_delimiter["payload_status"]["status"], "parsed");
    assert_eq!(
        temporal_delimiter["payload_status"]["feature"],
        "AV2-5.5-TEMPORAL-DELIMITER"
    );
    assert!(temporal_delimiter.get("header").is_some());
    assert!(temporal_delimiter.get("payload_len").is_some());

    let sequence_header = &records[1];
    assert_eq!(sequence_header["payload_status"]["status"], "parsed");
    assert_eq!(
        sequence_header["payload_status"]["feature"],
        "AV2-5.4-SEQUENCE-HEADER"
    );
    assert!(sequence_header.get("header").is_some());

    // The parsed sequence header exposes its §5.4 child sections.
    let view = &sequence_header["sequence_header"];
    assert_eq!(view["fully_parsed"], true);
    assert_eq!(view["single_picture_header_flag"], true);
    assert_eq!(view["children"]["partition"], true);
    assert_eq!(view["children"]["tile"], true);
    assert_eq!(view["children"]["film_grain_params_present"], true);
}

#[test]
fn inspect_json_reports_bounded_sequence_header_child() {
    // A sequence header that sets seq_tile_info_present_flag bounds parsing at the
    // unimplemented tile_params() helper.
    let path = fixture("seq-header-tile-unimplemented.av2");
    let out = splot(&["inspect", "--json", path.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(0));
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let records = json.as_array().expect("inspect output is an array");
    let sequence_header = &records[1];
    assert_eq!(sequence_header["payload_status"]["status"], "unimplemented");
    assert_eq!(
        sequence_header["payload_status"]["feature"],
        "AV2-5.4.2-SEQUENCE-TILE-CONFIG"
    );
    let view = &sequence_header["sequence_header"];
    assert_eq!(view["fully_parsed"], false);
    assert_eq!(view["unimplemented_at"], "AV2-5.4.2-SEQUENCE-TILE-CONFIG");
    assert_eq!(view["children"]["filter"], true);
    assert_eq!(view["children"]["film_grain_params_present"], false);
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
fn inspect_json_exposes_frame_header_prefix() {
    // The fixture is TemporalDelimiter, SequenceHeader (id 0), then an
    // OBU_CLOSED_LOOP_KEY whose first tile group carries a frame header referencing
    // seq_header_id 0. The inspector surfaces the prefix-only activation fields.
    let path = fixture("frame-header-prefix.av2");
    let out = splot(&["inspect", "--json", path.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(0));
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let records = json.as_array().expect("inspect output is an array");
    assert!(
        records.len() >= 3,
        "stdout was: {}",
        String::from_utf8_lossy(&out.stdout)
    );

    let frame = &records[2];
    let prefix = &frame["frame_header_prefix"];
    assert_eq!(prefix["payload_kind"], "frame_header_prefix");
    assert_eq!(prefix["prefix_status"], "activation_fields_only");
    assert_eq!(prefix["cur_mfh_id"], 0);
    assert_eq!(prefix["seq_header_id_in_frame_header"], 0);
    assert_eq!(prefix["referenced_sequence_header_id"], 0);
    assert_eq!(prefix["is_key_frame"], true);
    // The payload itself is only prefix-parsed, never a complete frame header.
    assert_eq!(frame["payload_status"]["status"], "unimplemented");
}

#[test]
fn validate_frame_header_prefix_fixture_exits_zero() {
    let out = validate("frame-header-prefix.av2", &[]);
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("conformant"), "stdout was: {stdout}");
}

#[test]
fn missing_input_file_exits_two() {
    let out = validate("does-not-exist.av2", &[]);
    assert_eq!(out.status.code(), Some(2));
}
