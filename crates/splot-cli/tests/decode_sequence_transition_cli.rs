// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! End-to-end sequence-transition decode tests.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;
use common::{splot, temp_path};

const FIRST: &[u8] =
    include_bytes!("../../../tests/conformance/vectors/valid/syn-flat-intra-64x64-minimal.ivf");
const COMPATIBLE_SECOND: &[u8] =
    include_bytes!("../../../tests/conformance/vectors/valid/syn-flat-intra-64x64-q80.ivf");
const CROPPED_SECOND: &[u8] =
    include_bytes!("../../../tests/conformance/vectors/valid/syn-crop-intra-64x64-q80.ivf");
const CROPPED_TRANSITION: &[u8] =
    include_bytes!("../../../tests/conformance/vectors/valid/syn-2seq-crop-intra-64x64.obu");

fn repeated_sequence_ivf(second: &[u8]) -> Vec<u8> {
    let mut bytes = FIRST[..32].to_vec();
    bytes[24..28].copy_from_slice(&2u32.to_le_bytes());
    bytes.extend_from_slice(&FIRST[32..]);
    let second_record_start = bytes.len();
    bytes.extend_from_slice(&second[32..]);
    bytes[second_record_start + 4..second_record_start + 12].copy_from_slice(&1u64.to_le_bytes());
    bytes
}

#[test]
fn raw_cli_writes_both_changed_sequence_formats() {
    let input = temp_path("raw-input", "obu");
    let output = temp_path("raw-output", "raw");
    std::fs::write(&input, CROPPED_TRANSITION).unwrap();

    let result = splot(&[
        "decode",
        "--output-format",
        "raw",
        input.to_str().unwrap(),
        "-o",
        output.to_str().unwrap(),
    ]);

    assert_eq!(result.status.code(), Some(0));
    assert!(result.stdout.is_empty());
    assert!(result.stderr.is_empty());
    assert_eq!(std::fs::metadata(output).unwrap().len(), 11_544);
}

#[test]
fn y4m_cli_writes_compatible_changed_sequence() {
    let input = temp_path("compatible-input", "ivf");
    let output = temp_path("compatible-output", "y4m");
    std::fs::write(&input, repeated_sequence_ivf(COMPATIBLE_SECOND)).unwrap();

    let result = splot(&[
        "decode",
        input.to_str().unwrap(),
        "-o",
        output.to_str().unwrap(),
    ]);

    assert_eq!(result.status.code(), Some(0));
    assert!(result.stdout.is_empty());
    assert!(result.stderr.is_empty());
    let y4m = std::fs::read(output).unwrap();
    assert!(y4m.starts_with(b"YUV4MPEG2 W64 H64 F30:1 Ip A0:0 C420\n"));
    assert_eq!(
        y4m.windows(b"FRAME\n".len())
            .filter(|window| *window == b"FRAME\n")
            .count(),
        2
    );
}

#[test]
fn y4m_cli_rejects_format_change_without_touching_output() {
    let input = temp_path("incompatible-input", "ivf");
    let output = temp_path("incompatible-output", "y4m");
    let sentinel = b"existing output remains intact";
    std::fs::write(&input, repeated_sequence_ivf(CROPPED_SECOND)).unwrap();
    std::fs::write(&output, sentinel).unwrap();

    let result = splot(&[
        "decode",
        "--json",
        input.to_str().unwrap(),
        "-o",
        output.to_str().unwrap(),
    ]);

    assert_eq!(result.status.code(), Some(1));
    assert!(result.stderr.is_empty());
    let json: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(json["rule_id"], "decode/output-error");
    assert_eq!(json["output_format"], "y4m");
    assert_eq!(json["output_operation"], "serialize_y4m");
    assert_eq!(json["output_source_kind"], "y4m");
    assert!(
        json["output_source_message"]
            .as_str()
            .is_some_and(|message| message.contains("stream/frame mismatch"))
    );
    assert_eq!(std::fs::read(output).unwrap(), sentinel);
}
