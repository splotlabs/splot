// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! End-to-end `splot decode` raw output tests.

#![expect(clippy::expect_used)]

mod common;
use common::{
    conformance_vector, empty_avmenc_ivf, read_dir_names, splot, temp_dir, temp_input, temp_output,
};

const PLANABLE_CLOSED_LOOP_KEY: &[u8] = &[0x01, 0x10];

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
fn decode_raw_skip_film_grain_leaves_the_grain_out() {
    let input = conformance_vector("syn-filmgrain-intra-64x64.ivf");
    let decode = |flags: &[&str]| {
        let output = temp_output("raw");
        let mut args = vec!["decode", "--output-format", "raw", input.to_str().unwrap()];
        args.extend_from_slice(flags);
        args.extend_from_slice(&["-o", output.to_str().unwrap()]);
        let out = splot(&args);
        assert_eq!(out.status.code(), Some(0));
        std::fs::read(&output).unwrap()
    };
    let context =
        splot_decode::DecodeContext::new(splot_decode::DecodeRuntimeConfig::default()).unwrap();
    let mut expected = Vec::new();
    context
        .decode_raw_bytes(
            &std::fs::read(&input).unwrap(),
            splot_decode::DecodeOptions::default().with_skip_film_grain(true),
            &mut expected,
        )
        .unwrap();

    let skipped = decode(&["--skip-film-grain"]);
    assert_eq!(skipped, expected);
    assert_ne!(skipped, decode(&[]));
}

#[cfg(unix)]
#[test]
fn decode_raw_writes_to_null_device() {
    let input = conformance_vector("syn-flat-intra-64x64-minimal.ivf");

    let out = splot(&[
        "decode",
        "--output-format",
        "raw",
        input.to_str().unwrap(),
        "-o",
        "/dev/null",
    ]);

    assert_eq!(out.status.code(), Some(0));
    assert!(out.stdout.is_empty(), "stdout was not empty");
    assert!(out.stderr.is_empty(), "stderr was not empty");
}

#[test]
fn decode_empty_ivf_creates_empty_raw_output() {
    let input = temp_input("ivf", &empty_avmenc_ivf());
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
    assert_eq!(std::fs::read(&output).unwrap(), Vec::<u8>::new());
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
        "detail_kind: unsupported_feature",
        "unsupported_reason: unexpected_obu_order",
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
    assert_eq!(json["detail_kind"], "unsupported_feature");
    assert_eq!(json["unsupported_reason"], "unexpected_obu_order");
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
fn decode_raw_outputs_are_frame_delay_deterministic() {
    let input = conformance_vector("syn-flat-intra-64x64-minimal.ivf");
    let decode = |frame_delay| {
        let output = temp_output("raw");
        let out = splot(&[
            "decode",
            "--output-format",
            "raw",
            "--threads",
            "4",
            "--frame-delay",
            frame_delay,
            input.to_str().unwrap(),
            "-o",
            output.to_str().unwrap(),
        ]);
        assert_eq!(out.status.code(), Some(0), "frame_delay={frame_delay}");
        assert!(
            out.stdout.is_empty(),
            "stdout was not empty for {frame_delay}"
        );
        assert!(
            out.stderr.is_empty(),
            "stderr was not empty for {frame_delay}"
        );
        std::fs::read(&output).unwrap()
    };
    let expected = expected_minimal_raw();

    assert_eq!(decode("1"), expected);
    assert_eq!(decode("2"), expected);
    assert_eq!(decode("auto"), expected);
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

#[test]
fn decode_temporal_layer_and_output_mask_flags_match_library() {
    let context =
        splot_decode::DecodeContext::new(splot_decode::DecodeRuntimeConfig::default()).unwrap();
    let dep8 = conformance_vector("syn-tlayer2-dependent-8bit.ivf");
    let dep8_bytes = std::fs::read(&dep8).unwrap();
    for (flag_val, selection) in [
        ("0", splot_decode::TemporalLayerSelection::BASE_ONLY),
        ("all", splot_decode::TemporalLayerSelection::All),
    ] {
        let output = temp_output("raw");
        let out = splot(&[
            "decode",
            "--output-format",
            "raw",
            "--temporal-layer",
            flag_val,
            dep8.to_str().unwrap(),
            "-o",
            output.to_str().unwrap(),
        ]);
        assert_eq!(out.status.code(), Some(0));
        let mut expected = Vec::new();
        context
            .decode_raw_bytes(
                &dep8_bytes,
                splot_decode::DecodeOptions::default().with_temporal_layer_selection(selection),
                &mut expected,
            )
            .unwrap();
        assert_eq!(std::fs::read(&output).unwrap(), expected);
    }

    let sparse = conformance_vector("syn-tlayer4-sparse-dependencies.obu");
    let sparse_bytes = std::fs::read(&sparse).unwrap();
    for (mask_arg, mask) in [("4", 4u8), ("0x8", 8u8)] {
        let output = temp_output("raw");
        let out = splot(&[
            "decode",
            "--output-format",
            "raw",
            "--temporal-output-mask",
            mask_arg,
            sparse.to_str().unwrap(),
            "-o",
            output.to_str().unwrap(),
        ]);
        assert_eq!(out.status.code(), Some(0));
        let mut expected = Vec::new();
        context
            .decode_raw_bytes(
                &sparse_bytes,
                splot_decode::DecodeOptions::default().with_temporal_layer_selection(
                    splot_decode::TemporalLayerSelection::OutputMask(mask),
                ),
                &mut expected,
            )
            .unwrap();
        assert_eq!(std::fs::read(&output).unwrap(), expected);
    }
}

#[test]
fn decode_rejects_invalid_and_conflicting_temporal_flags() {
    let input = conformance_vector("syn-tlayer2-dependent-8bit.ivf");
    for extra in [
        &["--temporal-layer", "0", "--temporal-output-mask", "2"][..],
        &["--temporal-layer", "4"][..],
        &["--temporal-output-mask", "0"][..],
    ] {
        let output = temp_output("raw");
        let mut args = vec!["decode", "--output-format", "raw"];
        args.extend_from_slice(extra);
        args.extend_from_slice(&[input.to_str().unwrap(), "-o", output.to_str().unwrap()]);
        let out = splot(&args);
        assert_eq!(
            out.status.code(),
            Some(2),
            "expected exit 2 for {extra:?}, stderr={}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
}

#[test]
fn decode_embedded_layer_and_output_mask_flags_match_decode_options() {
    let input = conformance_vector("syn-mlayer2-crossref-8bit.obu");
    let bytes = std::fs::read(&input).unwrap();
    let context =
        splot_decode::DecodeContext::new(splot_decode::DecodeRuntimeConfig::default()).unwrap();

    for (args_extra, selection) in [
        (
            &["--embedded-layer", "all"][..],
            splot_decode::EmbeddedLayerSelection::All,
        ),
        (
            &["--embedded-layer", "0"][..],
            splot_decode::EmbeddedLayerSelection::BASE_ONLY,
        ),
        (
            &["--embedded-layer", "1"][..],
            splot_decode::EmbeddedLayerSelection::Target(
                splot_core::types::EmbeddedLayerId::from_bits(1),
            ),
        ),
        (
            &["--embedded-output-mask", "2"][..],
            splot_decode::EmbeddedLayerSelection::OutputMask(0b10),
        ),
        (
            &["--embedded-output-mask", "0x3"][..],
            splot_decode::EmbeddedLayerSelection::OutputMask(0b11),
        ),
    ] {
        let output = temp_output("raw");
        let mut args = vec!["decode", "--output-format", "raw"];
        args.extend_from_slice(args_extra);
        args.extend_from_slice(&[input.to_str().unwrap(), "-o", output.to_str().unwrap()]);
        let out = splot(&args);
        assert_eq!(
            out.status.code(),
            Some(0),
            "expected exit 0 for {args_extra:?}, stderr={}",
            String::from_utf8_lossy(&out.stderr)
        );
        let mut expected = Vec::new();
        context
            .decode_raw_bytes(
                &bytes,
                splot_decode::DecodeOptions::default().with_embedded_layer_selection(selection),
                &mut expected,
            )
            .unwrap();
        assert_eq!(std::fs::read(&output).unwrap(), expected);
    }
}

#[test]
fn decode_rejects_invalid_and_conflicting_embedded_flags() {
    let input = conformance_vector("syn-mlayer2-crossref-8bit.obu");
    for extra in [
        &["--embedded-layer", "0", "--embedded-output-mask", "2"][..],
        &["--embedded-layer", "8"][..],
        &["--embedded-output-mask", "0"][..],
        &["--embedded-output-mask", "256"][..],
    ] {
        let output = temp_output("raw");
        let mut args = vec!["decode", "--output-format", "raw"];
        args.extend_from_slice(extra);
        args.extend_from_slice(&[input.to_str().unwrap(), "-o", output.to_str().unwrap()]);
        let out = splot(&args);
        assert_eq!(
            out.status.code(),
            Some(2),
            "expected exit 2 for {extra:?}, stderr={}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
}
