// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Motion vector prediction and coding: temporal motion vectors (§ 7.9), the
//! reference MV bank, DRL reordering (§ 7.12), the DRL depth (§ 5.4.6) and
//! switchable interpolation filters (§ 5.18.5.1).

use super::*;
use crate::gop::GopStep;
use crate::preset::PresetParams;
use crate::rd::coeff_rate::RateSink;
use crate::tile_writer::Walk;
use splot_core::headers::frame::InterpolationFilter as FrameFilter;
use splot_core::headers::sequence::DrlReorder;

/// `preset` without MV prediction.
fn without_mv_prediction(preset: &PresetParams) -> &'static PresetParams {
    Box::leak(Box::new(PresetParams {
        mv_prediction: false,
        ..*preset
    }))
}

/// The square node of `size` luma samples at the top-left of `root`.
fn top_left(syntax: &IntraTileSyntax, root: PartitionNode, size: usize) -> PartitionNode {
    let mut node = root;
    loop {
        let (w, h) = node.dimensions().unwrap();
        let partition = match (w > h, w > size) {
            (true, _) => Partition::Vert,
            (false, true) => Partition::Horz,
            (false, false) => return node,
        };
        node = syntax.children(node, partition).unwrap()[0];
    }
}

/// With MV prediction the sequence enables temporal motion vectors, the bank
/// and DRL reordering, every inter frame takes the reference motion fields,
/// and the stream decodes to the encoder recon. Without it none of them is
/// coded.
#[test]
fn mv_prediction_reaches_the_headers_and_the_decoder() {
    let config = config(72, 40, BitDepth::Ten, 6, 110);
    let on = Preset::new(6).unwrap().params();
    assert!(on.mv_prediction && on.drl_bits > 1);
    let mut streams = Vec::new();
    for (preset, tools) in [(on, true), (without_mv_prediction(on), false)] {
        let (units, expected, cores, coder) = super::carry::planned_stream(&config, preset);
        let inter = coder.sequence.inter.unwrap();
        assert_eq!(inter.enable_ref_frame_mvs, tools);
        assert_eq!(inter.enable_refmvbank, tools);
        assert_eq!(inter.drl_reorder != DrlReorder::Disabled, tools);
        let drl_bits_minus_1 = u32::from(preset.drl_bits - 1);
        assert_eq!(inter.seq_max_drl_bits_minus_1, drl_bits_minus_1);
        let frames: Vec<_> = cores
            .iter()
            .filter_map(|core| core.inter.as_ref())
            .collect();
        assert!(frames.len() >= 7, "{} inter frames", frames.len());
        for header in frames {
            assert_eq!(header.use_ref_frame_mvs, Some(tools));
            assert_eq!(header.max_drl_bits_minus_1, Some(drl_bits_minus_1));
        }
        assert_same_samples(&decode(&units), &expected, &format!("tools {tools}"));
        streams.push(units);
    }
    assert_ne!(streams[0], streams[1]);
}

/// The first leaf of a frame has no spatial neighbour and an empty bank, so
/// a moving candidate in its § 7.12 stack is a temporal candidate that the
/// decoder projects from the motion field the encoder stored for the hidden
/// alt-ref of a pan. Without MV prediction the stack holds zero vectors.
#[test]
fn temporal_candidates_reach_the_first_leaf_of_a_frame() {
    let config = config(72, 40, BitDepth::Eight, 6, 110);
    let on = Preset::new(6).unwrap().params();
    for (preset, tools) in [(on, true), (without_mv_prediction(on), false)] {
        let mut coder = IntraFrameCoder::with_preset(&config, preset).unwrap();
        let mut planner = crate::gop::GopPlanner::new(config.keyint, 4);
        let mut steps = planner.plan(1).unwrap();
        steps.extend(planner.plan(4).unwrap());
        let mut plans = steps.into_iter().filter_map(|step| match step {
            GopStep::Code(plan) => Some(plan),
            GopStep::ShowExisting { .. } => None,
        });
        for plan in plans.by_ref().take(2) {
            let source = panning_frame(&config, plan.display_index as u32);
            coder.encode_frame(&source, &plan, 110).unwrap();
        }
        let plan = plans.next().unwrap();
        let tools_of = crate::gop::frame_tools(coder.preset, coder.tiles, 110);
        let core = plan
            .header(
                &coder.view,
                &coder.references,
                110,
                &FrameFilterParams::default(),
                tools_of,
            )
            .unwrap();
        assert!(core.inter.as_ref().unwrap().num_total_refs.unwrap() >= 2);
        let mut writer =
            KeyFrameTileWriter::new(&coder.sequence, &core, &coder.references, 0).unwrap();
        let stacks = writer
            .search_superblock(|syntax, root| {
                let leaf = syntax.leaf(top_left(syntax, root, 16))?;
                let mut stacks = Vec::new();
                for ref_frame in 0..syntax.reference_count() {
                    stacks.extend(syntax.single_mvs(&leaf, ref_frame)?.stack);
                }
                Ok(stacks)
            })
            .unwrap();
        let moving = stacks.iter().any(|&mv| mv != MotionVector::default());
        assert_eq!(moving, tools, "{stacks:?}");
    }
}

/// A rollback puts the reference MV bank back: a trial leaf whose vector
/// enters the bank and is rolled back leaves no candidate behind, so the
/// stack of a leaf without spatial neighbours in the next superblock is the
/// same as without the trial.
#[test]
fn rollback_restores_the_reference_mv_bank() {
    let config = config(136, 64, BitDepth::Eight, 9, 110);
    let mut coder = IntraFrameCoder::new(&config).unwrap();
    coder
        .encode_frame(
            &panning_frame(&config, 0),
            &crate::gop::plan(0, config.keyint),
            110,
        )
        .unwrap();
    let plan = crate::gop::plan(1, config.keyint);
    let tools = crate::gop::frame_tools(coder.preset, coder.tiles, 110);
    let filters = FrameFilterParams::default();
    let core = plan
        .header(&coder.view, &coder.references, 110, &filters, tools)
        .unwrap();
    let mut writer = KeyFrameTileWriter::new(&coder.sequence, &core, &coder.references, 0).unwrap();
    let (mi_rows, mi_cols) = writer.syntax().mi_dimensions();
    let sb = writer.syntax().superblock_size4() * 4;
    let source = panning_frame(&config, 1);
    let padded = PaddedSource::new(&source, coder.chroma, mi_cols * 4 + sb, mi_rows * 4 + sb);
    let lambda = Lambda::from_qindex(110, config.bit_depth).unwrap();
    let mut residual =
        SourceResidualCoder::new(&padded, 110, config.bit_depth, coder.preset, lambda);
    let (with_trial, without, trial) = writer
        .search_superblock(|syntax, root| {
            let sb4 = syntax.superblock_size4();
            let leaf = syntax.leaf(top_left(syntax, root, 8))?;
            let next = syntax.superblock_root(0, sb4);
            let target = syntax.leaf(top_left(syntax, next, 8))?;
            let start = syntax.snapshot(root)?;
            let mut code = |syntax: &mut IntraTileSyntax, row: i32, col: i32| {
                let mv = MotionVector { row, col };
                let choice = LeafChoice::inter(InterPrediction::Single(InterModes {
                    ref_frame: 0,
                    mode: InterMode::New,
                    drl: 0,
                    mv,
                    filter: splot_recon::InterpolationFilter::EightTap,
                    precision: None,
                    amvd: false,
                }));
                Walk::trial(syntax, &mut RateSink::adapting()).write_leaf(
                    &leaf,
                    choice,
                    &mut residual,
                    0..3,
                )?;
                Ok::<_, TileWriterError>(syntax.single_mvs(&target, 0)?.stack)
            };
            let without = code(syntax, 16, -24)?;
            syntax.restore(&start)?;
            let trial = code(syntax, -40, 8)?;
            syntax.restore(&start)?;
            let with_trial = code(syntax, 16, -24)?;
            Ok((with_trial, without, trial))
        })
        .unwrap();
    assert!(
        without.contains(&MotionVector { row: 16, col: -24 }),
        "{without:?}"
    );
    assert!(
        trial.contains(&MotionVector { row: -40, col: 8 }),
        "{trial:?}"
    );
    assert_eq!(with_trial, without);
}

/// A mini-GOP stream in 2x2 tiles at a preset with DRL depth above one
/// decodes to the encoder recon and codes the same bytes on 1, 2 and 8
/// threads.
#[test]
fn deep_drl_streams_decode_alike_on_any_thread_count() {
    let stream = |threads: usize| {
        let mut config = config(136, 72, BitDepth::Eight, 6, 100);
        assert!(config.preset.params().drl_bits > 1);
        config.tiles = TileLayout {
            cols_log2: 1,
            rows_log2: 1,
        };
        config.threads = splot_parallel::ThreadCount::from(threads);
        encode_stream(&config, 9)
    };
    let (one, expected) = stream(1);
    assert_same_samples(&decode(&one.concat()), &expected, "deep DRL");
    for threads in [2, 8] {
        assert!(stream(threads).0 == one, "{threads} threads");
    }
}

/// With the filter search every inter frame codes `SWITCHABLE` filters and
/// the stream decodes to the encoder recon; without it the frames code
/// `EIGHTTAP` and the stream differs.
#[test]
fn switchable_filters_reach_the_headers_and_the_decoder() {
    let config = config(72, 40, BitDepth::Ten, 6, 110);
    let on = Preset::new(6).unwrap().params();
    assert!(on.interp_filter_search);
    let off = Box::leak(Box::new(PresetParams {
        interp_filter_search: false,
        ..*on
    }));
    let mut streams = Vec::new();
    for (preset, filter) in [(on, FrameFilter::Switchable), (off, FrameFilter::Eighttap)] {
        let (units, expected, cores, _) = super::carry::planned_stream(&config, preset);
        let frames: Vec<_> = cores
            .iter()
            .filter_map(|core| core.inter.as_ref())
            .collect();
        assert!(frames.len() >= 7, "{} inter frames", frames.len());
        for header in frames {
            assert_eq!(header.interpolation_filter, Some(filter));
        }
        assert_same_samples(&decode(&units), &expected, &format!("{filter:?}"));
        streams.push(units);
    }
    assert_ne!(streams[0], streams[1]);
}

/// With the MVD tools the sequence enables adaptive MVD, MVD sign derivation
/// and flexible block precision, and the stream decodes to the encoder
/// recon; without them none is enabled and the stream differs.
#[test]
fn mvd_tools_reach_the_sequence_and_the_decoder() {
    let config = config(72, 40, BitDepth::Eight, 6, 140);
    let on = Preset::new(6).unwrap().params();
    assert_eq!(on.mvd_tools, [true; 3]);
    let off = Box::leak(Box::new(PresetParams {
        mvd_tools: [false; 3],
        ..*on
    }));
    let mut streams = Vec::new();
    for (preset, tools) in [(on, true), (off, false)] {
        let (units, expected, _, coder) = super::carry::planned_stream(&config, preset);
        let inter = coder.sequence.inter.unwrap();
        assert_eq!(inter.enable_adaptive_mvd, tools);
        assert_eq!(inter.enable_mvd_sign_derive, tools);
        assert_eq!(inter.enable_flex_mvres, tools);
        assert_same_samples(&decode(&units), &expected, &format!("tools {tools}"));
        streams.push(units);
    }
    assert_ne!(streams[0], streams[1]);
}
