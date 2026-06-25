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
const LOCAL_AC0EJ3_ENV: &str = "SPLOT_AC0EJ3_IVF";

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

fn local_ac0ej3_path() -> PathBuf {
    std::env::var_os(LOCAL_AC0EJ3_ENV)
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME")
                .map(|home| PathBuf::from(home).join("Documents/SplotLabs/ac0ej3.ivf"))
        })
        .expect("set SPLOT_AC0EJ3_IVF or HOME for the ignored local ac0ej3 regression")
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
#[ignore = "requires local mission fixture; set SPLOT_AC0EJ3_IVF or place it at $HOME/Documents/SplotLabs/ac0ej3.ivf"]
fn local_ac0ej3_reaches_current_runtime_gate_without_output() {
    let input = local_ac0ej3_path();
    assert!(
        input.is_file(),
        "local ac0ej3 fixture not found at {}; set {LOCAL_AC0EJ3_ENV}",
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
    assert_eq!(json["spec_section"], "5.20.5.3");
    assert_eq!(json["matrix_row"], "ac0ej3-selectable-transform-records");
    assert_eq!(
        json["feature_id"],
        "DECODE-AC0EJ3-SELECTABLE-TRANSFORM-RECORDS"
    );
    assert_eq!(json["detail_kind"], "unsupported_feature");
    assert_eq!(
        json["unsupported_reason"],
        "unsupported_wienerns_lr_selectable_transform_records_intrabc"
    );
    assert!(
        json["message"]
            .as_str()
            .unwrap()
            .contains("`use_intrabc` mode-info branch"),
        "diagnostic must describe the IntrABC mode-info frontier"
    );
    assert_eq!(json["byte_offset"], 110);
    assert_ne!(
        json["unsupported_reason"], "unsupported_dctonly_residual_luma_tx_type",
        "ac0ej3 must advance past the former active luma transform-type residual gate"
    );
    assert_ne!(
        json["unsupported_reason"],
        "unsupported_wienerns_lr_selectable_transform_records_unsupported_intra_tool",
        "ac0ej3 must advance past the former selectable intra-tool pre-tile gate"
    );
    assert_ne!(
        json["unsupported_reason"], "unsupported_wienerns_lr_selectable_transform_records_ccso",
        "ac0ej3 must advance past the former selectable CCSO pre-tile gate"
    );
    assert_ne!(
        json["unsupported_reason"], "unsupported_wienerns_lr_live_transform_record_uv_mode",
        "ac0ej3 must advance past the former SDP chroma uv-mode desync gate"
    );
    assert_ne!(
        json["unsupported_reason"],
        "unsupported_wienerns_lr_selectable_transform_records_block_shape",
        "ac0ej3 must advance past the luma-only narrow selectable transform-record gate"
    );
    assert_ne!(
        json["unsupported_reason"], "unsupported_wienerns_lr_live_transform_record_cfl_mode",
        "ac0ej3 must advance past the active CfL mode-info gate"
    );
    assert_ne!(
        json["unsupported_reason"], "unsupported_wienerns_lr_live_transform_record_mrl_mode",
        "ac0ej3 must advance past the former active MRL mode-info gate"
    );
    assert_ne!(
        json["unsupported_reason"], "unsupported_wienerns_lr_live_storage_unpopulated",
        "ac0ej3 must advance past the former live-storage allocation gate"
    );
    assert_ne!(
        json["unsupported_reason"], "unsupported_wienerns_lr_live_transform_record_residual_parse",
        "ac0ej3 must advance past the former transform-record residual parse gate"
    );
    assert_ne!(
        json["unsupported_reason"],
        "unsupported_wienerns_lr_selectable_transform_records_chroma_offset_leaf",
        "ac0ej3 must advance past the former chroma-offset selectable transform-record gate"
    );
    assert_ne!(
        json["unsupported_reason"],
        "unsupported_wienerns_lr_selectable_transform_records_empty_transform",
        "ac0ej3 must advance past the former empty-transform selectable transform-record gate"
    );
    assert_ne!(
        json["unsupported_reason"], "unsupported_wienerns_filter_bank",
        "ac0ej3 must advance past the parsed frame-level Wiener NS bank frontier"
    );
    assert_ne!(
        json["unsupported_reason"], "unsupported_wienerns_filter",
        "ac0ej3 must advance past the parser-only Wiener NS frontier"
    );
    assert_ne!(
        json["unsupported_reason"], "unsupported_active_wienerns_lr_units",
        "ac0ej3 must advance past the former active LR unit selection gate"
    );
    assert_ne!(
        json["unsupported_reason"], "unsupported_wienerns_lr_classified_wiener",
        "ac0ej3 must advance past the former classified-Wiener dependency gate"
    );
    assert_ne!(
        json["unsupported_reason"], "unsupported_wienerns_lr_source_bounds",
        "ac0ej3 must advance past the former source-bounds gate"
    );
    assert_ne!(
        json["unsupported_reason"], "incomplete_frame_header",
        "ac0ej3 must complete the key-frame header before runtime rejection"
    );
    assert_ne!(
        json["unsupported_reason"], "unsupported_cfl_intra",
        "ac0ej3 must advance past the former sequence CFL gate"
    );
    assert_ne!(
        json["unsupported_reason"], "unsupported_bit_depth",
        "ac0ej3 must advance past the former sequence bit-depth gate"
    );
    assert_ne!(
        json["unsupported_reason"], "unsupported_frame_candidate_count",
        "ac0ej3 must advance past the former total frame-count gate"
    );
    assert_ne!(
        json["unsupported_reason"], "unexpected_obu_order",
        "ac0ej3 must advance past the former leading CLK-plus-tile-group framing gate"
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
fn decode_two_frame_inter_fixture_decodes_both_frames_bit_exact() {
    // DECODE-FIRST-INTER-FRAME-FRONTIER: the committed syn-2frame-inter-64x64.ivf is
    // the verified first-inter decode target. Frame 0 is an OBU_CLOSED_LOOP_KEY intra
    // key frame; frame 1 is an OBU_REGULAR_TILE_GROUP inter frame (single reference,
    // is_inter == 1, skip == 1, the single-reference zero-MV NEARMV mode, no
    // residual, so §7.13.3.18 zero-fraction motion compensation reduces to a straight
    // copy of the co-located key block). avmdec --rawvideo --i420 and
    // dav2d --demuxer ivf decode the whole stream byte-for-byte identically:
    // decoded-output md5 4e1bd39f0b541ef1f479cff049e6985c over 12288 bytes (two flat
    // 64x64 4:2:0 frames; frame 1 == a copy of frame 0). The runtime now decodes both
    // frames: the key frame via the general-intra frontier, then the inter frame via
    // the new inter frontier (real §5.18.2 header parse, §5.20 inter mode_info symbol
    // reads, §7.11 zero-MV derivation, §7.13.3.18 copy, validated by §8.2.4
    // exit_symbol()). This replaces the prior inter_frame_decode_unimplemented reject.
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
    // 2 frames * 64x64 luma + 2 * 32x32 (U + V) = 2 * (4096 + 2 * 1024) = 12288 bytes.
    assert_eq!(decoded.len(), 12288, "two flat 8-bit 4:2:0 64x64 frames");
    // avmdec / dav2d oracle: both frames are flat (Y=100, U=120, V=130), and frame 1
    // is a byte-exact copy of frame 0 (the zero-MV motion-compensation copy).
    let frame_bytes = 6144;
    let (frame0, frame1) = decoded.split_at(frame_bytes);
    assert_eq!(frame0, frame1, "inter frame 1 is a copy of key frame 0");
    assert!(frame0[..4096].iter().all(|&s| s == 100), "luma flat 100");
    assert!(frame0[4096..5120].iter().all(|&s| s == 120), "U flat 120");
    assert!(frame0[5120..].iter().all(|&s| s == 130), "V flat 130");
}

#[test]
fn decode_two_frame_inter_residual_fixture_decodes_bit_exact() {
    // DECODE-INTER-RESIDUAL-DCT: the committed syn-2frame-inter-residual-64x64.ivf is
    // the verified first inter-residual (skip == 0) target. Frame 0 is an
    // OBU_CLOSED_LOOP_KEY flat-100 intra key frame; frame 1 is an
    // OBU_REGULAR_TILE_GROUP inter frame (single reference, is_inter == 1, skip == 0,
    // zero-MV NEARMV, a §5.20.7.27 luma DCT_DCT residual added over the §7.13.3.18
    // zero-fraction copy of frame 0; flat chroma carries no residual). avmdec
    // --rawvideo --i420 and dav2d --demuxer ivf decode the whole stream byte-for-byte
    // identically (decoded-output md5 ab2b067aed48cf46035fa031cefb3ab1 over 12288
    // bytes). The runtime decodes the residual coefficients (§5.20.7.27 with the
    // is_inter txb_skip / eob contexts), dequantizes (§7.14.4), inverse-transforms
    // (§7.15.4), and adds the residual (§7.14.3) over the MC prediction, all guarded
    // by §8.2.4 exit_symbol() (NO hardcoding; a wrong residual symbol read rejects).
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
    // Frame 0 (key) is flat per the oracle; frame 1's luma differs (a real residual)
    // while its chroma stays flat (luma-only residual).
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
    // DECODE-INTER-MVSTACK-SPATIAL: the committed syn-2frame-inter-mvstack-64x64.ivf
    // is the verified first MULTI-BLOCK neighbour-predicted-MV target. Frame 0 is a
    // general-intra DC_PRED key frame; frame 1 is an OBU_REGULAR_TILE_GROUP inter
    // frame whose 64x64 superblock is §5.20.3 SPLIT into four 32x32 single-reference
    // inter blocks: block 0 @ MI(0,0) is NEWMV with a non-zero MV (col 48 = +6 full
    // pels) and the later three blocks are NEARMV that predict block 0's MV from the
    // §7.11/§7.12 spatial-neighbour MV stack (find_mv_stack); all skip=1. avmdec
    // --rawvideo --i420 and dav2d --demuxer ivf decode the whole stream byte-for-byte
    // identically (decoded-output md5 e5b581a55433785c0071b635d5642083 over 12288
    // bytes). The OLD single-block inter decoder rejected this fixture. NO step is
    // hardcoded: the §8.2.4 exit_symbol() check guards bit-exactness, so a wrong
    // mode / DRL / MV / context read rejects rather than emitting a wrong frame.
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
    // Frame 1's luma differs from frame 0 (the +6-pel horizontal motion shifts the
    // quadrant content), proving the inter MVs are genuinely applied (not a copy);
    // the chroma is flat and unchanged.
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
    // DECODE-INTER-MULTI-SB-SPATIAL: the committed syn-2sb-inter-128x64-q80.ivf is the
    // verified first MULTI-SUPERBLOCK inter target. The 128x64 frame is two
    // horizontally-adjacent 64x64 superblocks. Frame 0 is a general-intra DC_PRED key
    // frame (left SB flat 100, right SB flat 150); frame 1 is an OBU_REGULAR_TILE_GROUP
    // inter frame whose two superblocks are each a single 64x64 inter block: SB0 @
    // MI(0,0) is NEWMV with a non-zero MV (col 48 = +6 full pels), and SB1 @ MI(0,16)
    // — in the SECOND superblock — is NEARMV that predicts SB0's MV across the
    // superblock boundary from the frame-wide §7.11/§7.12 spatial-neighbour MV stack
    // (find_mv_stack); both skip=1. avmdec --rawvideo --i420 and dav2d --demuxer ivf
    // decode the whole stream byte-for-byte identically (decoded-output md5
    // 477a993d671e93d37b92a0d368c238ff over 24576 bytes). The OLD single-64x64 inter
    // decoder rejected this fixture ("currently accepts only the verified 64x64 frame
    // size"). NO step is hardcoded: the §8.2.4 exit_symbol() check guards
    // bit-exactness.
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
    // Frame 1's luma differs from frame 0 (the +6-pel horizontal motion shifts the
    // 100/150 superblock content left across the superblock boundary), proving the
    // cross-superblock neighbour-predicted MVs are genuinely applied; chroma is flat
    // and unchanged.
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
fn decode_grid_inter_fixture_decodes_bit_exact() {
    // DECODE-INTER-GRID-SPATIAL: the committed syn-grid-inter-128x128-q80.ivf is the
    // verified first 2-D-GRID inter target. The 128x128 frame is a 2x2 grid of 64x64
    // superblocks. Frame 0 is a general-intra DC_PRED key frame (four flat 64x64
    // superblocks 100/150/80/200); frame 1 is an OBU_REGULAR_TILE_GROUP inter frame
    // whose four superblocks are each a single 64x64 inter block, all skip=1: SB0 @
    // MI(0,0) is NEWMV with a non-zero MV (col 48 = +6 full pels), and SB1 @ MI(0,16),
    // SB2 @ MI(16,0), SB3 @ MI(16,16) are NEARMV that predict SB0's MV via the
    // frame-wide §7.11/§7.12 spatial-neighbour MV stack — SB2 and SB3 (in the SECOND
    // superblock ROW) predict across the SB-ROW boundary, the case the single-SB-row
    // brick deferred. avmdec --rawvideo --i420 and dav2d --demuxer ivf decode the whole
    // stream byte-for-byte identically (decoded-output md5
    // 897bf67e72ec04cb7275fae08eab700c over 49152 bytes). The single-SB-row inter
    // decoder rejected this fixture (inter_unsupported_frame_size). NO step is
    // hardcoded: the §8.2.4 exit_symbol() check guards bit-exactness.
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
    let frame_bytes = 24576;
    let (frame0, frame1) = decoded.split_at(frame_bytes);
    let luma_bytes = 16384;
    // Frame 1's luma differs from frame 0 (the +6-pel horizontal motion shifts the
    // superblock content left across the superblock boundaries), proving the
    // cross-superblock neighbour-predicted MVs are genuinely applied; chroma is flat
    // and unchanged.
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
fn decode_distinct_mv_inter_fixture_decodes_bit_exact() {
    // DECODE-INTER-MVORDER-SPATIAL: the committed syn-2frame-inter-mvorder-64x64.ivf
    // CLOSES the verified-subset honesty gap left by the identical-MV inter fixtures
    // (mvstack / multi-SB / grid all propagated ONE col-48 MV, so the §7.12.2 stack
    // collapsed and the per-neighbour ORDERING was exercised-but-not-discriminated).
    // Frame 0 is a general-intra DC_PRED key frame (four flat 32x32 quadrants
    // 100/150/60/200); frame 1 is an OBU_REGULAR_TILE_GROUP inter frame whose 64x64
    // superblock is §5.20.3 SPLIT into four 32x32 single-reference inter blocks, all
    // skip=1, each carrying a DISTINCT MV: block 0 @ MI(0,0) NEWMV col 64, block 1 @
    // MI(0,8) NEWMV col -32, block 2 @ MI(8,0) NEWMV col 32, and the INTERIOR block 3
    // @ MI(8,8) NEARMV RefMvIdx 1 over a stack whose slot 0 is the LEFT neighbour
    // (col 32) and slot 1 is the ABOVE neighbour (col -32), reconstructing col -32 —
    // pinning the §7.12.2 left-before-above ordering and the §5.20.7.8 DRL slot-1
    // selection (a reversed order would reconstruct from col 32 and mismatch). Every
    // leaf is 32x32 (not > 32), so the §7.12.2.20 large-block MVP step is
    // inapplicable. avmdec --rawvideo --i420 and dav2d --demuxer ivf decode the whole
    // stream byte-for-byte identically (decoded-output md5
    // 284e1450b42180f02de7415ab0367bfe over 12288 bytes). NO step is hardcoded: the
    // §8.2.4 exit_symbol() check guards bit-exactness.
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
    // Frame 1's luma differs from frame 0 (the distinct per-quadrant horizontal
    // motion shifts the quadrant content), proving the distinct neighbour-predicted
    // MVs are genuinely applied; the chroma is flat 128 and unchanged.
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
    // DECODE-INTER-MULTIREF-RUNTIME: the committed syn-3frame-multiref-64x64.ivf is the
    // verified multi-reference target. Frame 0 is an OBU_CLOSED_LOOP_KEY flat intra key
    // (luma 100); frame 1 is an OBU_REGULAR_TILE_GROUP single-reference inter block
    // (§7.7 NumTotalRefs == 1, the key) reconstructing luma 160 and refreshing a SECOND
    // reference slot; frame 2 is an OBU_REGULAR_TILE_GROUP inter block over TWO valid
    // references (§7.7 ref_frame_idx [0, 1]) whose §5.20.7.12 single_ref selects slot 1
    // (the RETAINED frame 1, luma 160), NOT the key (luma 100). Encoded with
    // --cdf-update-mode=0 so no CDF adaptation propagates. avmdec --rawvideo --i420 and
    // dav2d --demuxer ivf --muxer yuv decode the whole stream byte-for-byte identically
    // (decoded-output md5 861078138ab514bd847ccfe22ac44fa1 over 18432 bytes: three flat
    // 8-bit 4:2:0 64x64 frames).
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
    // 3 frames * (64x64 luma + 2 * 32x32 chroma) = 3 * 6144 = 18432 bytes.
    assert_eq!(decoded.len(), 18432, "three flat 8-bit 4:2:0 64x64 frames");
    let frame_bytes = 6144;
    let frame0 = &decoded[..frame_bytes];
    let frame1 = &decoded[frame_bytes..2 * frame_bytes];
    let frame2 = &decoded[2 * frame_bytes..];
    // Frame 0 (key) is flat luma 100; frames 1 and 2 are flat luma 160.
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
    // ASYMMETRIC PROOF: frame 2 reads the retained frame 1 (slot 1, luma 160), NOT the
    // key (slot 0, luma 100). Frame 2 == frame 1 and frame 2 != frame 0.
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
    // DECODE-INTER-COMPOUND-AVERAGE: the committed
    // syn-3frame-compound-average-64x64.ivf is the verified two-reference
    // COMPOUND_AVERAGE target. Frame 0 is a general-intra low-frequency key frame,
    // frame 1 is a single-reference NEWMV inter frame, and frame 2 is a
    // `reference_select` compound block over refs [0, 1] with non-joint NEAR_NEARMV,
    // zero MVs, skip=1, COMPOUND_AVERAGE, and CWP/masks disabled. avmdec
    // --rawvideo --i420 and dav2d --demuxer ivf --muxer yuv decode the whole stream
    // byte-for-byte identically (raw SHA-256
    // 2b4f716243d9f5c30a244ecc6f7fdcb5bef804d2ba353a21d670d686cfe63ff4; raw MD5
    // 34074c6945348b146f84551a20d9affd).
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
    // 3 frames * (64x64 luma + 2 * 32x32 chroma) = 3 * 6144 = 18432 bytes.
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
    assert_eq!(json["matrix_row"], "decode-limits-budget");
    assert_eq!(json["feature_id"], "DOC-DECODE-LIMITS-CONTRACT");
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
