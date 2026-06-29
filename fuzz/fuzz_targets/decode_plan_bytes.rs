// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>
#![no_main]

use std::sync::OnceLock;

use libfuzzer_sys::fuzz_target;
use splot_decode::{
    DecodeContext, DecodeLimitThreshold, DecodeLimits, DecodeOptions, DecodeRuntimeConfig,
};
use splot_parallel::ThreadCount;

static CONTEXT: OnceLock<Option<DecodeContext>> = OnceLock::new();

fuzz_target!(|data: &[u8]| {
    let (flags, bitstream) = match data.split_first() {
        Some((flags, bitstream)) => (*flags, bitstream),
        None => (0, &[][..]),
    };

    let max_input_bytes = 1 + u64::from(flags & 0b0011_1111) * 512;
    let max_obus = 1 + u64::from((flags >> 6) & 0b0000_0011) * 16;
    let limits = DecodeLimits::DEFAULT
        .with_max_input_bytes(DecodeLimitThreshold::Max(max_input_bytes))
        .with_max_obus(DecodeLimitThreshold::Max(max_obus))
        .with_max_ivf_frame_records(DecodeLimitThreshold::Max(32))
        .with_max_frames_to_decode(DecodeLimitThreshold::Max(8));
    let options = DecodeOptions::new(limits);

    let Some(context) = CONTEXT
        .get_or_init(|| {
            DecodeContext::new(DecodeRuntimeConfig::new(ThreadCount::from(1usize))).ok()
        })
        .as_ref()
    else {
        return;
    };
    let _ = context.plan_bytes(bitstream, options);
});
