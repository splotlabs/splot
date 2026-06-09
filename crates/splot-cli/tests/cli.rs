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
fn inspect_json_reports_parsed_sequence_tile_config() {
    // A sequence header that sets seq_tile_info_present_flag now parses tile_params()
    // in full, so the payload is reported parsed (not bounded).
    let path = fixture("seq-header-tile-params.av2");
    let out = splot(&["inspect", "--json", path.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(0));
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let records = json.as_array().expect("inspect output is an array");
    let sequence_header = &records[1];
    assert_eq!(sequence_header["payload_status"]["status"], "parsed");
    assert_eq!(
        sequence_header["payload_status"]["feature"],
        "AV2-5.4-SEQUENCE-HEADER"
    );
    let view = &sequence_header["sequence_header"];
    assert_eq!(view["fully_parsed"], true);
    assert!(view.get("unimplemented_at").is_none());
    assert_eq!(view["children"]["tile"], true);
    assert_eq!(view["children"]["film_grain_params_present"], true);
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
fn inspect_json_exposes_frame_header_core() {
    // The fixture is TemporalDelimiter, a non-single-picture SequenceHeader (id 0),
    // then an OBU_CLOSED_LOOP_KEY whose first tile group carries a frame header parsed
    // through the full § 5.18.2 intra structure cluster (tile_info, quantization,
    // segmentation, QM setup, delta-q, lossless tail): a 16x16 key frame. The
    // inspector surfaces the core summary.
    let path = fixture("frame-header-core.av2");
    let out = splot(&["inspect", "--json", path.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(0));
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let records = json.as_array().expect("inspect output is an array");
    assert!(
        records.len() >= 3,
        "stdout was: {}",
        String::from_utf8_lossy(&out.stdout)
    );

    let core = &records[2]["frame_header_core"];
    assert_eq!(core["payload_kind"], "frame_header_core");
    assert_eq!(core["status"], "stopped_before_deblocking_filter_params");
    assert_eq!(core["frame_type"], "key");
    assert_eq!(core["frame_is_intra"], true);
    assert_eq!(core["show_existing_frame"], false);
    assert_eq!(core["frame_size"]["width"], 16);
    assert_eq!(core["frame_size"]["height"], 16);

    // The § 5.18.7.2 / § 5.18.6 / § 5.18.7.1 structure summaries: a single-tile
    // layout, base_q_idx == 100 with no deltas, segmentation and the quantizer
    // matrix disabled, no delta-q, and a non-lossless frame.
    let tile = &core["tile_layout"];
    assert_eq!(tile["reuse_tile_info"], false);
    assert_eq!(tile["tile_cols"], 1);
    assert_eq!(tile["tile_rows"], 1);
    assert_eq!(tile["tile_cols_log2"], 0);
    assert_eq!(tile["tile_rows_log2"], 0);
    assert_eq!(tile["context_update_tile_id"], 0);
    assert!(tile.get("tile_size_bytes").is_none());
    let quant = &core["quantization"];
    assert_eq!(quant["base_q_idx"], 100);
    assert_eq!(quant["delta_q_y_dc"], 0);
    assert_eq!(quant["diff_uv_delta"], false);
    let seg = &core["segmentation"];
    assert_eq!(seg["segmentation_enabled"], false);
    let qm = &core["qm_params"];
    assert_eq!(qm["using_qmatrix"], false);
    assert_eq!(qm["levels"].as_array().map(Vec::len), Some(0));
    let delta_q = &core["delta_q"];
    assert_eq!(delta_q["delta_q_present"], false);
    assert_eq!(delta_q["delta_q_res"], 0);
    let lossless = &core["lossless"];
    assert_eq!(lossless["coded_lossless"], false);
    assert_eq!(lossless["has_lossless_segment"], false);
    assert_eq!(lossless["allow_tcq"], false);
    assert_eq!(lossless["allow_parity_hiding"], false);
}

#[test]
fn validate_frame_header_core_fixture_exits_zero() {
    let out = validate("frame-header-core.av2", &[]);
    assert_eq!(out.status.code(), Some(0));
}

#[test]
fn validate_frame_header_prefix_fixture_exits_zero() {
    let out = validate("frame-header-prefix.av2", &[]);
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("conformant"), "stdout was: {stdout}");
}

#[test]
fn inspect_json_surfaces_operating_point_set() {
    // TemporalDelimiter then a global operating_point_set_obu (ops_id 0, ops_cnt 1).
    let path = fixture("operating-point-set.av2");
    let out = splot(&["inspect", "--json", path.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(0));
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let records = json.as_array().expect("inspect output is an array");
    let ops = records
        .iter()
        .find(|r| r.get("operating_point_set").is_some())
        .expect("an operating-point-set record");
    assert_eq!(ops["payload_status"]["status"], "parsed");
    assert_eq!(ops["payload_status"]["syntax"], "operating_point_set_obu");
    let view = &ops["operating_point_set"];
    assert_eq!(view["is_global"], true);
    assert_eq!(view["reset_flag"], false);
    assert_eq!(view["ops_id"], 0);
    assert_eq!(view["ops_cnt"], 1);
    assert_eq!(view["payload_count"], 1);
}

#[test]
fn inspect_json_surfaces_buffer_removal_timing() {
    // TemporalDelimiter, a global OPS (ops_id 0, ops_cnt 1), then an OPS-dependent
    // buffer_removal_timing_obu referencing it.
    let path = fixture("buffer-removal-timing.av2");
    let out = splot(&["inspect", "--json", path.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(0));
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let records = json.as_array().expect("inspect output is an array");
    let brt = records
        .iter()
        .find(|r| r.get("buffer_removal_timing").is_some())
        .expect("a buffer-removal-timing record");
    assert_eq!(brt["payload_status"]["status"], "parsed");
    assert_eq!(brt["payload_status"]["syntax"], "buffer_removal_timing_obu");
    let view = &brt["buffer_removal_timing"];
    assert_eq!(view["ops_dependent"], true);
    assert_eq!(view["br_ops_id"], 0);
    assert_eq!(view["br_ops_cnt"], 1);
    assert_eq!(view["op_count"], 1);
}

#[test]
fn inspect_json_surfaces_quantizer_matrix() {
    // TemporalDelimiter, a sequence header, then a quantizer_matrix_obu selecting
    // level 0 with its default matrix.
    let path = fixture("quantizer-matrix.av2");
    let out = splot(&["inspect", "--json", path.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(0));
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let records = json.as_array().expect("inspect output is an array");
    let qm = records
        .iter()
        .find(|r| r.get("quantizer_matrix").is_some())
        .expect("a quantizer-matrix record");
    assert_eq!(qm["payload_status"]["status"], "parsed");
    assert_eq!(qm["payload_status"]["syntax"], "quantizer_matrix_obu");
    let view = &qm["quantizer_matrix"];
    assert_eq!(view["qm_bit_map"], 1);
    assert_eq!(view["num_planes"], 1);
    assert_eq!(view["is_reset"], false);
    assert_eq!(view["levels"][0]["level"], 0);
    assert_eq!(view["levels"][0]["is_default"], true);
}

#[test]
fn inspect_json_surfaces_padding() {
    // TemporalDelimiter then a global padding_obu (one padding byte + trailing_bits).
    let path = fixture("padding.av2");
    let out = splot(&["inspect", "--json", path.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(0));
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let records = json.as_array().expect("inspect output is an array");
    let padding = records
        .iter()
        .find(|r| r.get("padding").is_some())
        .expect("a padding record");
    assert_eq!(padding["payload_status"]["status"], "parsed");
    assert_eq!(padding["payload_status"]["syntax"], "padding_obu");
    let view = &padding["padding"];
    assert_eq!(view["padding_len"], 1);
    assert_eq!(view["trailing_len"], 1);
}

#[test]
fn inspect_json_surfaces_metadata_short() {
    // TemporalDelimiter then a global metadata_short_obu carrying METADATA_TYPE_HDR_CLL.
    let path = fixture("metadata-short.av2");
    let out = splot(&["inspect", "--json", path.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(0));
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let records = json.as_array().expect("inspect output is an array");
    let metadata = records
        .iter()
        .find(|r| r.get("metadata_short").is_some())
        .expect("a metadata-short record");
    assert_eq!(metadata["payload_status"]["status"], "parsed");
    assert_eq!(metadata["payload_status"]["syntax"], "metadata_short_obu");
    let view = &metadata["metadata_short"];
    assert_eq!(view["is_suffix"], false);
    assert_eq!(view["cancel"], false);
    assert_eq!(view["metadata_type"], 1);
    assert_eq!(view["metadata_type_name"], "METADATA_TYPE_HDR_CLL");
    assert_eq!(view["unit"]["payload_size"], 4);
}

#[test]
fn inspect_json_surfaces_metadata_group() {
    // TemporalDelimiter then a global metadata_group_obu with one HDR_CLL unit.
    let path = fixture("metadata-group.av2");
    let out = splot(&["inspect", "--json", path.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(0));
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let records = json.as_array().expect("inspect output is an array");
    let metadata = records
        .iter()
        .find(|r| r.get("metadata_group").is_some())
        .expect("a metadata-group record");
    assert_eq!(metadata["payload_status"]["status"], "parsed");
    assert_eq!(metadata["payload_status"]["syntax"], "metadata_group_obu");
    let view = &metadata["metadata_group"];
    assert_eq!(view["is_suffix"], false);
    assert_eq!(view["unit_count"], 1);
    assert_eq!(view["units"][0]["metadata_type"], 1);
    assert_eq!(view["units"][0]["cancel"], false);
    assert_eq!(view["units"][0]["payload_size"], 4);
}

#[test]
fn inspect_json_surfaces_film_grain() {
    // TemporalDelimiter, a sequence header, then a film_grain_obu updating slot 0.
    let path = fixture("film-grain.av2");
    let out = splot(&["inspect", "--json", path.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(0));
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let records = json.as_array().expect("inspect output is an array");
    let fg = records
        .iter()
        .find(|r| r.get("film_grain").is_some())
        .expect("a film-grain record");
    assert_eq!(fg["payload_status"]["status"], "parsed");
    assert_eq!(fg["payload_status"]["syntax"], "film_grain_obu");
    let view = &fg["film_grain"];
    assert_eq!(view["fgm_update_flags"], 1);
    assert_eq!(view["fgm_chroma_idc"], 0);
    assert_eq!(view["monochrome"], false);
    assert_eq!(view["updated_slots"][0], 0);
    assert_eq!(view["models"][0]["slot"], 0);
}

#[test]
fn validate_operating_point_set_fixture_exits_zero() {
    let out = validate("operating-point-set.av2", &[]);
    assert_eq!(out.status.code(), Some(0));
}

#[test]
fn validate_buffer_removal_timing_fixture_exits_zero() {
    let out = validate("buffer-removal-timing.av2", &[]);
    assert_eq!(out.status.code(), Some(0));
}

#[test]
fn validate_quantizer_matrix_fixture_exits_zero() {
    let out = validate("quantizer-matrix.av2", &[]);
    assert_eq!(out.status.code(), Some(0));
}

#[test]
fn validate_film_grain_fixture_exits_zero() {
    let out = validate("film-grain.av2", &[]);
    assert_eq!(out.status.code(), Some(0));
}

#[test]
fn missing_input_file_exits_two() {
    let out = validate("does-not-exist.av2", &[]);
    assert_eq!(out.status.code(), Some(2));
}
