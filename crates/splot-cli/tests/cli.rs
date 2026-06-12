// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! End-to-end CLI tests: run the built `splot` binary against the committed
//! fixtures in `tests/fixtures/` and assert on exit codes and output.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};
use std::process::Output;
use std::sync::atomic::{AtomicUsize, Ordering};

static TEMP_COUNTER: AtomicUsize = AtomicUsize::new(0);

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

fn temp_input(extension: &str, data: &[u8]) -> PathBuf {
    let id = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "splot-cli-test-{}-{id}.{extension}",
        std::process::id()
    ));
    std::fs::write(&path, data).expect("write temporary input");
    path
}

fn ivf_stream(payloads: &[&[u8]]) -> Vec<u8> {
    let mut data = Vec::new();
    let header = splot_core::ivf::IvfHeader::new(*b"AV02", 16, 16, 24, 1, payloads.len() as u32);
    splot_core::ivf::write_ivf_header(&mut data, &header).expect("write IVF header");
    for (pts, payload) in payloads.iter().enumerate() {
        splot_core::ivf::write_ivf_frame(&mut data, pts as u64, payload).expect("write IVF frame");
    }
    data
}

#[test]
fn validate_header_only_sequence_header_reports_missing_output_frame_unit() {
    // `conformant.av2` is a temporal delimiter plus a sequence header at obu_xlayer_id 0 with
    // no frame-bearing OBU — a header-only coded extended layer unit. AV2 § 7.3.6 line 536
    // ("at least one coded output frame unit shall be present") applies to every CELU, so the
    // validator reports exactly `celu/missing-output-frame-unit` (exit 1). The fixture exercises
    // the sequence-header parse / inspect paths; this test pins the CELU-completeness finding.
    let out = validate("conformant.av2", &[]);
    assert_eq!(out.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("celu/missing-output-frame-unit"),
        "stdout was: {stdout}"
    );
}

#[test]
fn validate_ivf_conformant_exits_zero() {
    let path = temp_input("ivf", &ivf_stream(&[&[0x01, 0x08]]));
    let out = splot(&["validate", path.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("conformant"), "stdout was: {stdout}");
}

#[test]
fn validate_ivf_json_reports_container_diagnostic() {
    let mut data = ivf_stream(&[&[0x01, 0x08]]);
    data.extend_from_slice(&5u32.to_le_bytes());
    data.extend_from_slice(&1u64.to_le_bytes());
    data.extend_from_slice(&[0x01, 0x08]);
    let path = temp_input("ivf", &data);
    let out = splot(&["validate", "--json", path.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("\"rule_id\": \"ivf/truncated-frame-payload\""),
        "stdout was: {stdout}"
    );
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
fn inspect_ivf_json_includes_container_metadata() {
    let path = temp_input("ivf", &ivf_stream(&[&[0x01, 0x08]]));
    let out = splot(&["inspect", "--json", path.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(0));
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let records = json.as_array().expect("inspect output is an array");
    assert_eq!(records.len(), 1);
    let record = &records[0];
    assert_eq!(record["byte_offset"], 45);
    assert_eq!(record["ivf_header"]["fourcc"], "AV02");
    assert_eq!(record["ivf_header"]["width"], 16);
    assert_eq!(record["ivf_frame"]["index"], 0);
    assert_eq!(record["ivf_frame"]["payload_offset"], 44);
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
    // segmentation, QM setup, delta-q, lossless tail, and the loop-filter cluster
    // deblocking/GDF/CDEF): a 16x16 key frame. The inspector surfaces the core summary.
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
    assert_eq!(core["status"], "stopped_before_read_tx_mode");
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
    assert_eq!(seg["enabled_features"].as_array().map(Vec::len), Some(0));
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
    // The § 5.18.5.2 / § 5.18.7.9 / § 5.18.7.10 loop-filter cluster: deblocking with all
    // apply flags off, and GDF / CDEF disabled at the sequence level.
    let deblocking = &core["deblocking"];
    assert_eq!(
        deblocking["apply_deblocking_filter"],
        serde_json::json!([false, false, false, false])
    );
    assert_eq!(core["gdf"]["gdf_frame_enable"], false);
    assert_eq!(core["cdef"]["cdef_frame_enable"], false);
    // The § 5.18.7.11 / § 5.18.7.12 lr / ccso cluster: restoration and CCSO are disabled at
    // the sequence level, so lr_params() reports the default unit sizes with uses_lr false
    // and ccso_params() returns with no ccso_frame_flag. The parser advances to the stop
    // before read_tx_mode().
    assert_eq!(core["lr"]["uses_lr"], false);
    assert_eq!(
        core["lr"]["loop_restoration_size"],
        serde_json::json!([64, 32, 32])
    );
    assert!(core["ccso"].get("ccso_frame_flag").is_none());
}

#[test]
fn validate_frame_header_core_fixture_exits_zero() {
    let out = validate("frame-header-core.av2", &[]);
    assert_eq!(out.status.code(), Some(0));
}

#[test]
fn inspect_json_exposes_mfh_backed_frame_header_core() {
    // The fixture is TemporalDelimiter, a non-single-picture SequenceHeader (id 0),
    // an in-band MultiFrameHeader (mfhId 1 -> mfh_seq_header_id 0, no frame-size payload
    // so the § 5.18.2 omitted-size inference applies, no segment info), then an
    // OBU_CLOSED_LOOP_KEY whose first tile group references `cur_mfh_id = 1`. The
    // inspector resolves the MFH to its sequence header and surfaces the core summary:
    // the § 5.18.4.1 default dimensions come from the MFH (inferred to the 16x16
    // sequence maxima), and segmentation parses its sequence/zero arm.
    let path = fixture("frame-header-core-mfh.av2");
    let out = splot(&["inspect", "--json", path.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(0));
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let records = json.as_array().expect("inspect output is an array");
    assert!(
        records.len() >= 4,
        "stdout was: {}",
        String::from_utf8_lossy(&out.stdout)
    );

    // Index 2 is the multi-frame header; index 3 is the cur_mfh_id == 1 frame.
    assert_eq!(records[2]["header"]["obu_type"], "MultiFrameHeader");
    let core = &records[3]["frame_header_core"];
    assert_eq!(core["payload_kind"], "frame_header_core");
    assert_eq!(core["status"], "stopped_before_read_tx_mode");
    assert_eq!(core["cur_mfh_id"], 1);
    assert_eq!(core["frame_type"], "key");
    assert_eq!(core["frame_is_intra"], true);
    // Omitted MFH frame size -> § 5.18.2 (:4101) infers the 16x16 sequence maxima.
    assert_eq!(core["frame_size"]["width"], 16);
    assert_eq!(core["frame_size"]["height"], 16);
    let tile = &core["tile_layout"];
    assert_eq!(tile["tile_cols"], 1);
    assert_eq!(tile["tile_rows"], 1);
    assert_eq!(core["quantization"]["base_q_idx"], 100);
    assert_eq!(core["segmentation"]["segmentation_enabled"], false);
    // The resolved MFH did not signal a deblocking update (mfh_deblocking_filter_update
    // == 0), so apply_deblocking_filter[0]/[1] are read from the frame (both 0); GDF and
    // CDEF are disabled at the sequence level.
    assert_eq!(
        core["deblocking"]["apply_deblocking_filter"],
        serde_json::json!([false, false, false, false])
    );
    assert_eq!(core["gdf"]["gdf_frame_enable"], false);
    assert_eq!(core["cdef"]["cdef_frame_enable"], false);
    // Restoration / CCSO disabled at the sequence level -> lr_params() reports the default
    // sizes and the parser stops before read_tx_mode().
    assert_eq!(core["lr"]["uses_lr"], false);
    assert!(core["ccso"].get("ccso_frame_flag").is_none());
}

#[test]
fn validate_frame_header_core_mfh_fixture_exits_zero() {
    // The cur_mfh_id == 1 frame is an output key frame (immediate_output_frame == 1),
    // so the lone coded extended layer unit satisfies the § 7.3.6 output rule.
    let out = validate("frame-header-core-mfh.av2", &[]);
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("conformant"), "stdout was: {stdout}");
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
fn validate_quantizer_matrix_fixture_reports_missing_output_frame_unit() {
    // The quantizer-matrix fixture is a header-only coded extended layer unit at
    // obu_xlayer_id 0 (sequence header + quantizer_matrix_obu, no frame-bearing OBU), so
    // § 7.3.6 line 536 fires `celu/missing-output-frame-unit` (exit 1). It exercises the QM
    // parse / inspect paths.
    let out = validate("quantizer-matrix.av2", &[]);
    assert_eq!(out.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("celu/missing-output-frame-unit"),
        "stdout was: {stdout}"
    );
}

#[test]
fn validate_film_grain_fixture_reports_missing_output_frame_unit() {
    // The film-grain fixture is a header-only coded extended layer unit at obu_xlayer_id 0
    // (sequence header + film_grain_obu, no frame-bearing OBU), so § 7.3.6 line 536 fires
    // `celu/missing-output-frame-unit` (exit 1). It exercises the FGM parse / inspect paths.
    let out = validate("film-grain.av2", &[]);
    assert_eq!(out.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("celu/missing-output-frame-unit"),
        "stdout was: {stdout}"
    );
}

#[test]
fn missing_input_file_exits_two() {
    let out = validate("does-not-exist.av2", &[]);
    assert_eq!(out.status.code(), Some(2));
}
