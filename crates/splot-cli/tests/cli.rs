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

fn inspect_json(path: &Path) -> serde_json::Value {
    let out = splot(&[
        "inspect",
        "--json",
        path.to_str().expect("inspect input path is valid UTF-8"),
    ]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "stdout: {} stderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).expect("inspect output is valid JSON")
}

fn temp_path(stem: &str, extension: &str) -> PathBuf {
    let id = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system time is after Unix epoch")
        .as_nanos();
    std::env::temp_dir().join(format!(
        "splot-cli-test-{stem}-{}-{nanos}-{id}.{extension}",
        std::process::id()
    ))
}

fn temp_input(extension: &str, data: &[u8]) -> PathBuf {
    let path = temp_path("input", extension);
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
fn validate_reads_from_stdin_dash_and_matches_file() {
    use std::io::Write;
    use std::process::{Command, Stdio};

    let data = ivf_stream(&[&[0x02, 0x88, 0x05]]);
    let path = temp_input("ivf", &data);

    let file_out = splot(&["validate", path.to_str().unwrap()]);

    let mut child = Command::new(env!("CARGO_BIN_EXE_splot"))
        .args(["validate", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn splot");
    child
        .stdin
        .take()
        .expect("child stdin")
        .write_all(&data)
        .expect("write to child stdin");
    let stdin_out = child.wait_with_output().expect("wait for splot");

    assert_eq!(file_out.status.code(), stdin_out.status.code());
    assert_eq!(file_out.stdout, stdin_out.stdout);
}

#[test]
fn validate_ivf_trailing_partial_frame_header_exits_zero() {
    let mut data = ivf_stream(&[&[0x01, 0x08]]);
    data.extend_from_slice(&1148u32.to_le_bytes());
    data.extend_from_slice(&6480u64.to_le_bytes()[..6]);
    let path = temp_input("ivf", &data);
    let out = splot(&["validate", path.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("[WARNING] ivf/trailing-partial-frame-header"),
        "stdout was: {stdout}"
    );
    assert!(
        stdout.contains("0 error(s), 1 warning(s)"),
        "stdout was: {stdout}"
    );
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
    let json = inspect_json(&path);
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
fn inspect_ivf_trailing_partial_frame_header_prints_warning() {
    let mut data = ivf_stream(&[&[0x01, 0x08]]);
    data.extend_from_slice(&1148u32.to_le_bytes());
    data.extend_from_slice(&6480u64.to_le_bytes()[..6]);
    let path = temp_input("ivf", &data);
    let out = splot(&["inspect", path.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("1 OBU(s)"), "stdout was: {stdout}");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("warning: ivf/trailing-partial-frame-header"),
        "stderr was: {stderr}"
    );
}

#[test]
fn inspect_ivf_json_trailing_partial_frame_header_prints_warning_on_stderr() {
    let mut data = ivf_stream(&[&[0x01, 0x08]]);
    data.extend_from_slice(&1148u32.to_le_bytes());
    data.extend_from_slice(&6480u64.to_le_bytes()[..6]);
    let path = temp_input("ivf", &data);
    let out = splot(&["inspect", "--json", path.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(0));
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let records = json.as_array().expect("inspect output is an array");
    assert_eq!(records.len(), 1);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("warning: ivf/trailing-partial-frame-header"),
        "stderr was: {stderr}"
    );
}

#[test]
fn inspect_json_includes_payload_status_without_dropping_header_fields() {
    let path = fixture("conformant.av2");
    let json = inspect_json(&path);
    let records = json.as_array().expect("inspect output is an array");
    assert!(records.len() >= 2, "records were: {json}");

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
    let path = fixture("seq-header-tile-params.av2");
    let json = inspect_json(&path);
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
    let path = fixture("frame-header-prefix.av2");
    let json = inspect_json(&path);
    let records = json.as_array().expect("inspect output is an array");
    assert!(records.len() >= 3, "records were: {json}");

    let frame = &records[2];
    let prefix = &frame["frame_header_prefix"];
    assert_eq!(prefix["payload_kind"], "frame_header_prefix");
    assert_eq!(prefix["prefix_status"], "activation_fields_only");
    assert_eq!(prefix["cur_mfh_id"], 0);
    assert_eq!(prefix["seq_header_id_in_frame_header"], 0);
    assert_eq!(prefix["referenced_sequence_header_id"], 0);
    assert_eq!(prefix["is_key_frame"], true);
    let status = &frame["payload_status"];
    assert_eq!(status["status"], "prefix_parsed_awaiting_state");
    assert_eq!(status["syntax"], "tile_group_prefix");
    assert_eq!(status["feature"], "AV2-5.19-TILE-GROUP");
    assert_eq!(status["blocked_on"], "active sequence header state");
}

#[test]
fn inspect_json_exposes_frame_header_core() {
    let path = fixture("frame-header-core.av2");
    let json = inspect_json(&path);
    let records = json.as_array().expect("inspect output is an array");
    assert!(records.len() >= 3, "records were: {json}");

    let core = &records[2]["frame_header_core"];
    assert_eq!(core["payload_kind"], "frame_header_core");
    assert_eq!(core["status"], "intra_header_complete");
    assert_eq!(core["frame_type"], "key");
    assert_eq!(core["frame_is_intra"], true);
    assert_eq!(core["show_existing_frame"], false);
    assert_eq!(core["frame_size"]["width"], 16);
    assert_eq!(core["frame_size"]["height"], 16);

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
    let deblocking = &core["deblocking"];
    assert_eq!(
        deblocking["apply_deblocking_filter"],
        serde_json::json!([false, false, false, false])
    );
    assert_eq!(core["gdf"]["gdf_frame_enable"], false);
    assert_eq!(core["cdef"]["cdef_frame_enable"], false);
    assert_eq!(core["lr"]["uses_lr"], false);
    assert_eq!(
        core["lr"]["loop_restoration_size"],
        serde_json::json!([64, 32, 32])
    );
    assert!(core["ccso"].get("ccso_frame_flag").is_none());
    let tail = &core["intra_tail"];
    assert_eq!(tail["tx_mode"], "tx_mode_largest");
    assert_eq!(tail["reference_select"], false);
    assert_eq!(tail["skip_mode_present"], false);
    assert_eq!(tail["allow_bawp"], false);
    assert_eq!(tail["use_global_motion"], false);
    assert_eq!(tail["film_grain"]["apply_grain"], false);

    let structure = &records[2]["tile_group_structure"];
    assert_eq!(structure["payload_kind"], "tile_group_structure");
    assert_eq!(structure["num_tiles"], 1);
    assert_eq!(structure["tile_start_and_end_present_flag"], false);
    assert_eq!(structure["tg_start"], 0);
    assert_eq!(structure["tg_end"], 0);
    assert_eq!(structure["status"], "complete");
    assert!(structure["header_bytes"].is_u64());
    assert!(structure["payload_size"].is_u64());

    let framing = structure["tile_framing"]
        .as_array()
        .expect("tile_framing is an array");
    assert_eq!(framing.len(), 1, "single tile -> one framing record");
    assert_eq!(framing[0]["tile_num"], 0);
    assert!(
        framing[0].get("size_field_offset").is_none(),
        "the lone last tile reads no le(TileSizeBytes) size field"
    );
    assert_eq!(framing[0]["tile_data_offset"], 0);
    assert_eq!(framing[0]["tile_size"], structure["payload_size"]);
    assert!(structure.get("tile_framing_defect").is_none());
}

#[test]
fn validate_frame_header_core_fixture_exits_zero() {
    let out = validate("frame-header-core.av2", &[]);
    assert_eq!(out.status.code(), Some(0));
}

/// Frames one OBU as Annex B `leb128(num_bytes_in_obu)` + header byte + payload. The
/// single-byte size is a valid leb128 for sizes < 128 (matches the validator's framing).
fn annex_b_obu(header: u8, payload: &[u8]) -> Vec<u8> {
    let size = payload.len() + 1;
    assert!(u8::try_from(size).is_ok());
    let mut data = Vec::with_capacity(size + 1);
    data.push(size as u8);
    data.push(header);
    data.extend_from_slice(payload);
    data
}

#[test]
fn inspect_json_surfaces_frame_header_copy_on_non_first_tile_group() {
    let mut data = annex_b_obu(0x12, &[]); // OBU_TEMPORAL_DELIMITER (global)
    data.extend(annex_b_obu(0x10, &[0b0110_1010]));
    let path = temp_input("av2", &data);
    let json = inspect_json(&path);
    let records = json.as_array().expect("inspect output is an array");
    let copy = &records[1]["frame_header_copy"];
    assert_eq!(copy["payload_kind"], "frame_header_copy");
    assert_eq!(copy["compared"], false);
    assert_eq!(copy["copy_region_start_byte"], 4);
    assert_eq!(copy["copy_region_start_bit"], 2);
    assert!(records[0].get("frame_header_copy").is_none());
}

#[test]
fn inspect_json_exposes_mfh_backed_frame_header_core() {
    let path = fixture("frame-header-core-mfh.av2");
    let json = inspect_json(&path);
    let records = json.as_array().expect("inspect output is an array");
    assert!(records.len() >= 4, "records were: {json}");

    assert_eq!(records[2]["header"]["obu_type"], "MultiFrameHeader");
    let core = &records[3]["frame_header_core"];
    assert_eq!(core["payload_kind"], "frame_header_core");
    assert_eq!(core["status"], "intra_header_complete");
    assert_eq!(core["cur_mfh_id"], 1);
    assert_eq!(core["frame_type"], "key");
    assert_eq!(core["frame_is_intra"], true);
    assert_eq!(core["frame_size"]["width"], 16);
    assert_eq!(core["frame_size"]["height"], 16);
    let tile = &core["tile_layout"];
    assert_eq!(tile["tile_cols"], 1);
    assert_eq!(tile["tile_rows"], 1);
    assert_eq!(core["quantization"]["base_q_idx"], 100);
    assert_eq!(core["segmentation"]["segmentation_enabled"], false);
    assert_eq!(
        core["deblocking"]["apply_deblocking_filter"],
        serde_json::json!([false, false, false, false])
    );
    assert_eq!(core["gdf"]["gdf_frame_enable"], false);
    assert_eq!(core["cdef"]["cdef_frame_enable"], false);
    assert_eq!(core["lr"]["uses_lr"], false);
    assert!(core["ccso"].get("ccso_frame_flag").is_none());
    let tail = &core["intra_tail"];
    assert_eq!(tail["tx_mode"], "tx_mode_largest");
    assert_eq!(tail["reduced_tx_set"], 2);
    assert_eq!(tail["film_grain"]["apply_grain"], false);
}

#[test]
fn validate_frame_header_core_mfh_fixture_exits_zero() {
    let out = validate("frame-header-core-mfh.av2", &[]);
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("conformant"), "stdout was: {stdout}");
}

#[test]
fn inspect_json_surfaces_inter_disable_cdf_update() {
    let data: [u8; 28] = [
        0x01, 0x08, 0x13, 0x04, 0x80, 0x0c, 0x01, 0x77, 0x0f, 0x0f, 0x00, 0x00, 0x00, 0x07, 0x70,
        0x00, 0x00, 0x06, 0x00, 0x10, 0x00, 0x02, 0x05, 0x1c, 0xf8, 0x00, 0x48, 0x08,
    ];
    let path = temp_input("av2", &data);
    let json = inspect_json(&path);
    let records = json.as_array().expect("inspect output is an array");
    let core = &records[2]["frame_header_core"];
    assert_eq!(core["frame_type"], "inter");
    assert_eq!(core["frame_is_intra"], false);
    let inter = &core["inter"];
    assert_eq!(inter["stop"], "reached_shared_tail");
    assert_eq!(
        inter["disable_cdf_update"], false,
        "the inter view must surface the parsed §5.18.2 disable_cdf_update bit; \
         records were: {json}"
    );
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
    let path = fixture("operating-point-set.av2");
    let json = inspect_json(&path);
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
    let path = fixture("buffer-removal-timing.av2");
    let json = inspect_json(&path);
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
    let path = fixture("quantizer-matrix.av2");
    let json = inspect_json(&path);
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
    let path = fixture("padding.av2");
    let json = inspect_json(&path);
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
    let path = fixture("metadata-short.av2");
    let json = inspect_json(&path);
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
    let path = fixture("metadata-group.av2");
    let json = inspect_json(&path);
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
    let path = fixture("film-grain.av2");
    let json = inspect_json(&path);
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

#[test]
fn validate_max_diagnostics_preserves_exit_and_counts() {
    let uncapped = validate("bad-global-xlayer.av2", &[]);
    let capped = validate("bad-global-xlayer.av2", &["--max-diagnostics", "1"]);
    assert_eq!(capped.status.code(), uncapped.status.code());
    assert_eq!(capped.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&capped.stdout);
    assert!(
        stdout.contains("... 1 more diagnostic(s) not shown (--max-diagnostics 1)"),
        "stdout was: {stdout}"
    );
    assert!(
        stdout.contains("2 error(s), 0 warning(s), 0 info"),
        "stdout was: {stdout}"
    );
}

#[test]
fn validate_summary_only_preserves_exit_codes() {
    let bad = validate("bad-global-xlayer.av2", &["--summary-only"]);
    assert_eq!(bad.status.code(), Some(1));
    let bad_stdout = String::from_utf8_lossy(&bad.stdout);
    assert!(
        !bad_stdout.contains("obu-header/global-xlayer-required"),
        "summary-only must omit per-diagnostic lines; stdout was: {bad_stdout}"
    );
    assert!(
        bad_stdout.contains("2 error(s)"),
        "stdout was: {bad_stdout}"
    );

    let good = validate("operating-point-set.av2", &["--summary-only"]);
    assert_eq!(good.status.code(), Some(0));
    assert!(
        String::from_utf8_lossy(&good.stdout).contains("conformant"),
        "stdout was: {}",
        String::from_utf8_lossy(&good.stdout)
    );
}

#[test]
fn validate_rejects_non_numeric_max_diagnostics() {
    let out = validate(
        "bad-global-xlayer.av2",
        &["--max-diagnostics", "notanumber"],
    );
    assert_eq!(out.status.code(), Some(2));
}
