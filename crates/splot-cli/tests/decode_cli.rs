// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! End-to-end `splot decode` CLI contract tests.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};
use std::process::Output;
use std::sync::atomic::{AtomicUsize, Ordering};

use splot_decode::DecodeOptions;

mod common;
use common::read_dir_names;

static TEMP_COUNTER: AtomicUsize = AtomicUsize::new(0);

const PLANABLE_CLOSED_LOOP_KEY: &[u8] = &[0x01, 0x10];
const UNSUPPORTED_OPEN_LOOP_KEY: &[u8] = &[0x01, 0x14];
const MALFORMED_ANNEX_B: &[u8] = &[0x05, 0x10];
const LOCAL_DECODER_MISSION_ENV: &str = "SPLOT_LOCAL_DECODER_MISSION_IVF";

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

fn local_decoder_mission_path() -> PathBuf {
    std::env::var_os(LOCAL_DECODER_MISSION_ENV)
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME")
                .map(|home| PathBuf::from(home).join("Documents/SplotLabs/local-decoder-mission.ivf"))
        })
        .expect("set SPLOT_LOCAL_DECODER_MISSION_IVF or HOME for the ignored local decoder mission regression")
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

fn default_max_obus() -> u64 {
    DecodeOptions::default()
        .limits()
        .max_obus()
        .max_value()
        .expect("default max_obus is finite")
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
        "detail_kind: unsupported_feature".to_string(),
        "unsupported_reason: unexpected_planned_stream_shape".to_string(),
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
    assert_eq!(json["detail_kind"], "unsupported_feature");
    assert_eq!(
        json["unsupported_reason"],
        "unexpected_planned_stream_shape"
    );
    assert_eq!(json["output_format"], "hash");
}

#[test]
#[ignore = "requires local mission fixture; set SPLOT_LOCAL_DECODER_MISSION_IVF or place it at $HOME/Documents/SplotLabs/local-decoder-mission.ivf"]
fn local_decoder_mission_reaches_current_runtime_gate_without_output() {
    let input = local_decoder_mission_path();
    assert!(
        input.is_file(),
        "local decoder mission fixture not found at {}; set {LOCAL_DECODER_MISSION_ENV}",
        input.display()
    );

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
    assert_eq!(json["detail_kind"], "unsupported_feature");
    assert_eq!(json["unsupported_reason"], "inter_unsupported_frame_tools");
    assert_eq!(
        json["byte_offset"], 12431,
        "the frontier holds at coded frame 3's header (temporal MVs / use_ref_frame_mvs); \
         the warp family, BAWP, and display-order output scheduling are admitted, coded \
         frame 2 parses end-to-end, and output frame 0 is byte-identical to AVM"
    );
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
    assert_eq!(
        frame["hashes"][0]["digest_hex"],
        "92c4477c8b50d5646c6ed5351cbb8f4fc04517ba39354a127c306e196fd059af"
    );
}

#[test]
fn decode_general_intra_fixture_reconstructs_full_frame() {
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
fn decode_two_frame_inter_fixture_decodes_both_frames_bit_exact() {
    let input = conformance_vector("syn-2frame-inter-64x64.ivf");
    let output = temp_output("yuv");

    let out = splot(&[
        "decode",
        "--output-format",
        "raw",
        input.to_str().unwrap(),
        "-o",
        output.to_str().unwrap(),
    ]);

    assert_eq!(
        out.status.code(),
        Some(0),
        "the two-frame inter fixture must decode successfully: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let decoded = std::fs::read(&output).expect("decoded raw output");
    assert_eq!(decoded.len(), 12288, "two flat 8-bit 4:2:0 64x64 frames");
    let frame_bytes = 6144;
    let (frame0, frame1) = decoded.split_at(frame_bytes);
    assert_eq!(frame0, frame1, "inter frame 1 is a copy of key frame 0");
    assert!(frame0[..4096].iter().all(|&s| s == 100), "luma flat 100");
    assert!(frame0[4096..5120].iter().all(|&s| s == 120), "U flat 120");
    assert!(frame0[5120..].iter().all(|&s| s == 130), "V flat 130");
}

#[test]
fn decode_two_frame_inter_residual_fixture_decodes_bit_exact() {
    let input = conformance_vector("syn-2frame-inter-residual-64x64.ivf");
    let output = temp_output("yuv");

    let out = splot(&[
        "decode",
        "--output-format",
        "raw",
        input.to_str().unwrap(),
        "-o",
        output.to_str().unwrap(),
    ]);

    assert_eq!(
        out.status.code(),
        Some(0),
        "the inter-residual fixture must decode successfully: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let decoded = std::fs::read(&output).expect("decoded raw output");
    assert_eq!(decoded.len(), 12288, "two 8-bit 4:2:0 64x64 frames");
    let frame_bytes = 6144;
    let (frame0, frame1) = decoded.split_at(frame_bytes);
    assert!(
        frame0[..4096].iter().all(|&s| s == 100),
        "key luma flat 100"
    );
    assert_ne!(
        &frame0[..4096],
        &frame1[..4096],
        "inter luma differs from the key (real residual, not a copy)"
    );
    assert_eq!(
        &frame0[4096..],
        &frame1[4096..],
        "inter chroma equals key chroma (no chroma residual)"
    );
    assert!(
        frame1[4096..5120].iter().all(|&s| s == 120),
        "inter U flat 120"
    );
    assert!(frame1[5120..].iter().all(|&s| s == 130), "inter V flat 130");
}

#[test]
fn decode_two_frame_inter_mvstack_fixture_decodes_bit_exact() {
    let input = conformance_vector("syn-2frame-inter-mvstack-64x64.ivf");
    let output = temp_output("yuv");

    let out = splot(&[
        "decode",
        "--output-format",
        "raw",
        input.to_str().unwrap(),
        "-o",
        output.to_str().unwrap(),
    ]);

    assert_eq!(
        out.status.code(),
        Some(0),
        "the multi-block inter fixture must decode successfully: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let decoded = std::fs::read(&output).expect("decoded raw output");
    assert_eq!(decoded.len(), 12288, "two 8-bit 4:2:0 64x64 frames");
    let frame_bytes = 6144;
    let (frame0, frame1) = decoded.split_at(frame_bytes);
    assert_ne!(
        &frame0[..4096],
        &frame1[..4096],
        "inter luma differs from the key (real neighbour-predicted MVs)"
    );
    assert_eq!(
        &frame0[4096..],
        &frame1[4096..],
        "inter chroma equals key chroma (flat, MV is horizontal)"
    );
}

#[test]
fn decode_multi_sb_inter_fixture_decodes_bit_exact() {
    let input = conformance_vector("syn-2sb-inter-128x64-q80.ivf");
    let output = temp_output("yuv");

    let out = splot(&[
        "decode",
        "--output-format",
        "raw",
        input.to_str().unwrap(),
        "-o",
        output.to_str().unwrap(),
    ]);

    assert_eq!(
        out.status.code(),
        Some(0),
        "the multi-superblock inter fixture must decode successfully: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let decoded = std::fs::read(&output).expect("decoded raw output");
    assert_eq!(decoded.len(), 24576, "two 8-bit 4:2:0 128x64 frames");
    let frame_bytes = 12288;
    let (frame0, frame1) = decoded.split_at(frame_bytes);
    let luma_bytes = 8192;
    assert_ne!(
        &frame0[..luma_bytes],
        &frame1[..luma_bytes],
        "inter luma differs from the key (real cross-SB neighbour-predicted MVs)"
    );
    assert_eq!(
        &frame0[luma_bytes..],
        &frame1[luma_bytes..],
        "inter chroma equals key chroma (flat, MV is horizontal)"
    );
}

#[test]
fn decode_grid_inter_fixture_decodes_both_frames() {
    let input = conformance_vector("syn-grid-inter-128x128-q80.ivf");
    let output = temp_output("yuv");

    let out = splot(&[
        "decode",
        "--output-format",
        "raw",
        input.to_str().unwrap(),
        "-o",
        output.to_str().unwrap(),
    ]);

    assert_eq!(
        out.status.code(),
        Some(0),
        "the 2-D-grid inter fixture must decode successfully: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let decoded = std::fs::read(&output).expect("decoded raw output");
    assert_eq!(decoded.len(), 49152, "two 8-bit 4:2:0 128x128 frames");
}

#[test]
fn decode_distinct_mv_inter_fixture_decodes_bit_exact() {
    let input = conformance_vector("syn-2frame-inter-mvorder-64x64.ivf");
    let output = temp_output("yuv");

    let out = splot(&[
        "decode",
        "--output-format",
        "raw",
        input.to_str().unwrap(),
        "-o",
        output.to_str().unwrap(),
    ]);

    assert_eq!(
        out.status.code(),
        Some(0),
        "the distinct-MV inter fixture must decode successfully: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let decoded = std::fs::read(&output).expect("decoded raw output");
    assert_eq!(decoded.len(), 12288, "two 8-bit 4:2:0 64x64 frames");
    let frame_bytes = 6144;
    let (frame0, frame1) = decoded.split_at(frame_bytes);
    assert_ne!(
        &frame0[..4096],
        &frame1[..4096],
        "inter luma differs from the key (distinct neighbour-predicted MVs)"
    );
    assert_eq!(
        &frame0[4096..],
        &frame1[4096..],
        "inter chroma equals key chroma (flat, MVs are horizontal)"
    );
}

#[test]
fn decode_multiref_three_frame_fixture_is_bit_exact() {
    let input = conformance_vector("syn-3frame-multiref-64x64.ivf");
    let output = temp_output("yuv");

    let out = splot(&[
        "decode",
        "--output-format",
        "raw",
        input.to_str().unwrap(),
        "-o",
        output.to_str().unwrap(),
    ]);

    assert_eq!(
        out.status.code(),
        Some(0),
        "the multi-reference 3-frame fixture must decode successfully: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let decoded = std::fs::read(&output).expect("decoded raw output");
    assert_eq!(decoded.len(), 18432, "three flat 8-bit 4:2:0 64x64 frames");
    let frame_bytes = 6144;
    let frame0 = &decoded[..frame_bytes];
    let frame1 = &decoded[frame_bytes..2 * frame_bytes];
    let frame2 = &decoded[2 * frame_bytes..];
    assert!(
        frame0[..4096].iter().all(|&s| s == 100),
        "key luma flat 100"
    );
    assert!(
        frame1[..4096].iter().all(|&s| s == 160),
        "frame 1 luma flat 160"
    );
    assert!(
        frame2[..4096].iter().all(|&s| s == 160),
        "frame 2 luma flat 160"
    );
    assert_eq!(
        frame2, frame1,
        "frame 2 reads the retained inter frame (slot 1)"
    );
    assert_ne!(
        frame2, frame0,
        "frame 2 must DIFFER from the key (proving single_ref selected slot 1, not slot 0)"
    );
}

#[test]
fn decode_compound_average_three_frame_fixture_is_bit_exact() {
    let input = conformance_vector("syn-3frame-compound-average-64x64.ivf");
    let output = temp_output("yuv");

    let out = splot(&[
        "decode",
        "--output-format",
        "raw",
        input.to_str().unwrap(),
        "-o",
        output.to_str().unwrap(),
    ]);

    assert_eq!(
        out.status.code(),
        Some(0),
        "the compound-average 3-frame fixture must decode successfully: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let decoded = std::fs::read(&output).expect("decoded raw output");
    assert_eq!(decoded.len(), 18432, "three 8-bit 4:2:0 64x64 frames");
    let frame_bytes = 6144;
    let frame0 = &decoded[..frame_bytes];
    let frame1 = &decoded[frame_bytes..2 * frame_bytes];
    let frame2 = &decoded[2 * frame_bytes..];
    assert_rounded_average(frame0, frame1, frame2);
    assert_ne!(frame2, frame0, "compound frame differs from ref 0");
    assert_ne!(frame2, frame1, "compound frame differs from ref 1");
}

fn assert_rounded_average(ref0: &[u8], ref1: &[u8], compound: &[u8]) {
    assert_eq!(ref0.len(), ref1.len(), "reference frame lengths");
    assert_eq!(ref0.len(), compound.len(), "compound frame length");
    for (index, ((&a, &b), &actual)) in ref0
        .iter()
        .zip(ref1.iter())
        .zip(compound.iter())
        .enumerate()
    {
        let expected = ((u16::from(a) + u16::from(b) + 1) >> 1) as u8;
        assert_eq!(
            actual, expected,
            "compound raw sample {index}: rounded average of refs"
        );
    }
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
    let max_obus = default_max_obus();
    let limit_input = repeated_sequence_header_obus((max_obus + 1).try_into().unwrap());
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
    assert_eq!(json["detail_kind"], "resource_limit");
    assert_eq!(json["limit_name"], "max_obus");
    assert_eq!(json["limit"], serde_json::json!(max_obus));
    assert_eq!(json["actual"], serde_json::json!(max_obus + 1));
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

/// Raw-frame geometry for the AVM compare harness, derived from the IVF header.
/// Ceiling: assumes 4:2:0 with two-byte samples (upgrade: read the sequence
/// header bit depth once an 8-bit mission stream needs this harness).
struct RawFrameGeometry {
    width: usize,
    height: usize,
}

impl RawFrameGeometry {
    fn from_ivf_header(input: &Path) -> Option<Self> {
        let mut header = [0u8; 32];
        let mut file = std::fs::File::open(input).ok()?;
        std::io::Read::read_exact(&mut file, &mut header).ok()?;
        let width = usize::from(u16::from_le_bytes([header[12], header[13]]));
        let height = usize::from(u16::from_le_bytes([header[14], header[15]]));
        (width > 0 && height > 0).then_some(Self { width, height })
    }

    fn frame_bytes(&self) -> usize {
        self.width * self.height * 3
    }

    fn locate(&self, offset_in_frame: usize) -> (char, usize, usize) {
        let y_bytes = self.width * self.height * 2;
        let u_bytes = y_bytes / 4;
        let (plane, base, row_width) = if offset_in_frame < y_bytes {
            ('Y', 0, self.width)
        } else if offset_in_frame < y_bytes + u_bytes {
            ('U', y_bytes, self.width / 2)
        } else {
            ('V', y_bytes + u_bytes, self.width / 2)
        };
        let sample = (offset_in_frame - base) / 2;
        (plane, sample % row_width, sample / row_width)
    }
}

fn per_frame_digest_line(index: usize, frame: &[u8]) -> String {
    use sha2::Digest as _;
    use std::fmt::Write as _;

    let mut line = format!("{index:06} ");
    for byte in sha2::Sha256::digest(frame) {
        let _ = write!(line, "{byte:02x}");
    }
    line.push('\n');
    line
}

/// The §10.1 mission harness: byte-compares splot raw decode output against the
/// pinned AVM oracle (`avmdec --i420 --rawvideo`) and reports the first
/// mismatching frame/plane/sample plus per-frame digest lists.
#[test]
#[ignore = "local mission harness; needs SPLOT_LOCAL_DECODER_MISSION_IVF (or the default fixture path) and an avmdec build (SPLOT_AVM_DECODER)"]
fn local_decoder_mission_full_stream_avm_compare() {
    let input = local_decoder_mission_path();
    if !input.is_file() {
        eprintln!(
            "skip: local decoder mission fixture missing at {}",
            input.display()
        );
        return;
    }
    let avmdec = std::env::var_os("SPLOT_AVM_DECODER")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME")
                .map(|home| PathBuf::from(home).join("Devel/avm/build_inspect/avmdec"))
        })
        .expect("set SPLOT_AVM_DECODER or HOME");
    if !avmdec.is_file() {
        eprintln!("skip: avmdec not found at {}", avmdec.display());
        return;
    }
    let geometry = RawFrameGeometry::from_ivf_header(&input).expect("readable IVF header");
    let work = std::env::var_os("SPLOT_LOCAL_DECODER_MISSION_WORK")
        .map_or_else(std::env::temp_dir, PathBuf::from)
        .join("splot-local-decoder-mission-fullstream");
    std::fs::create_dir_all(&work).expect("create harness work dir");
    let limit = std::env::var("SPLOT_LOCAL_DECODER_MISSION_LIMIT").ok();
    let tag = limit.as_deref().unwrap_or("full");

    let avm_out = work.join(format!("avm-{tag}.yuv"));
    if !avm_out.is_file() {
        let mut command = std::process::Command::new(&avmdec);
        command.arg(&input).args(["--i420", "--rawvideo", "-o"]);
        command.arg(&avm_out);
        if let Some(limit) = limit.as_deref() {
            command.arg(format!("--limit={limit}"));
        }
        let status = command
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .expect("run avmdec");
        assert!(
            status.success(),
            "avmdec failed; delete {avm_out:?} to retry"
        );
    }

    let splot_out = work.join(format!("splot-{tag}.yuv"));
    let mut args = vec!["decode", "--output-format", "raw", "-o"];
    let splot_out_text = splot_out.to_str().expect("utf-8 work path").to_owned();
    args.push(&splot_out_text);
    let limit_arg = limit.as_deref().map(|limit| format!("--limit={limit}"));
    if let Some(limit_arg) = limit_arg.as_deref() {
        args.push(limit_arg);
    }
    let input_text = input.to_str().expect("utf-8 fixture path").to_owned();
    args.push(&input_text);
    let out = splot(&args);
    assert_eq!(
        out.status.code(),
        Some(0),
        "splot decode failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let frame_bytes = geometry.frame_bytes();
    let mut avm = std::fs::File::open(&avm_out).expect("open AVM output");
    let mut splot_file = std::fs::File::open(&splot_out).expect("open splot output");
    let mut avm_frame = vec![0u8; frame_bytes];
    let mut splot_frame = vec![0u8; frame_bytes];
    let mut avm_digests = String::new();
    let mut splot_digests = String::new();
    let mut frame = 0usize;
    let mut first_mismatch: Option<(usize, usize)> = None;
    loop {
        let avm_read = read_full_frame(&mut avm, &mut avm_frame);
        let splot_read = read_full_frame(&mut splot_file, &mut splot_frame);
        assert_eq!(
            avm_read % frame_bytes,
            0,
            "AVM output truncated mid-frame at frame {frame}"
        );
        assert_eq!(
            splot_read % frame_bytes,
            0,
            "splot output truncated mid-frame at frame {frame}"
        );
        if avm_read == 0 || splot_read == 0 {
            assert_eq!(
                avm_read, splot_read,
                "frame-count divergence at frame {frame}: one stream ended first \
                 (first byte mismatch so far: {first_mismatch:?})"
            );
            break;
        }
        avm_digests.push_str(&per_frame_digest_line(frame, &avm_frame));
        splot_digests.push_str(&per_frame_digest_line(frame, &splot_frame));
        if first_mismatch.is_none()
            && let Some(offset) = avm_frame
                .iter()
                .zip(&splot_frame)
                .position(|(avm_byte, splot_byte)| avm_byte != splot_byte)
        {
            first_mismatch = Some((frame, offset));
        }
        frame += 1;
    }
    std::fs::write(work.join(format!("avm-{tag}.frames.sha256")), &avm_digests)
        .expect("write AVM digest list");
    std::fs::write(
        work.join(format!("splot-{tag}.frames.sha256")),
        &splot_digests,
    )
    .expect("write splot digest list");

    if let Some((frame, offset)) = first_mismatch {
        let (plane, x, y) = geometry.locate(offset);
        panic!(
            "first mismatch: frame {frame} plane {plane} x={x} y={y} \
             (byte {offset} in frame; digest lists under {})",
            work.display()
        );
    }
    eprintln!("byte-identical: {frame} frames of {frame_bytes} bytes");
}

fn read_full_frame(file: &mut std::fs::File, buf: &mut [u8]) -> usize {
    use std::io::Read as _;

    let mut filled = 0usize;
    while filled < buf.len() {
        let read = file.read(&mut buf[filled..]).expect("read raw stream");
        if read == 0 {
            break;
        }
        filled += read;
    }
    filled
}
