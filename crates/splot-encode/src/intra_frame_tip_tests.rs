// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! TIP as a reference (§ 7.10.6, § 5.18.2 `tip_frame_mode`, § 5.20.7.6
//! `tip_mode`). Random TIP leaves are covered by
//! `random_inter_leaves_decode_to_the_writer_recon`.

use super::*;
use crate::preset::PresetParams;
use splot_core::headers::frame::TipFrameMode;

/// Preset 6 with `tip`.
fn with_tip(tip: u8) -> &'static PresetParams {
    with_tip_output(tip, 0)
}

/// Preset 6 with `tip` and `tip_output`.
fn with_tip_output(tip: u8, tip_output: u8) -> &'static PresetParams {
    Box::leak(Box::new(PresetParams {
        tip,
        tip_output,
        ..*Preset::new(6).unwrap().params()
    }))
}

/// With `tip` the sequence enables TIP, every inter frame with two
/// references codes `TIP_FRAME_AS_REF` (with equal weights at `tip = 1`),
/// and mini-GOP streams of 4:2:0 10-bit, 4:2:2 8-bit and 4:4:4 10-bit decode
/// to the encoder recon; without it no frame codes TIP.
#[test]
fn tip_frames_reach_the_headers_and_the_decoder() {
    for (chroma, bit_depth) in [
        (PixelFormat::Yuv420, BitDepth::Ten),
        (PixelFormat::Yuv422, BitDepth::Eight),
        (PixelFormat::Yuv444, BitDepth::Ten),
    ] {
        let mut config = config(72, 40, bit_depth, 6, 110);
        config.chroma = chroma;
        for tip in [2, 1, 0] {
            let (units, expected, cores, coder) =
                super::carry::planned_stream(&config, with_tip(tip));
            assert_eq!(coder.sequence.inter.unwrap().enable_tip, tip > 0);
            let weights: Vec<_> = cores
                .iter()
                .filter_map(|core| core.inter.as_ref())
                .filter(|inter| inter.tip_frame_mode == Some(TipFrameMode::AsRef))
                .map(|inter| inter.tip_global_wtd_index.unwrap())
                .collect();
            assert_eq!(!weights.is_empty(), tip > 0, "{chroma:?} {weights:?}");
            if tip == 1 {
                assert!(weights.iter().all(|&weight| weight == 0), "{weights:?}");
            }
            let name = format!("{chroma:?} tip {tip}");
            assert_same_samples(&decode(&units), &expected, &name);
        }
    }
}

/// A tiled stream with the TIP weight search codes the same bytes on 1, 2 and 8 threads and
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
        super::carry::planned_stream(&config, with_tip(2))
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

/// The weight search picks a weight that leans to the past reference when
/// the frame repeats the past reference, a weight that leans the other way
/// when it repeats the future reference, and the stream decodes to the
/// encoder recon.
#[test]
fn the_tip_weight_search_follows_the_nearer_content() {
    let config = config(128, 128, BitDepth::Eight, 6, 110);
    let mut coder = IntraFrameCoder::with_preset(&config, with_tip(2)).unwrap();
    let mut planner = crate::gop::GopPlanner::new(config.keyint, 4);
    let mut steps = planner.plan(1).unwrap();
    steps.extend(planner.plan(4).unwrap());
    let mut plans = steps.into_iter().filter_map(|step| match step {
        crate::gop::GopStep::Code(plan) => Some(plan),
        crate::gop::GopStep::ShowExisting { .. } => None,
    });
    let (past, future) = (panning_frame(&config, 0), panning_frame(&config, 12));
    let mut units = Vec::new();
    let mut expected = Vec::new();
    for (plan, source) in plans.by_ref().zip([&past, &future]) {
        let coded = coder.encode_frame(source, &plan, 110).unwrap();
        if plan.show_frame {
            expected.extend(recon_bytes(&coded, config.bit_depth));
        }
        units.push(coded.temporal_unit);
    }
    let middle = plans.next().unwrap();
    let tools = crate::gop::frame_tools(&coder.preset, coder.tiles, 110);
    let core = middle
        .header(
            &coder.view,
            &coder.references,
            110,
            &FrameFilterParams::default(),
            tools,
        )
        .unwrap();
    assert!(coder.references.tip_pair_available(&core));
    let (to_past, to_future) = (
        coder.tip_weight(&past, &core).unwrap(),
        coder.tip_weight(&future, &core).unwrap(),
    );
    let factor = |index: u8| [8, 12, 16, 18, 20, 4, 6, -4][usize::from(index)] - 8;
    assert!(
        factor(to_past) * factor(to_future) < 0,
        "{to_past} {to_future}"
    );
    let mut coded = coder.code_frame(&past, &middle, 110, &[]).unwrap();
    let core = coded.reference.as_ref().unwrap().0.clone();
    assert_eq!(core.inter.unwrap().tip_global_wtd_index, Some(to_past));
    coder.commit(&mut coded).unwrap();
    if middle.show_frame {
        expected.extend(recon_bytes(&coded, config.bit_depth));
    }
    units.push(coded.temporal_unit);
    assert_same_samples(&decode(&units.concat()), &expected, "weight search");
}

/// With a `tip_output` that prefers it, the shown non-reference frames of a
/// panning texture at a coarse quantizer code `OBU_REGULAR_TIP`
/// `TIP_FRAME_AS_OUTPUT` frames from the projected motion, and streams of
/// 4:2:0 10-bit, 4:2:2 8-bit and 4:4:4 10-bit decode to the encoder recon;
/// with `tip_output = 0` the sequence disables TIP output and no frame codes
/// it.
#[test]
fn tip_output_frames_decode_to_the_encoder_recon() {
    for (chroma, bit_depth) in [
        (PixelFormat::Yuv420, BitDepth::Ten),
        (PixelFormat::Yuv422, BitDepth::Eight),
        (PixelFormat::Yuv444, BitDepth::Ten),
    ] {
        let mut config = config(72, 40, bit_depth, 6, 250);
        config.chroma = chroma;
        for tip_output in [200, 0] {
            let coder =
                IntraFrameCoder::with_preset(&config, with_tip_output(1, tip_output)).unwrap();
            let (units, expected, cores, coder) =
                super::carry::planned_stream_of(&config, coder, panning_frame, 250);
            let inter = coder.sequence.inter.unwrap();
            assert_eq!(inter.enable_tip_output, tip_output > 0);
            let outputs: Vec<_> = cores
                .iter()
                .filter(|core| core.obu_type == splot_core::types::ObuType::RegularTip)
                .collect();
            assert_eq!(!outputs.is_empty(), tip_output > 0, "{chroma:?}");
            for core in outputs {
                let inter = core.inter.as_ref().unwrap();
                assert_eq!(inter.tip_frame_mode, Some(TipFrameMode::AsOutput));
                assert_eq!(core.refresh_frame_flags, Some(0));
            }
            let name = format!("{chroma:?} tip_output {tip_output}");
            assert_same_samples(&decode(&units), &expected, &name);
        }
    }
}

/// `tip_output = 0` codes the same stream as a coder whose preset prefers
/// TIP output but whose sequence disables it: the preset value reaches the
/// stream only through `disable_tip_output` and the frame decision.
#[test]
fn tip_output_zero_codes_the_stream_with_tip_output_disabled() {
    let config = config(72, 40, BitDepth::Ten, 6, 180);
    let off = IntraFrameCoder::with_preset(&config, with_tip_output(1, 0)).unwrap();
    let mut disabled = IntraFrameCoder::with_preset(&config, with_tip_output(1, 200)).unwrap();
    disabled
        .set_tools(EncoderToolFlags {
            tip_output: false,
            ..disabled.params.tools
        })
        .unwrap();
    let (off, ..) = super::carry::planned_stream_of(&config, off, panning_frame, 180);
    let (disabled, ..) = super::carry::planned_stream_of(&config, disabled, panning_frame, 180);
    assert_eq!(off, disabled);
}

/// Preset 6 with TIP output preferred and TIP refinement over optical flow.
fn with_tip_refinement() -> &'static PresetParams {
    Box::leak(Box::new(PresetParams {
        tip: 1,
        tip_output: 200,
        opfl: true,
        tip_refinemv: true,
        ..*Preset::new(6).unwrap().params()
    }))
}

/// With TIP refinement, TIP blocks and TIP output frames refine their
/// projected vectors, frames with a past and a future reference code no TIP
/// weight, and panning streams at a fine and a coarse quantizer decode to the
/// encoder recon in 4:2:0 10-bit, 4:2:2 8-bit and 4:4:4 10-bit, and in a
/// 160x96 4:2:0 stream with 16x16-capable TIP blocks.
#[test]
fn refined_tip_streams_decode_to_the_encoder_recon() {
    for (chroma, bit_depth, (width, height)) in [
        (PixelFormat::Yuv420, BitDepth::Ten, (72, 40)),
        (PixelFormat::Yuv422, BitDepth::Eight, (72, 40)),
        (PixelFormat::Yuv444, BitDepth::Ten, (72, 40)),
        (PixelFormat::Yuv420, BitDepth::Eight, (160, 96)),
    ] {
        for qp in [110, 250] {
            let mut config = config(width, height, bit_depth, 6, qp);
            config.chroma = chroma;
            let coder = IntraFrameCoder::with_preset(&config, with_tip_refinement()).unwrap();
            let (units, expected, cores, coder) =
                super::carry::planned_stream_of(&config, coder, panning_frame, qp);
            assert!(coder.sequence.inter.unwrap().enable_tip_refinemv);
            let tips: Vec<_> = cores
                .iter()
                .filter_map(|core| core.inter.as_ref())
                .filter(|inter| {
                    inter
                        .tip_frame_mode
                        .is_some_and(|mode| mode != TipFrameMode::Disabled)
                })
                .collect();
            assert!(!tips.is_empty(), "{chroma:?} {qp}");
            assert!(
                tips.iter()
                    .all(|inter| inter.tip_global_wtd_index == Some(0))
            );
            let name = format!("{chroma:?} qp {qp}");
            assert_same_samples(&decode(&units), &expected, &name);
        }
    }
}

/// AVM decodes refined TIP streams like splot.
#[test]
#[ignore = "needs an AVM decoder build at SPLOT_AVM_DECODER"]
fn avm_decodes_refined_tip_streams_like_splot() {
    let Some(avmdec) = avm_decoder() else {
        return;
    };
    for chroma in [
        PixelFormat::Yuv420,
        PixelFormat::Yuv422,
        PixelFormat::Yuv444,
    ] {
        for qp in [110, 250] {
            let mut config = config(72, 40, BitDepth::Ten, 6, qp);
            config.chroma = chroma;
            let coder = IntraFrameCoder::with_preset(&config, with_tip_refinement()).unwrap();
            let (units, expected, ..) =
                super::carry::planned_stream_of(&config, coder, panning_frame, qp);
            let avm = avm_decode(
                &avmdec,
                &[units],
                (config.width, config.height),
                config.bit_depth,
                &format!("tiprefine{chroma:?}{qp}"),
            );
            assert!(avm == expected, "{chroma:?} {qp}: AVM output differs");
        }
    }
}
