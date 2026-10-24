// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Inter leaves wider or taller than 64: the chunked § 5.20.7.23 residual.

use super::*;
use crate::preset::PresetParams;
use crate::rd::coeff_rate::RateSink;
use crate::tile_writer::Walk;

/// Preset 6, which codes 128x128 superblocks, with `large_inter`.
pub(super) fn preset6(large_inter: bool) -> &'static PresetParams {
    Box::leak(Box::new(PresetParams {
        large_inter,
        ..*Preset::new(6).unwrap().params()
    }))
}

/// Runs `test` on the first superblock of the first inter frame of a
/// panning stream at `config`, with the residual coder of that frame.
pub(super) fn in_first_inter_frame<R>(
    config: &Config,
    test: impl FnOnce(
        &mut IntraTileSyntax,
        PartitionNode,
        &mut SourceResidualCoder,
    ) -> core::result::Result<R, TileWriterError>,
) -> R {
    let mut coder = IntraFrameCoder::with_preset(config, preset6(true)).unwrap();
    let key = crate::gop::plan(0, config.keyint);
    coder
        .encode_frame(&panning_frame(config, 0), &key, 110)
        .unwrap();
    let plan = crate::gop::plan(1, config.keyint);
    let tools = crate::gop::frame_tools(coder.preset, coder.tiles, 110);
    let core = plan
        .header(
            &coder.view,
            &coder.references,
            110,
            &FrameFilterParams::default(),
            tools,
        )
        .unwrap();
    let mut writer = KeyFrameTileWriter::new(&coder.sequence, &core, &coder.references, 0).unwrap();
    let (mi_rows, mi_cols) = writer.syntax().mi_dimensions();
    let sb = writer.syntax().superblock_size4() * 4;
    let source = panning_frame(config, 1);
    let padded = PaddedSource::new(&source, coder.chroma, mi_cols * 4 + sb, mi_rows * 4 + sb);
    let lambda = Lambda::from_qindex(110, config.bit_depth).unwrap();
    let mut residual =
        SourceResidualCoder::new(&padded, 110, config.bit_depth, coder.preset, lambda);
    writer
        .search_superblock(|syntax, root| test(syntax, root, &mut residual))
        .unwrap()
}

/// A single-reference prediction from the first reference.
fn single(mode: InterMode, mv: MotionVector) -> InterPrediction {
    InterPrediction::Single(InterModes {
        ref_frame: 0,
        mode,
        drl: 0,
        mv,
        filter: splot_recon::InterpolationFilter::EightTap,
        precision: None,
        amvd: false,
    })
}

/// The facade allows inter on a 128x128 leaf and plans its residual as the
/// decoder reads it: each 64x64 luma chunk, and after the first chunk of
/// each chroma group that group's U and V blocks (§ 5.20.7.23).
#[test]
fn a_128x128_inter_leaf_plans_the_chunked_residual() {
    let y = |x, y| (0, x, y, false);
    let uv = |x, y, fills| [(1, x, y, fills), (2, x, y, fills)];
    let cases = [
        (
            PixelFormat::Yuv420,
            [vec![y(0, 0)], uv(0, 0, true).to_vec()].concat(),
            vec![y(64, 0), y(0, 64), y(64, 64)],
        ),
        (
            PixelFormat::Yuv422,
            [vec![y(0, 0)], uv(0, 0, false).to_vec(), vec![y(64, 0)]].concat(),
            [vec![y(0, 64)], uv(0, 64, false).to_vec(), vec![y(64, 64)]].concat(),
        ),
        (
            PixelFormat::Yuv444,
            [
                vec![y(0, 0)],
                uv(0, 0, false).to_vec(),
                vec![y(64, 0)],
                uv(64, 0, false).to_vec(),
            ]
            .concat(),
            [
                vec![y(0, 64)],
                uv(0, 64, false).to_vec(),
                vec![y(64, 64)],
                uv(64, 64, false).to_vec(),
            ]
            .concat(),
        ),
    ];
    for (chroma, top, bottom) in cases {
        let mut config = config(136, 136, BitDepth::Eight, 6, 110);
        config.chroma = chroma;
        let blocks = in_first_inter_frame(&config, |syntax, root, _| {
            let leaf = syntax.leaf(root)?;
            assert_eq!(leaf.dimensions()?, (128, 128));
            assert!(syntax.inter_allowed(&leaf));
            let prediction = single(InterMode::Global, MotionVector::default());
            Ok(syntax.plan_inter_leaf(&leaf, prediction)?)
        });
        let planned: Vec<_> = blocks
            .iter()
            .map(|tx| (tx.plane, tx.x, tx.y, tx.fills_block))
            .collect();
        assert_eq!(planned, [top, bottom].concat(), "{chroma:?}");
        assert!(
            blocks.iter().all(|tx| tx.tx_size == 4),
            "{chroma:?} TX_64X64"
        );
    }
}

/// A trial 128x128 inter leaf with a coded residual is rolled back whole: the
/// leaf coded after it costs the same bits, reconstructs the same samples
/// and leaves the same § 7.12 stack in the next superblock as without it.
#[test]
fn rollback_restores_a_128x128_inter_leaf() {
    let config = config(200, 136, BitDepth::Ten, 6, 110);
    let (with_trial, without) = in_first_inter_frame(&config, |syntax, root, residual| {
        let leaf = syntax.leaf(root)?;
        let next = syntax.superblock_root(0, syntax.superblock_size4());
        let target = syntax.leaf(super::mvpred::top_left(syntax, next, 8))?;
        let start = syntax.snapshot(root)?;
        let mut code = |syntax: &mut IntraTileSyntax, row: i32, col: i32| {
            let mut sink = RateSink::adapting();
            let choice = LeafChoice::inter(single(InterMode::New, MotionVector { row, col }));
            let blocks =
                Walk::trial(syntax, &mut sink).write_leaf(&leaf, choice, residual, 0..3)?;
            let recon = blocks
                .iter()
                .map(|tx| syntax.reconstruction(tx))
                .collect::<core::result::Result<Vec<_>, _>>()?;
            let stack = syntax.single_mvs(&target, 0)?.stack;
            Ok::<_, TileWriterError>((sink.bits, recon, stack))
        };
        let without = code(syntax, 16, -24)?;
        syntax.restore(&start)?;
        let trial = code(syntax, -40, 8)?;
        assert_ne!(trial.0, without.0, "the trial codes another residual");
        syntax.restore(&start)?;
        Ok((code(syntax, 16, -24)?, without))
    });
    assert_eq!(with_trial, without);
}

/// With `large_inter`, panning streams of every chroma format, both bit
/// depths and 2x1 tiles decode to the encoder recon.
#[test]
fn large_inter_streams_decode_to_the_encoder_recon() {
    let cases = [
        (PixelFormat::Yuv420, BitDepth::Eight, false),
        (PixelFormat::Yuv420, BitDepth::Ten, true),
        (PixelFormat::Yuv422, BitDepth::Ten, false),
        (PixelFormat::Yuv444, BitDepth::Eight, false),
    ];
    for (chroma, bit_depth, tiled) in cases {
        let mut config = config(if tiled { 264 } else { 136 }, 136, bit_depth, 6, 110);
        config.chroma = chroma;
        if tiled {
            config.tiles = TileLayout {
                cols_log2: 1,
                rows_log2: 0,
            };
        }
        let (units, expected, ..) = super::carry::planned_stream(&config, preset6(true));
        let name = format!("{chroma:?} {bit_depth:?} tiled {tiled}");
        assert_same_samples(&decode(&units), &expected, &name);
    }
}

/// A tiled stream with `large_inter` codes the same bytes on 1, 2 and 8
/// threads.
#[test]
fn large_inter_streams_match_on_any_thread_count() {
    let stream = |threads: usize| {
        let mut config = config(264, 136, BitDepth::Eight, 6, 110);
        config.tiles = TileLayout {
            cols_log2: 1,
            rows_log2: 0,
        };
        config.threads = splot_parallel::ThreadCount::from(threads);
        super::carry::planned_stream(&config, preset6(true)).0
    };
    let one = stream(1);
    for threads in [2, 8] {
        assert!(stream(threads) == one, "{threads} threads");
    }
}

/// The inter leaves wider or taller than 64 of the trees of 128x128
/// superblocks: the root leaf and the leaves of a root partition other than
/// `SPLIT`.
fn large_inter_leaves(trees: &[PartitionTree]) -> usize {
    let inter = |tree: &PartitionTree| matches!(tree, PartitionTree::Leaf(choice) if choice.inter.is_some());
    trees
        .iter()
        .map(|tree| match tree {
            PartitionTree::Split { kind, children } if *kind != Partition::Split => {
                children.iter().filter(|child| inter(child)).count()
            }
            tree => usize::from(inter(tree)),
        })
        .sum()
}

/// On a pan, the search of an inter frame picks inter leaves wider or taller
/// than 64 in every chroma format, repeats from one state, and rolls back;
/// without `large_inter` it picks none.
#[test]
fn a_pan_codes_large_inter_leaves() {
    for chroma in CHROMAS {
        let mut config = config(200, 136, BitDepth::Eight, 6, 90);
        config.chroma = chroma;
        for large_inter in [true, false] {
            let mut coder = IntraFrameCoder::with_preset(&config, preset6(large_inter)).unwrap();
            let key = crate::gop::plan(0, config.keyint);
            coder
                .encode_frame(&panning_frame(&config, 0), &key, 90)
                .unwrap();
            let plan = crate::gop::plan(1, config.keyint);
            let trees = search_frame(&coder, &plan, &panning_frame(&config, 1), config.bit_depth);
            let count = large_inter_leaves(&trees);
            assert_eq!(count > 0, large_inter, "{chroma:?} {count}");
        }
    }
}
