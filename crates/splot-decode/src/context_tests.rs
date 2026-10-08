// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::unwrap_used, clippy::expect_used)]

use super::*;
use crate::test_support::MINIMAL_FIXTURE;
use splot_parallel::ThreadCount;

#[test]
fn default_runtime_config_is_auto() {
    assert_eq!(
        DecodeRuntimeConfig::default().thread_count,
        ThreadCount::Auto
    );
}

#[test]
fn context_resolves_fixed_thread_count() {
    let ctx = DecodeContext::new(DecodeRuntimeConfig::new(ThreadCount::from(4usize))).unwrap();
    assert_eq!(ctx.threads().get(), 4);
}

#[test]
fn zero_threads_maps_to_auto() {
    let ctx = DecodeContext::new(DecodeRuntimeConfig::new(ThreadCount::from(0usize))).unwrap();
    assert!(ctx.threads().get() >= 1);
}

#[test]
fn discard_output_decodes_supported_fixture() {
    let ctx = DecodeContext::new(DecodeRuntimeConfig::new(ThreadCount::from(1usize))).unwrap();

    ctx.decode_discard_bytes(MINIMAL_FIXTURE, DecodeOptions::default())
        .unwrap();
}

#[test]
fn a_context_reused_across_streams_decodes_like_a_new_one() {
    const STREAMS: [&[u8]; 4] = [
        include_bytes!("../../../tests/conformance/vectors/valid/syn-3frame-multiref-64x64.ivf"),
        include_bytes!(
            "../../../tests/conformance/vectors/valid/syn-8frame-opfl-refine-all-64x64-q120.ivf"
        ),
        include_bytes!("../../../tests/conformance/vectors/valid/syn-frame-tip-families-64x64.ivf"),
        include_bytes!("../../../tests/conformance/vectors/valid/syn-warp-inter-128x128.ivf"),
    ];
    let context = || {
        DecodeContext::new(
            DecodeRuntimeConfig::new(ThreadCount::from(4usize))
                .with_frame_delay(splot_parallel::FrameDelay::from(4usize)),
        )
        .unwrap()
    };
    let decode = |context: &DecodeContext, bytes: &[u8]| {
        let mut output = Vec::new();
        context
            .decode_raw_bytes(bytes, DecodeOptions::default(), &mut output)
            .unwrap();
        output
    };
    let reused = context();
    for bytes in STREAMS.into_iter().chain(STREAMS) {
        assert_eq!(decode(&reused, bytes), decode(&context(), bytes));
    }
}
