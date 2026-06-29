// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>
#![no_main]

use std::sync::OnceLock;

use libfuzzer_sys::fuzz_target;
use splot_decode::{
    DECODE_HASH_REPORT_BYTE_STREAM_ID, DECODE_HASH_REPORT_CONTRACT_ID,
    DECODE_HASH_REPORT_CONTRACT_VERSION, DECODE_HASH_REPORT_HASH_ALGORITHM_ID,
    DECODE_HASH_REPORT_SHA256_DIGEST_HEX_LEN, DecodeContext, DecodeHashPixelFormat,
    DecodeLimitThreshold, DecodeLimits, DecodeOptions, DecodeOutputVariant, DecodeRuntimeConfig,
};
use splot_parallel::ThreadCount;

const FIXTURE_MODE_FLAG: u8 = 0b1000_0000;
const MAX_RAW_INPUT_BYTES: usize = 4096;
const MAX_FIXTURE_MUTATIONS: usize = 8;
const MINIMAL_FIXTURE: &[u8] =
    include_bytes!("../../tests/conformance/vectors/valid/syn-flat-intra-64x64-minimal.ivf");

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
        fixture_bytes = mutated_minimal_fixture(payload);
        fixture_bytes.as_slice()
    };

    let options = DecodeOptions::new(runtime_hash_fuzz_limits(flags, bitstream.len()));
    if let Ok(report) = context.decode_hash_report_bytes(bitstream, options) {
        if bitstream == MINIMAL_FIXTURE {
            assert_minimal_hash_report_shape(&report);
        }
    }
});

fn mutated_minimal_fixture(mutations: &[u8]) -> Vec<u8> {
    let mut bytes = MINIMAL_FIXTURE.to_vec();
    let (count, mutation_bytes) = match mutations.split_first() {
        Some((count, mutation_bytes)) => (usize::from(*count), mutation_bytes),
        None => (0, &[][..]),
    };
    let mutation_count = count.min(MAX_FIXTURE_MUTATIONS);

    for chunk in mutation_bytes.chunks_exact(3).take(mutation_count) {
        let offset_seed = u16::from_le_bytes([chunk[0], chunk[1]]);
        let offset = usize::from(offset_seed) % bytes.len();
        bytes[offset] = chunk[2];
    }

    bytes
}

fn runtime_hash_fuzz_limits(flags: u8, input_len: usize) -> DecodeLimits {
    let raw_input_limit = input_len.max(MINIMAL_FIXTURE.len()).max(1) as u64;
    let scale = 1 + u64::from(flags & 0b0000_1111);
    let max = DecodeLimitThreshold::Max;

    DecodeLimits::DEFAULT
        .with_max_input_bytes(max(raw_input_limit))
        .with_max_obus(max(4 + scale * 4))
        .with_max_ivf_frame_records(max(1 + scale))
        .with_max_frames_to_decode(max(1))
        .with_max_output_frames(max(1))
        .with_max_frame_width(max(256))
        .with_max_frame_height(max(256))
        .with_max_luma_samples_per_frame(max(256 * 256))
        .with_max_decoded_frame_bytes(max(256 * 256 * 3))
        .with_max_reference_slots(max(8))
        .with_max_reference_store_bytes(max(256 * 256 * 3 * 8))
        .with_max_tile_count(max(1 + scale))
        .with_max_tile_partition_steps(max(64 + scale * 32))
        .with_max_tile_payload_bytes(max(128 + scale * 64))
        .with_max_output_bytes(max(8192 + scale * 512))
}

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
