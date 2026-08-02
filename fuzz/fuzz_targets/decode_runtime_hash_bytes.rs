// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>
#![no_main]

use std::sync::OnceLock;

use libfuzzer_sys::fuzz_target;
use splot_decode::{
    DECODE_HASH_REPORT_BYTE_STREAM_ID, DECODE_HASH_REPORT_CONTRACT_ID,
    DECODE_HASH_REPORT_CONTRACT_VERSION, DECODE_HASH_REPORT_HASH_ALGORITHM_ID,
    DECODE_HASH_REPORT_SHA256_DIGEST_HEX_LEN, DecodeContext, DecodeHashPixelFormat,
    DecodeOptions, DecodeOutputVariant, DecodeRuntimeConfig,
};
use splot_parallel::ThreadCount;

#[path = "../support/decode_runtime.rs"]
mod decode_runtime;

const FIXTURE_MODE_FLAG: u8 = 0b1000_0000;
const MAX_RAW_INPUT_BYTES: usize = 4096;

static CONTEXT: OnceLock<Option<DecodeContext>> = OnceLock::new();

fuzz_target!(|data: &[u8]| {
    let (flags, payload) = match data.split_first() {
        Some((flags, payload)) => (*flags, payload),
        None => (0, &[][..]),
    };

    let Some(context) = CONTEXT
        .get_or_init(|| {
            DecodeContext::new(DecodeRuntimeConfig::new(ThreadCount::from(1usize))).ok()
        })
        .as_ref()
    else {
        return;
    };

    let fixture_bytes;
    let bitstream = if flags & FIXTURE_MODE_FLAG == 0 {
        let len = payload.len().min(MAX_RAW_INPUT_BYTES);
        &payload[..len]
    } else {
        fixture_bytes = decode_runtime::mutated_minimal_fixture(payload);
        fixture_bytes.as_slice()
    };

    let options = DecodeOptions::new(decode_runtime::limits(flags, bitstream.len()));
    if let Ok(report) = context.decode_hash_report_bytes(bitstream, options) {
        if bitstream == decode_runtime::MINIMAL_FIXTURE {
            assert_minimal_hash_report_shape(&report);
        }
    }
});

fn assert_minimal_hash_report_shape(report: &splot_decode::DecodeHashReport) {
    assert_eq!(report.contract_id, DECODE_HASH_REPORT_CONTRACT_ID);
    assert_eq!(report.contract_version, DECODE_HASH_REPORT_CONTRACT_VERSION);
    assert_eq!(
        report.selected_output_variants,
        vec![DecodeOutputVariant::RawIntermediateOutput]
    );
    assert_eq!(report.frames.len(), 1);

    let frame = &report.frames[0];
    assert_eq!(frame.output_index, 0);
    assert_eq!(frame.visible_luma_left, 0);
    assert_eq!(frame.visible_luma_top, 0);
    assert_eq!(frame.visible_luma_width, 64);
    assert_eq!(frame.visible_luma_height, 64);
    assert_eq!(frame.chroma_left, Some(0));
    assert_eq!(frame.chroma_top, Some(0));
    assert_eq!(frame.chroma_width, Some(32));
    assert_eq!(frame.chroma_height, Some(32));
    assert_eq!(frame.bit_depth, 8);
    assert_eq!(frame.pixel_format, DecodeHashPixelFormat::Yuv420);
    assert_eq!(frame.hashes.len(), 1);

    let hash = &frame.hashes[0];
    assert_eq!(hash.variant, DecodeOutputVariant::RawIntermediateOutput);
    assert_eq!(hash.algorithm_id, DECODE_HASH_REPORT_HASH_ALGORITHM_ID);
    assert_eq!(hash.byte_stream_id, DECODE_HASH_REPORT_BYTE_STREAM_ID);
    assert_eq!(
        hash.digest_hex.len(),
        DECODE_HASH_REPORT_SHA256_DIGEST_HEX_LEN
    );
    assert!(
        hash.digest_hex
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
    );
}
