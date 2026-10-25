// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! TIP as a reference (§ 7.11.3, § 5.18.2 `tip_frame_mode`, § 5.20.7.6
//! `tip_mode`). Random TIP leaves are covered by
//! `random_inter_leaves_decode_to_the_writer_recon`.

use super::*;
use crate::preset::PresetParams;
use splot_core::headers::frame::TipFrameMode;

/// Preset 6 with `tip`.
fn with_tip(tip: u8) -> &'static PresetParams {
    Box::leak(Box::new(PresetParams {
        tip,
        ..*Preset::new(6).unwrap().params()
    }))
}

/// With `tip` the sequence enables TIP, every inter frame with two
/// references codes `TIP_FRAME_AS_REF`, and mini-GOP streams of 4:2:0
/// 10-bit, 4:2:2 8-bit and 4:4:4 10-bit decode to the encoder recon; without
/// it no frame codes TIP.
#[test]
fn tip_frames_reach_the_headers_and_the_decoder() {
    for (chroma, bit_depth) in [
        (PixelFormat::Yuv420, BitDepth::Ten),
        (PixelFormat::Yuv422, BitDepth::Eight),
        (PixelFormat::Yuv444, BitDepth::Ten),
    ] {
        let mut config = config(72, 40, bit_depth, 6, 110);
        config.chroma = chroma;
        for tip in [1, 0] {
            let (units, expected, cores, coder) =
                super::carry::planned_stream(&config, with_tip(tip));
            assert_eq!(coder.sequence.inter.unwrap().enable_tip, tip > 0);
            let tip_frames = cores
                .iter()
                .filter_map(|core| core.inter.as_ref())
                .filter(|inter| inter.tip_frame_mode == Some(TipFrameMode::AsRef))
                .count();
            assert_eq!(tip_frames > 0, tip > 0, "{chroma:?} {tip_frames}");
            let name = format!("{chroma:?} tip {tip}");
            assert_same_samples(&decode(&units), &expected, &name);
        }
    }
}

/// A tiled stream with TIP codes the same bytes on 1, 2 and 8 threads and
/// decodes to the encoder recon.
#[test]
fn tip_streams_match_on_any_thread_count() {
    let stream = |threads: usize| {
        let mut config = config(136, 72, BitDepth::Eight, 6, 110);
        config.tiles = TileLayout {
            cols_log2: 1,
            rows_log2: 0,
        };
        config.threads = splot_parallel::ThreadCount::from(threads);
        super::carry::planned_stream(&config, with_tip(1))
    };
    let (one, expected, ..) = stream(1);
    assert_same_samples(&decode(&one), &expected, "tiled");
    for threads in [2, 8] {
        assert!(stream(threads).0 == one, "{threads} threads");
    }
}
