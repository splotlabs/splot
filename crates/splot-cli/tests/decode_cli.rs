// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! End-to-end `splot decode` CLI contract tests.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};
use std::process::Output;
use std::sync::atomic::{AtomicUsize, Ordering};

use splot_decode::DecodeOptions;

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

fn conformance_vector(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/conformance/vectors/valid")
        .join(name)
}

fn repeated_sequence_header_obus(count: usize) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(count * 2);
    for _ in 0..count {
        bytes.extend_from_slice(&[0x01, 0x08]);
    }
    bytes
}

fn default_max_input_bytes() -> u64 {
    DecodeOptions::default()
        .limits()
        .max_input_bytes()
        .max_value()
        .expect("default max_input_bytes is finite")
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

fn decode_hash_json(path: &Path, threads: &str) -> serde_json::Value {
    let out = splot(&[
        "decode",
        "--output-format",
        "hash",
        "--json",
        "--threads",
        threads,
        path.to_str().unwrap(),
    ]);
    assert_eq!(out.status.code(), Some(0), "threads={threads}");
    assert!(
        out.stderr.is_empty(),
        "stderr was: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).unwrap()
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
    for expected in [
        "rule_id: decode/unsupported-feature".to_string(),
        "severity: Error".to_string(),
        "spec_section: 7.1".to_string(),
        "matrix_row: minimal-decode-tier-contract".to_string(),
        "feature_id: DECODE-MINIMAL-TIER-RUNTIME-SUCCESS".to_string(),
        "detail_kind: unsupported_feature".to_string(),
        "unsupported_reason: unexpected_planned_stream_shape".to_string(),
        "tier_id: minimal-intra-8bit420-hash-v1".to_string(),
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
    assert_eq!(json["output_format"], "hash");
}

#[test]
fn decode_hash_json_success_for_minimal_fixture() {
    let input = conformance_vector("syn-flat-intra-64x64-minimal.ivf");

    let out = splot(&[
        "decode",
        "--json",
        "--output-format",
        "hash",
        input.to_str().unwrap(),
    ]);

    assert_eq!(out.status.code(), Some(0));
    assert!(out.stderr.is_empty(), "stderr was not empty");
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(json["contract_id"], "splot.decode.hash_report");
    assert_eq!(json["contract_version"], 1);
    assert_eq!(
        json["selected_output_variants"][0],
        "raw_intermediate_output"
    );
    assert_eq!(json["frames"].as_array().unwrap().len(), 1);
    let frame = &json["frames"][0];
    assert_eq!(frame["output_index"], 0);
    assert_eq!(frame["visible_luma_width"], 64);
    assert_eq!(frame["visible_luma_height"], 64);
    assert_eq!(frame["chroma_width"], 32);
    assert_eq!(frame["chroma_height"], 32);
    assert_eq!(frame["bit_depth"], 8);
    assert_eq!(frame["pixel_format"], "yuv420");
    assert_eq!(frame["hashes"][0]["variant"], "raw_intermediate_output");
    assert_eq!(frame["hashes"][0]["algorithm_id"], "splot-dfh-sha256-v1");
    assert_eq!(
        frame["hashes"][0]["byte_stream_id"],
        "av2-output-samples-v1"
    );
    // SHA-256 of the decoded raw planar output. The conformant luma-skip fixture
    // routes through the general intra path; avmdec and dav2d both decode it to
    // this output (docs/LOCAL-REFERENCE-EVIDENCE.toml).
    assert_eq!(
        frame["hashes"][0]["digest_hex"],
        "92c4477c8b50d5646c6ed5351cbb8f4fc04517ba39354a127c306e196fd059af"
    );
}

#[test]
fn decode_general_intra_fixture_reconstructs_full_frame() {
    // A real AVM-generated minimal-tool intra key frame (base_q_idx 80, one
    // 64x64 block carrying a nonzero DC residual). splot routes it off the
    // frozen base_q_idx==255 hash tier into the general intra path, runs the
    // real AV2 §5.20.3.1 partition traversal, decodes the §5.20.5.3 block
    // mode-info symbols, decodes the §5.20.7.27 luma + chroma transform-block
    // coefficients, then dequantizes / inverse-transforms / residual-adds each
    // plane and reconstructs the full frame. avmdec and dav2d both decode this
    // fixture to flat planes Y=100, U=120, V=130; the splot hash of that frame is
    // pinned here as the first oracle-anchored full-frame decode.
    let input = conformance_vector("syn-flat-intra-64x64-q80.ivf");

    let out = splot(&[
        "decode",
        "--json",
        "--output-format",
        "hash",
        input.to_str().unwrap(),
    ]);

    assert_eq!(out.status.code(), Some(0));
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(
        json["frames"][0]["hashes"][0]["digest_hex"],
        "ce9c46b1078b9dd593254837ead7dcd6cee8b3ec6cc3c7d34f54fb08df703979"
    );
}

#[test]
fn general_intra_fixtures_decode_to_distinct_pinned_hashes() {
    // Both committed minimal-tool intra fixtures route through the general intra
    // path: the former frozen base_q_idx==255 minimal fixture was retired (its
    // hand-retimed tile payload was inverted vs the AVM all_zero skip polarity
    // and rejected by avmdec) and replaced with an avmdec/dav2d-conformant
    // luma-skip stream. Each must decode to its own pinned hash with no
    // cross-fixture state bleed.
    let cases = [
        (
            "syn-flat-intra-64x64-minimal.ivf",
            "92c4477c8b50d5646c6ed5351cbb8f4fc04517ba39354a127c306e196fd059af",
        ),
        (
            "syn-flat-intra-64x64-q80.ivf",
            "ce9c46b1078b9dd593254837ead7dcd6cee8b3ec6cc3c7d34f54fb08df703979",
        ),
    ];

    for (name, expected) in cases {
        let input = conformance_vector(name);
        let out = splot(&[
            "decode",
            "--json",
            "--output-format",
            "hash",
            input.to_str().unwrap(),
        ]);

        assert_eq!(out.status.code(), Some(0), "{name} should decode");
        let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
        assert_eq!(
            json["frames"][0]["hashes"][0]["digest_hex"], expected,
            "unexpected hash for {name}"
        );
    }
}

#[test]
fn decode_two_frame_inter_fixture_is_rejected_at_inter_frame_today() {
    // DECODE-FIRST-INTER-FRAME-FRONTIER honesty pin. The committed
    // syn-2frame-inter-64x64.ivf is the verified first-inter decode target:
    // frame 0 is an OBU_CLOSED_LOOP_KEY intra key frame, frame 1 is an
    // OBU_REGULAR_TILE_GROUP inter frame. avmdec and dav2d decode the whole
    // stream identically (frame 1 == a straight copy of frame 0).
    //
    // The multi-frame planner + runtime loop now admit the inter
    // OBU_REGULAR_TILE_GROUP and walk the frames in order, and the KEY frame now
    // decodes bit-exact: the general-intra frontier reconstructs its non-follow
    // H_PRED chroma (uv_mode == 6 over a DC luma) at the no-neighbour top-left
    // 64x64 block (verified against avmdec: the key frame is Y=100/U=120/V=130).
    // The honest rejection has therefore moved to the INTER frame itself: the
    // runtime decodes the key frame, then rejects the inter
    // OBU_REGULAR_TILE_GROUP (at its byte offset 100) with a structured
    // decode/unsupported-feature diagnostic and NO output (the inter header shared
    // tail, the inter mode info, the motion vectors, and the motion compensation
    // are not yet implemented). When the inter slice lands this flips to a
    // bit-exact two-frame decode.
    let input = conformance_vector("syn-2frame-inter-64x64.ivf");
    let output = temp_output("yuv");

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
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(json["rule_id"], "decode/unsupported-feature");
    assert_eq!(json["feature_id"], "DECODE-MINIMAL-TIER-RUNTIME-SUCCESS");
    assert_eq!(
        json["unsupported_reason"],
        "inter_frame_decode_unimplemented"
    );
    assert_eq!(json["byte_offset"], 100);
    assert!(
        !output.exists(),
        "rejected inter decode must not write an output file"
    );
}

#[test]
fn decode_hash_json_success_creates_no_implicit_output_file() {
    let input = conformance_vector("syn-flat-intra-64x64-minimal.ivf");
    let cwd = temp_dir("minimal-hash-cwd");

    let out = splot_in(
        &[
            "decode",
            "--json",
            "--output-format",
            "hash",
            input.to_str().unwrap(),
        ],
        &cwd,
    );

    assert_eq!(out.status.code(), Some(0));
    assert_eq!(read_dir_paths(&cwd), Vec::<PathBuf>::new());
}

#[test]
fn decode_hash_json_success_leaves_existing_output_path_untouched() {
    let input = conformance_vector("syn-flat-intra-64x64-minimal.ivf");
    let dir = temp_dir("minimal-hash-output");
    let output = dir.join("hash.json");
    let original_output = b"existing hash output sentinel";
    std::fs::write(&output, original_output).expect("write temporary output sentinel");
    let before_entries = read_dir_names(&dir);

    let out = splot(&[
        "decode",
        "--json",
        "--output-format",
        "hash",
        input.to_str().unwrap(),
        "-o",
        output.to_str().unwrap(),
    ]);

    assert_eq!(out.status.code(), Some(0));
    assert_eq!(
        std::fs::read(&output).expect("read temporary output sentinel"),
        original_output
    );
    assert_eq!(read_dir_names(&dir), before_entries);
}

#[test]
fn decode_hash_json_success_hashes_are_thread_deterministic() {
    let input = conformance_vector("syn-flat-intra-64x64-minimal.ivf");
    let one = decode_hash_json(&input, "1");
    let auto = decode_hash_json(&input, "auto");
    let fixed = decode_hash_json(&input, "2");

    assert_eq!(one["frames"], auto["frames"]);
    assert_eq!(one["frames"], fixed["frames"]);
    assert_eq!(
        one["selected_output_variants"],
        auto["selected_output_variants"]
    );
    assert_eq!(
        one["selected_output_variants"],
        fixed["selected_output_variants"]
    );
    assert_eq!(one["selected_thread_policy"], "1");
    assert_eq!(fixed["selected_thread_policy"], "2");
    assert!(
        auto["selected_thread_policy"]
            .as_str()
            .unwrap()
            .parse::<usize>()
            .unwrap()
            >= 1
    );
    assert_ne!(auto["selected_thread_policy"], "auto");
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
    assert!(
        stderr.lines().any(|line| line == "spec_section: "),
        "stderr did not contain an empty spec_section line: {stderr}"
    );
    assert!(
        !stderr.contains("spec_section: 5.2.1"),
        "Annex B parser issue was mis-cited to OBU syntax: {stderr}"
    );
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
    assert_eq!(json["spec_section"], "");
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
fn decode_oversized_input_reports_resource_limit_without_touching_output() {
    let input = temp_path("oversized-input", "av2");
    let max_input_bytes = default_max_input_bytes();
    let actual = max_input_bytes
        .checked_add(1)
        .expect("default max_input_bytes leaves room for sentinel byte");
    std::fs::File::create(&input)
        .expect("create sparse oversized input")
        .set_len(actual)
        .expect("size sparse oversized input");
    let output = temp_output("hashes");
    let original_output = b"oversized-input output sentinel";
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
    assert_eq!(json["spec_section"], "");
    assert_eq!(json["matrix_row"], "decode-limits-budget");
    assert_eq!(json["feature_id"], "DOC-DECODE-LIMITS-CONTRACT");
    assert_eq!(json["detail_kind"], "resource_limit");
    assert_eq!(json["limit_name"], "max_input_bytes");
    assert_eq!(json["limit"], max_input_bytes);
    assert_eq!(json["actual"], actual);
    assert_eq!(json["unit"], "bytes");
    assert_eq!(json["output_format"], "hash");
    assert_eq!(
        std::fs::read(&output).expect("read temporary output sentinel"),
        original_output
    );
}

#[test]
fn decode_invalid_output_format_is_usage_error() {
    let input = temp_input("av2", PLANABLE_CLOSED_LOOP_KEY);

    let out = splot(&[
        "decode",
        "--output-format",
        "frames",
        input.to_str().unwrap(),
    ]);

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
