// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use splot_decode::{DecodeLimitThreshold, DecodeLimits};

const MAX_FIXTURE_MUTATIONS: usize = 8;

pub const MINIMAL_FIXTURE: &[u8] =
    include_bytes!("../../tests/conformance/vectors/valid/syn-flat-intra-64x64-minimal.ivf");

pub fn mutated_minimal_fixture(mutations: &[u8]) -> Vec<u8> {
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

pub fn limits(flags: u8, input_len: usize) -> DecodeLimits {
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
}
