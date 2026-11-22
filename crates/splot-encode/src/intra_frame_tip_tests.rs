// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! TIP as a reference (§ 7.10.6, § 5.18.2 `tip_frame_mode`, § 5.20.7.6
//! `tip_mode`). Random TIP leaves are covered by
//! `random_inter_leaves_decode_to_the_writer_recon`.

use super::*;
use crate::preset::PresetParams;
use splot_core::headers::FilmGrainModel;
use splot_core::headers::frame::{FrameHeaderCore, TipFrameMode};
use splot_core::headers::sequence::ChromaFormatIdc;
use splot_recon::{
    DecodedFrame, DecodedFrameInfo, FramePlanes, OutputIndex, Plane, PlaneRect, PlaneSize,
    apply_film_grain,
};

/// Preset 6 with `tip`.
fn with_tip(tip: u8) -> &'static PresetParams {
    with_tip_output(tip, 0)
}

/// Preset 6 with `tip` and `tip_output`, without TIP refinement, so frames
/// code their TIP weight.
fn with_tip_output(tip: u8, tip_output: u8) -> &'static PresetParams {
    Box::leak(Box::new(PresetParams {
        tip,
        tip_output,
        tip_refinemv: false,
        ..*Preset::new(6).unwrap().params()
    }))
}

/// With `tip` the sequence enables TIP, every inter frame with two
/// references codes `TIP_FRAME_AS_REF` (with equal weights at `tip = 1`),
/// and mini-GOP streams of 4:2:0 10-bit, 4:2:2 8-bit and 4:4:4 10-bit decode
/// to the encoder recon; without it no frame codes TIP.
#[test]
fn tip_frames_reach_the_headers_and_the_decoder() {
    each_case(
        [
            (PixelFormat::Yuv420, BitDepth::Ten),
            (PixelFormat::Yuv422, BitDepth::Eight),
            (PixelFormat::Yuv444, BitDepth::Ten),
        ],
        |(chroma, bit_depth)| {
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
        },
    );
}

/// A tiled stream with the TIP weight search codes the same bytes on 1, 2 and 8 threads and
/// decodes to the encoder recon.
#[test]
fn tip_streams_match_on_any_thread_count() {
    let stream = |threads: usize| {
        let mut config = config(136, 72, BitDepth::Eight, 6, 110);
        config.tiles = TileLayout::Uniform {
            cols_log2: 1,
            rows_log2: 0,
        };
        config.threads = splot_parallel::ThreadCount::from(threads);
        super::carry::planned_stream(&config, with_tip(2))
    };
    let (one, expected, ..) = stream(1);
    assert_same_samples(&decode(&one), &expected, "tiled");
    each_case([2, 8], |threads| {
        assert!(stream(threads).0 == one, "{threads} threads");
    });
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
    let tools = crate::gop::frame_tools(
        (&coder.preset, crate::preset::PresetContext::default()),
        &coder.tiles,
        110,
    );
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

/// A frame between the key frame and an intra-only anchor has no inter frame
/// as its closest past or future reference, so it codes no TIP (§ 7.9) and
/// the stream decodes to the encoder recon.
#[test]
fn frames_between_intra_references_code_no_tip() {
    let config = config(128, 128, BitDepth::Eight, 6, 110);
    let mut coder = IntraFrameCoder::with_preset(&config, with_tip(2)).unwrap();
    let mut planner = crate::gop::GopPlanner::new(config.keyint, 4).with_intra_only(1);
    let mut steps = planner.plan(1).unwrap();
    steps.extend(planner.plan(4).unwrap());
    let mut plans = steps.into_iter().filter_map(|step| match step {
        crate::gop::GopStep::Code(plan) => Some(plan),
        crate::gop::GopStep::ShowExisting { .. } => None,
    });
    let mut units = Vec::new();
    let mut expected = Vec::new();
    for (plan, index) in plans.by_ref().zip([0, 4]) {
        assert_eq!(plan.intra_only, index == 4);
        let coded = coder
            .encode_frame(&panning_frame(&config, index), &plan, 110)
            .unwrap();
        if plan.show_frame {
            expected.extend(recon_bytes(&coded, config.bit_depth));
        }
        units.push(coded.temporal_unit);
    }
    let middle = plans.next().unwrap();
    let tools = crate::gop::frame_tools(
        (&coder.preset, crate::preset::PresetContext::default()),
        &coder.tiles,
        110,
    );
    let core = middle
        .header(
            &coder.view,
            &coder.references,
            110,
            &FrameFilterParams::default(),
            tools,
        )
        .unwrap();
    assert_eq!(
        core.inter.as_ref().unwrap().tip_frame_mode,
        Some(TipFrameMode::AsRef)
    );
    assert!(!coder.references.tip_pair_available(&core));
    let mut coded = coder
        .code_frame(&panning_frame(&config, 2), &middle, 110, &[])
        .unwrap();
    let core = coded.reference.as_ref().unwrap().0.clone();
    assert_eq!(
        core.inter.as_ref().unwrap().tip_frame_mode,
        Some(TipFrameMode::Disabled)
    );
    coder.commit(&mut coded).unwrap();
    if middle.show_frame {
        expected.extend(recon_bytes(&coded, config.bit_depth));
    }
    units.push(coded.temporal_unit);
    assert_same_samples(&decode(&units.concat()), &expected, "intra pair");
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
    let tools = crate::gop::frame_tools(
        (&coder.preset, crate::preset::PresetContext::default()),
        &coder.tiles,
        110,
    );
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

/// With `df_sub_pu` the sequence codes `enable_df_sub_pu` and
/// `enable_tip_explicit_qp = 0`, each `TIP_FRAME_AS_OUTPUT` frame codes
/// `allow_df_sub_pu = 0`, and the stream decodes to the encoder recon.
#[test]
fn tip_output_frames_decode_with_df_sub_pu() {
    let config = config(72, 40, BitDepth::Ten, 6, 250);
    let preset = Box::leak(Box::new(PresetParams {
        df_sub_pu: true,
        ..*with_tip_output(1, 200)
    }));
    let coder = IntraFrameCoder::with_preset(&config, preset).unwrap();
    let (units, expected, cores, coder) =
        super::carry::planned_stream_of(&config, coder, panning_frame, 250);
    let inter = coder.sequence.inter.unwrap();
    assert!(inter.enable_df_sub_pu && !inter.enable_tip_explicit_qp);
    let outputs: Vec<_> = cores
        .iter()
        .filter(|core| core.obu_type == splot_core::types::ObuType::RegularTip)
        .collect();
    assert!(!outputs.is_empty());
    for core in outputs {
        let inter = core.inter.as_ref().unwrap();
        assert_eq!(inter.allow_df_sub_pu, Some(false));
    }
    assert_same_samples(&decode(&units), &expected, "tip output df_sub_pu");
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
    each_case(
        [
            (PixelFormat::Yuv420, BitDepth::Ten, (72, 40)),
            (PixelFormat::Yuv422, BitDepth::Eight, (72, 40)),
            (PixelFormat::Yuv444, BitDepth::Ten, (72, 40)),
            (PixelFormat::Yuv420, BitDepth::Eight, (160, 96)),
        ],
        |(chroma, bit_depth, (width, height))| {
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
        },
    );
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

/// AVM decodes TIP output frames of a zooming texture like splot, in 4:4:4
/// and 4:2:0. Their projected motion differs per 8x8 unit, so § 7.10.6 optical
/// flow refines it, and § 7.13.3.7 clips each chroma read to the
/// § 7.13.3.4 reference area of the unrefined vector.
#[test]
#[ignore = "needs an AVM decoder build at SPLOT_AVM_DECODER"]
fn avm_decodes_refined_tip_output_chroma_like_splot() {
    let Some(avmdec) = avm_decoder() else {
        return;
    };
    for chroma in [PixelFormat::Yuv444, PixelFormat::Yuv420] {
        let mut config = config(128, 80, BitDepth::Eight, 6, 250);
        config.chroma = chroma;
        let coder = IntraFrameCoder::with_preset(&config, with_tip_refinement()).unwrap();
        let (units, expected, cores, coder) =
            super::carry::planned_stream_of(&config, coder, zooming_frame, 250);
        assert!(coder.sequence.inter.unwrap().enable_tip_refinemv);
        let refined_outputs: Vec<usize> = cores
            .iter()
            .filter(|core| {
                let current = core.display_order_hint().unwrap();
                core.inter.as_ref().is_some_and(|inter| {
                    inter.tip_frame_mode == Some(TipFrameMode::AsOutput)
                        && inter.opfl_refine_type.is_some_and(|kind| kind != 0)
                        && inter.tip_interpolation_filter
                            == Some(splot_core::headers::frame::InterpolationFilter::EighttapSharp)
                        && inter.order_hints.iter().any(|&hint| hint < current)
                        && inter.order_hints.iter().any(|&hint| hint > current)
                })
            })
            .map(|core| core.display_order_hint().unwrap() as usize)
            .collect();
        assert!(
            !refined_outputs.is_empty(),
            "{chroma:?}: no refined TIP output"
        );
        let avm = avm_decode(
            &avmdec,
            std::slice::from_ref(&units),
            (config.width, config.height),
            config.bit_depth,
            &format!("tipoutchroma{chroma:?}"),
        );
        let splot = decode(&units);
        assert_same_samples(&splot, &expected, &format!("{chroma:?}"));
        let frame_len = splot.len() / 9;
        for index in refined_outputs {
            let span = index * frame_len..(index + 1) * frame_len;
            assert!(
                avm[span.clone()] == splot[span],
                "{chroma:?}: AVM differs in TIP output {index}"
            );
        }
        assert!(avm == splot, "{chroma:?}: AVM output differs");
    }
}

/// Frame `index` of a texture zooming about the frame centre by 4% a frame,
/// so neighbouring 8x8 units move differently.
pub(super) fn zooming_frame(config: &Config, index: u32) -> SourceFrame {
    let max = u32::from(config.bit_depth.max_sample());
    let sizes = config.plane_sizes().unwrap();
    let planes = core::array::from_fn(|plane| {
        let size = sizes[plane];
        let (w, h) = (size.width() as i64, size.height() as i64);
        let scale = 100 + 4 * i64::from(index);
        (0..h)
            .flat_map(|y| (0..w).map(move |x| (x, y)))
            .map(|(x, y)| {
                let zx = (x - w / 2) * 100 / scale + w / 2;
                let zy = (y - h / 2) * 100 / scale + h / 2;
                texture(zx, zy, plane, max)
            })
            .collect()
    });
    SourceFrame::new(config, u64::from(index), planes).unwrap()
}

/// A tree of `node` whose 16x16 and 32x32 nodes code TIP `NEARMV` leaves
/// where they may, split further at random, and random trees elsewhere.
fn tip_tree(
    syntax: &mut IntraTileSyntax,
    node: PartitionNode,
    rng: &mut super::Rng,
    coverage: &mut super::Coverage,
) -> core::result::Result<PartitionTree, TileWriterError> {
    let (width, height) = node.dimensions()?;
    let options = syntax.partition_options(node)?;
    if options.implied().is_some() {
        return super::random_tree(syntax, node, rng, coverage);
    }
    if width <= 32 && width == height && options.allows(Partition::None) {
        let leaf = syntax.leaf(node)?;
        if (width == 16 || rng.below(2) == 0) && syntax.tip_allowed(&leaf)? {
            coverage[19] += 1;
            return Ok(PartitionTree::Leaf(LeafChoice::inter(
                InterPrediction::Single(InterModes {
                    ref_frame: syntax.reference_count(),
                    mode: InterMode::Near,
                    drl: rng.below(syntax.drl_count() as u64) as usize,
                    mv: MotionVector::default(),
                    filter: splot_recon::InterpolationFilter::EightTap,
                    precision: None,
                    amvd: false,
                    interintra: None,
                    bawp: None,
                }),
            )));
        }
    }
    if width > 16
        && options.allows(Partition::Split)
        && super::codable_children(syntax, node, Partition::Split)?
    {
        let mut children = Vec::new();
        for child in syntax.children(node, Partition::Split)? {
            children.push(if syntax.in_frame(child) {
                tip_tree(syntax, child, rng, coverage)?
            } else {
                PartitionTree::Leaf(LeafChoice::DC)
            });
        }
        return Ok(PartitionTree::Split {
            kind: Partition::Split,
            children,
        });
    }
    super::random_tree(syntax, node, rng, coverage)
}

/// TIP leaves of 16x16 and 32x32 in a zooming frame with TIP refinement
/// record 8x8 TIP units for their neighbours, as the decoder does (§ 7.10.6
/// `PredictionUnitSize` with `enable_tip_refinemv`), so later TIP leaves take
/// the same spatial TIP candidates and the frame decodes to the writer recon.
#[test]
fn refined_tip_units_match_the_decoder_in_a_zooming_frame() {
    let config = config(192, 128, BitDepth::Eight, 6, 110);
    assert!(config.preset.params().tip_refinemv);
    let leaves = each_case(0..4, |seed| {
        let (units, expected, coverage) =
            super::inter_stream_of(&config, 0x71F + seed, zooming_frame, tip_tree);
        assert_same_samples(&decode(&units.concat()), &expected, &format!("seed {seed}"));
        coverage[19]
    });
    assert!(leaves.iter().sum::<usize>() > 0);
}

/// A noisy panning frame for the film grain tests.
fn grainy_panning_frame(config: &Config, index: u32) -> SourceFrame {
    super::grain_streams::with_noise(config, &panning_frame(config, index), 5 + index)
}

/// A 72x40 4:2:0 8-bit stream of [`grainy_panning_frame`] at a coarse
/// quantizer with TIP output preferred and film grain.
fn tip_grain_stream() -> (Vec<u8>, Vec<u8>, Vec<FrameHeaderCore>) {
    let mut config = config(72, 40, BitDepth::Eight, 6, 250);
    config.film_grain = true;
    let coder = IntraFrameCoder::with_preset(&config, with_tip_output(1, 200)).unwrap();
    let (units, recon, cores, _) =
        super::carry::planned_stream_of(&config, coder, grainy_panning_frame, 250);
    (units, recon, cores)
}

/// The 72x40 4:2:0 8-bit frame `bytes` plus the § 7.21.7 grain of `model`
/// and `seed`.
fn with_grain(bytes: &[u8], model: &FilmGrainModel, seed: u16) -> Vec<u8> {
    let plane = |offset: usize, (width, height): (usize, usize)| {
        let samples = bytes[offset..offset + width * height]
            .iter()
            .map(|&sample| u16::from(sample))
            .collect();
        let size = PlaneSize::new(width, height).unwrap();
        let rect = PlaneRect::new(0, 0, width, height).unwrap();
        Plane::from_vec(size, width, rect, samples).unwrap()
    };
    let planes = FramePlanes::new(
        plane(0, (72, 40)),
        Some(plane(72 * 40, (36, 20))),
        Some(plane(72 * 40 + 36 * 20, (36, 20))),
    );
    let info = DecodedFrameInfo::new(
        OutputIndex::new(0),
        BitDepth::Eight,
        PixelFormat::Yuv420,
        PlaneSize::new(72, 40).unwrap(),
        PlaneRect::new(0, 0, 72, 40).unwrap(),
    )
    .unwrap();
    let frame = DecodedFrame::try_new(info, planes).unwrap();
    let grained = visible_planes(&apply_film_grain(&frame, model, seed).unwrap()).unwrap();
    raw_bytes(grained.iter().map(Vec::as_slice), BitDepth::Eight)
}

/// With film grain, TIP output frames code `apply_grain` with a seed and
/// decode to the TIP recon plus the § 7.21.7 grain of the key frame's model
/// and that seed, so grain never reaches the recon.
#[test]
fn tip_output_frames_decode_with_film_grain() {
    let (units, recon, cores) = tip_grain_stream();
    let config = config(72, 40, BitDepth::Eight, 6, 250);
    let [y, cb, cr] = grain::estimate(&grainy_panning_frame(&config, 0), config.chroma).unwrap();
    let model = encoder_film_grain_model(ChromaFormatIdc::Yuv420, [&y, &cb, &cr]).unwrap();
    let decoded = decode(&units);
    assert_eq!(decoded.len(), recon.len());
    let frame_len = 72 * 40 * 3 / 2;
    let mut tips = 0;
    for core in &cores {
        let Some(grain) = core.inter.as_ref().and_then(|inter| inter.tip_film_grain) else {
            continue;
        };
        assert_eq!((grain.apply_grain, grain.fgm_id), (true, Some(0)));
        let index = core.display_order_hint().unwrap() as usize;
        let span = index * frame_len..(index + 1) * frame_len;
        let expected = with_grain(&recon[span.clone()], &model, grain.grain_seed.unwrap());
        assert_ne!(
            expected,
            recon[span.clone()],
            "TIP output {index} has no grain"
        );
        assert!(decoded[span] == expected[..], "TIP output {index}");
        tips += 1;
    }
    assert!(tips > 0);
}

/// AVM decodes TIP output frames with film grain like splot.
#[test]
#[ignore = "needs an AVM decoder build at SPLOT_AVM_DECODER"]
fn avm_decodes_grain_tip_output_like_splot() {
    let Some(avmdec) = avm_decoder() else {
        return;
    };
    let (units, _, cores) = tip_grain_stream();
    assert!(cores.iter().any(|core| {
        core.inter
            .as_ref()
            .is_some_and(|inter| inter.tip_film_grain.is_some())
    }));
    let avm = avm_decode(
        &avmdec,
        std::slice::from_ref(&units),
        (72, 40),
        BitDepth::Eight,
        "tip-grain",
    );
    assert!(avm == decode(&units), "AVM output differs");
}

/// Preset 6 with TIP, TIP output and hole fill.
fn with_hole_fill() -> &'static PresetParams {
    Box::leak(Box::new(PresetParams {
        tip_hole_fill: true,
        ..*with_tip_output(1, 200)
    }))
}

/// A panning mini-GOP stream with `tip_hole_fill` on `threads` threads in
/// 2x1 tiles.
fn hole_fill_stream(threads: usize) -> (Vec<u8>, Vec<u8>, Vec<FrameHeaderCore>, IntraFrameCoder) {
    let mut config = config(136, 72, BitDepth::Eight, 6, 250);
    config.tiles = TileLayout::Uniform {
        cols_log2: 1,
        rows_log2: 0,
    };
    config.threads = splot_parallel::ThreadCount::from(threads);
    let coder = IntraFrameCoder::with_preset(&config, with_hole_fill()).unwrap();
    super::carry::planned_stream_of(&config, coder, panning_frame, 250)
}

/// With `tip_hole_fill` the sequence enables hole fill, TIP frames (as a
/// reference and as output) code `allow_tip_hole_fill = 1`, the stream
/// decodes to the encoder recon, and 1, 2 and 8 threads write the same bytes.
#[test]
fn hole_fill_streams_decode_to_the_encoder_recon_on_any_thread_count() {
    let (one, expected, cores, coder) = hole_fill_stream(1);
    assert!(coder.sequence.inter.unwrap().enable_tip_hole_fill);
    let filled: Vec<_> = cores
        .iter()
        .filter_map(|core| core.inter.as_ref())
        .filter(|inter| {
            inter
                .tip_frame_mode
                .is_some_and(|m| m != TipFrameMode::Disabled)
        })
        .map(|inter| (inter.tip_frame_mode, inter.allow_tip_hole_fill))
        .collect();
    assert!(!filled.is_empty());
    assert!(
        filled.iter().all(|&(_, fill)| fill == Some(true)),
        "{filled:?}"
    );
    assert!(
        filled
            .iter()
            .any(|&(mode, _)| mode == Some(TipFrameMode::AsOutput)),
        "{filled:?}"
    );
    assert_same_samples(&decode(&one), &expected, "hole fill");
    for threads in [2, 8] {
        assert!(hole_fill_stream(threads).0 == one, "{threads} threads");
    }
}

/// AVM decodes hole-filled TIP streams like splot.
#[test]
#[ignore = "needs an AVM decoder build at SPLOT_AVM_DECODER"]
fn avm_decodes_hole_fill_streams_like_splot() {
    let Some(avmdec) = avm_decoder() else {
        return;
    };
    let (units, expected, ..) = hole_fill_stream(2);
    let avm = avm_decode(&avmdec, &[units], (136, 72), BitDepth::Eight, "hole-fill");
    assert!(avm == expected, "AVM output differs");
}

/// A tiled 10-bit noisy [`zooming_frame`] mini-GOP stream at preset 6 with
/// the TIP `NEWMV` search and every BAWP variant on, at `qp` on `threads`
/// threads: the units, the recon, the headers and the
/// [`crate::tile_writer::CODED_SINGLE`] counts of the final pass.
fn tip_bawp_stream(
    qp: u16,
    threads: usize,
    mvd_tools: [bool; 3],
) -> (Vec<u8>, Vec<u8>, Vec<FrameHeaderCore>, [usize; 4]) {
    let preset: &'static PresetParams = Box::leak(Box::new(PresetParams {
        tip_newmv: true,
        tip_amvd: true,
        mvd_tools,
        bawp: true,
        bawp_search: 2,
        ..*Preset::new(6).unwrap().params()
    }));
    let mut config = config(136, 72, BitDepth::Ten, 6, qp);
    config.tiles = TileLayout::Uniform {
        cols_log2: 1,
        rows_log2: 0,
    };
    config.threads = splot_parallel::ThreadCount::from(threads);
    let coder = IntraFrameCoder::with_preset(&config, preset).unwrap();
    crate::tile_writer::CODED_SINGLE.set([0; 4]);
    crate::mode_decision::TIP_NEWMV_ON_NEAR.set(0);
    let noisy = |config: &Config, index: u32| {
        super::grain_streams::with_noise(config, &zooming_frame(config, index), 3 + index)
    };
    let (units, expected, cores, _) = super::carry::planned_stream_of(&config, coder, noisy, qp);
    (
        units,
        expected,
        cores,
        crate::tile_writer::CODED_SINGLE.get(),
    )
}

/// The search codes TIP `NEWMV` leaves, BAWP `NEARMV` leaves and BAWP leaves
/// without chroma weighting in frames that later frames reference, with the
/// deblocking filter on; leaves whose TIP `NEWMV` search ends on a `NEARMV`
/// vector add no `NEWMV` candidate, so they code `NEARMV`; the stream decodes
/// to the encoder recon and codes the same bytes on 1, 2 and 8 threads.
#[test]
fn tip_newmv_and_bawp_variants_decode_on_any_thread_count() {
    let (one, expected, cores, coded) = tip_bawp_stream(180, 1, [false; 3]);
    assert!(coded[..3].iter().all(|&count| count > 0), "{coded:?}");
    let on_near = crate::mode_decision::TIP_NEWMV_ON_NEAR.get();
    assert!(on_near > 0, "no TIP NEWMV search ended on a NEARMV vector");
    assert!(cores.iter().any(|core| {
        core.deblocking_filter_params
            .as_ref()
            .is_some_and(|params| params.apply_deblocking_filter.iter().any(|&on| on))
    }));
    assert_same_samples(&decode(&one), &expected, "TIP NEWMV and BAWP");
    for threads in [2, 8] {
        assert!(
            tip_bawp_stream(180, threads, [false; 3]).0 == one,
            "{threads} threads"
        );
    }
}

/// With adaptive MVD the TIP `NEWMV` search codes `use_amvd` TIP leaves in
/// frames that later frames reference; the stream decodes to the encoder
/// recon and codes the same bytes on 1, 2 and 8 threads.
#[test]
fn tip_amvd_decodes_on_any_thread_count() {
    let (one, expected, _, coded) = tip_bawp_stream(180, 1, [true; 3]);
    assert!(coded[3] > 0, "{coded:?}");
    assert_same_samples(&decode(&one), &expected, "TIP AMVD");
    for threads in [2, 8] {
        assert!(
            tip_bawp_stream(180, threads, [true; 3]).0 == one,
            "{threads} threads"
        );
    }
}

/// AVM decodes the TIP `NEWMV` and BAWP variant stream like splot.
#[test]
#[ignore = "needs an AVM decoder build at SPLOT_AVM_DECODER"]
fn avm_decodes_tip_newmv_and_bawp_variants_like_splot() {
    let Some(avmdec) = avm_decoder() else {
        return;
    };
    for mvd_tools in [[false; 3], [true; 3]] {
        let (units, ..) = tip_bawp_stream(180, 1, mvd_tools);
        let avm = avm_decode(
            &avmdec,
            std::slice::from_ref(&units),
            (136, 72),
            BitDepth::Ten,
            "tip-bawp",
        );
        assert!(avm == decode(&units), "{mvd_tools:?}: AVM output differs");
    }
}

/// Preset 6 with TIP output, deblocking of prediction sub-units, explicit
/// TIP quantization and the TIP output search at `level`.
fn with_tip_output_options(level: u8) -> &'static PresetParams {
    Box::leak(Box::new(PresetParams {
        df_sub_pu: true,
        tip_explicit_qp: true,
        tip_output_search: level,
        ..*with_tip_output(1, 200)
    }))
}

/// A panning mini-GOP stream with the TIP output options on `threads`
/// threads in 2x1 tiles; `forced` sets what every TIP output frame codes.
fn tip_output_options_stream(
    threads: usize,
    level: u8,
    forced: Option<(TipGlobalMv, Option<InterpolationFilter>, bool)>,
) -> (Vec<u8>, Vec<u8>, Vec<FrameHeaderCore>, IntraFrameCoder) {
    let mut config = config(136, 72, BitDepth::Eight, 6, 250);
    config.tiles = TileLayout::Uniform {
        cols_log2: 1,
        rows_log2: 0,
    };
    config.threads = splot_parallel::ThreadCount::from(threads);
    crate::intra_frame::TIP_OUTPUT_FORCED.set(forced);
    let coder = IntraFrameCoder::with_preset(&config, with_tip_output_options(level)).unwrap();
    let stream = super::carry::planned_stream_of(&config, coder, panning_frame, 250);
    crate::intra_frame::TIP_OUTPUT_FORCED.set(None);
    stream
}

/// TIP output frames that code a non-zero global MV, each TIP filter, TIP
/// deblocking and explicit quantization decode to the encoder recon and
/// code the same bytes on 1, 2 and 8 threads; the searched levels 1 and 2
/// decode to the encoder recon too.
#[test]
fn tip_output_options_decode_to_the_encoder_recon_on_any_thread_count() {
    for forced in [
        (TipGlobalMv::new(2, -1), None, false),
        (
            TipGlobalMv::default(),
            Some(InterpolationFilter::Eighttap),
            true,
        ),
        (
            TipGlobalMv::new(0, 2),
            Some(InterpolationFilter::EighttapSmooth),
            true,
        ),
    ] {
        let (one, expected, cores, coder) = tip_output_options_stream(1, 0, Some(forced));
        assert!(coder.sequence.inter.unwrap().enable_tip_explicit_qp);
        let outputs: Vec<_> = cores
            .iter()
            .filter(|core| core.obu_type == splot_core::types::ObuType::RegularTip)
            .collect();
        assert!(!outputs.is_empty(), "{forced:?}");
        for core in outputs {
            let inter = core.inter.as_ref().unwrap();
            let filter = forced.1.unwrap_or(InterpolationFilter::EighttapSharp);
            assert_eq!(inter.tip_global_mv, Some(forced.0));
            assert_eq!(inter.tip_interpolation_filter, Some(filter));
            assert_eq!(inter.apply_deblocking_filter_tip, Some(forced.2));
            assert!(core.quantization_params.is_some());
        }
        assert_same_samples(&decode(&one), &expected, &format!("{forced:?}"));
        for threads in [2, 8] {
            let stream = tip_output_options_stream(threads, 0, Some(forced)).0;
            assert!(stream == one, "{forced:?}: {threads} threads");
        }
    }
    for level in [1, 2] {
        let (units, expected, ..) = tip_output_options_stream(1, level, None);
        assert_same_samples(&decode(&units), &expected, &format!("level {level}"));
    }
}

/// AVM decodes TIP output frames with a global MV, the smooth filter, TIP
/// deblocking and explicit quantization like splot.
#[test]
#[ignore = "needs an AVM decoder build at SPLOT_AVM_DECODER"]
fn avm_decodes_tip_output_options_like_splot() {
    let Some(avmdec) = avm_decoder() else {
        return;
    };
    let forced = (
        TipGlobalMv::new(0, 2),
        Some(InterpolationFilter::EighttapSmooth),
        true,
    );
    let (units, expected, ..) = tip_output_options_stream(2, 0, Some(forced));
    let avm = avm_decode(&avmdec, &[units], (136, 72), BitDepth::Eight, "tip-options");
    assert!(avm == expected, "AVM output differs");
}

/// With `short_refresh` the sequence codes `enable_short_refresh_frame_flags`;
/// inter and TIP output frames code the one slot they refresh, or none, and
/// the mini-GOP stream is shorter than without it, decodes to the encoder
/// recon, and has the same bytes on 1, 2 and 8 threads.
#[test]
fn short_refresh_streams_decode_to_the_encoder_recon_on_any_thread_count() {
    let stream = |short_refresh: bool, threads: usize| {
        let mut config = config(72, 40, BitDepth::Eight, 6, 250);
        config.threads = splot_parallel::ThreadCount::from(threads);
        let params = Box::leak(Box::new(PresetParams {
            short_refresh,
            ..*with_tip_output(1, 200)
        }));
        let coder = IntraFrameCoder::with_preset(&config, params).unwrap();
        super::carry::planned_stream_of(&config, coder, panning_frame, 250)
    };
    let (units, expected, cores, coder) = stream(true, 1);
    assert!(
        coder
            .sequence
            .inter
            .unwrap()
            .enable_short_refresh_frame_flags
    );
    let refreshed: Vec<u32> = cores
        .iter()
        .filter(|core| core.frame_type != Some(splot_core::headers::frame::FrameType::Key))
        .map(|core| core.refresh_frame_flags.unwrap().count_ones())
        .collect();
    assert!(
        refreshed.contains(&0) && refreshed.contains(&1),
        "{refreshed:?}"
    );
    assert!(refreshed.iter().all(|&slots| slots <= 1));
    assert!(
        cores
            .iter()
            .any(|core| core.obu_type == splot_core::types::ObuType::RegularTip)
    );
    assert_same_samples(&decode(&units), &expected, "short refresh");
    assert!(units.len() < stream(false, 1).0.len());
    for threads in [2, 8] {
        assert!(stream(true, threads).0 == units, "{threads} threads");
    }
}
