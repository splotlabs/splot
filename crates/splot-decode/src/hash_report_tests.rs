// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::*;

#[test]
fn raw_intermediate_report_uses_stable_contract_fields() {
    let report = DecodeHashReport::raw_intermediate_output("threads=1", Vec::new());

    assert_eq!(report.contract_id, "splot.decode.hash_report");
    assert_eq!(report.contract_version, 1);
    assert_eq!(
        report.selected_output_variants,
        vec![DecodeOutputVariant::RawIntermediateOutput]
    );
    assert_eq!(report.selected_thread_policy, "threads=1");
    assert!(report.frames.is_empty());
}

#[test]
fn stable_string_identifiers_match_output_contract() {
    assert_eq!(
        DecodeOutputVariant::RawIntermediateOutput.as_str(),
        "raw_intermediate_output"
    );
    assert_eq!(DecodeHashPixelFormat::Monochrome.as_str(), "monochrome");
    assert_eq!(DecodeHashPixelFormat::Yuv420.as_str(), "yuv420");
    assert_eq!(DecodeHashPixelFormat::Yuv422.as_str(), "yuv422");
    assert_eq!(DecodeHashPixelFormat::Yuv444.as_str(), "yuv444");
    assert_eq!(DECODE_HASH_REPORT_HASH_ALGORITHM_ID, "splot-dfh-sha256-v1");
    assert_eq!(DECODE_HASH_REPORT_BYTE_STREAM_ID, "av2-output-samples-v1");
    assert_eq!(DECODE_HASH_REPORT_SHA256_DIGEST_HEX_LEN, 64);
}

#[test]
fn raw_intermediate_hash_entry_uses_stable_identifiers() {
    let digest = "0".repeat(DECODE_HASH_REPORT_SHA256_DIGEST_HEX_LEN);
    let entry = DecodeHashEntry::raw_intermediate_output_sha256(digest.clone());

    assert_eq!(entry.variant, DecodeOutputVariant::RawIntermediateOutput);
    assert_eq!(entry.algorithm_id, DECODE_HASH_REPORT_HASH_ALGORITHM_ID);
    assert_eq!(entry.byte_stream_id, DECODE_HASH_REPORT_BYTE_STREAM_ID);
    assert_eq!(entry.digest_hex, digest);
}
