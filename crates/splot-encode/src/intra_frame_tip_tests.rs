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

/// A frame whose references are all in its future has no TIP reference
/// pair (§ 7.12.2): it codes `TIP_FRAME_DISABLED`, since the decoder reads
/// `tip_mode` in every TIP frame, and the stream decodes to the encoder recon.
#[test]
fn a_frame_without_a_tip_pair_codes_tip_disabled() {
    let config = config(64, 48, BitDepth::Eight, 6, 110);
    let mut coder = IntraFrameCoder::with_preset(&config, with_tip(1)).unwrap();
    let mut planner = crate::gop::GopPlanner::new(config.keyint, 4);
    let mut steps = planner.plan(1).unwrap();
    steps.extend(planner.plan(4).unwrap());
    let mut plans = steps.into_iter().filter_map(|step| match step {
        crate::gop::GopStep::Code(plan) => Some(plan),
        crate::gop::GopStep::ShowExisting { .. } => None,
    });
    let mut units = Vec::new();
    let mut expected = Vec::new();
    for plan in plans.by_ref().take(2) {
        let coded = coder
            .encode_frame(
                &panning_frame(&config, plan.display_index as u32),
                &plan,
                110,
            )
            .unwrap();
        if plan.show_frame {
            expected.extend(recon_bytes(&coded, config.bit_depth));
        }
        units.push(coded.temporal_unit);
    }
    let middle = plans.next().unwrap();
    let past_less = crate::gop::GopFrame {
        order_hint: 0,
        ..middle
    };
    let tools = crate::gop::frame_tools(&coder.preset, coder.tiles, 110);
    let core = past_less
        .header(
            &coder.view,
            &coder.references,
            110,
            &FrameFilterParams::default(),
            tools,
        )
        .unwrap();
    assert!(core.inter.as_ref().unwrap().num_total_refs.unwrap() >= 2);
    assert_eq!(
        core.inter.as_ref().unwrap().tip_frame_mode,
        Some(TipFrameMode::AsRef)
    );
    assert!(!coder.references.tip_pair_available(&core));
    let mut coded = coder
        .code_frame(&panning_frame(&config, 0), &past_less, 110, &[])
        .unwrap();
    let core = coded.reference.as_ref().unwrap().0.clone();
    assert_eq!(
        core.inter.as_ref().unwrap().tip_frame_mode,
        Some(TipFrameMode::Disabled)
    );
    coder.commit(&mut coded).unwrap();
    if past_less.show_frame {
        expected.extend(recon_bytes(&coded, config.bit_depth));
    }
    units.push(coded.temporal_unit);
    assert_same_samples(&decode(&units.concat()), &expected, "no TIP pair");
}
