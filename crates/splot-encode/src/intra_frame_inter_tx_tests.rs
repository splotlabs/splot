// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Luma transform partitions of inter leaves (§ 5.20.6.2, § 5.20.6.3).

use super::large_inter::{in_first_inter_frame, preset6};
use super::mvpred::top_left;
use super::*;
use crate::preset::PresetParams;
use splot_decode::encoder_syntax::TxPartition;

/// Preset 6 with `inter_tx_split`.
fn with_split(inter_tx_split: u8) -> &'static PresetParams {
    Box::leak(Box::new(PresetParams {
        inter_tx_split,
        ..*preset6(true)
    }))
}

/// A luma transform block: `(x, y, width)`.
type Luma = (usize, usize, usize);

/// A 32x32 inter leaf plans each luma transform partition as the decoder's
/// records, in their scan order, before its unchanged chroma blocks; a
/// 128x128 inter leaf has none but `NONE`.
#[test]
fn inter_tx_partitions_plan_the_decoder_records() {
    let config = config(136, 136, BitDepth::Eight, 6, 110);
    let (planned, large) = in_first_inter_frame(&config, preset6(true), |syntax, root, _| {
        let large = syntax.luma_tx_partitions(&syntax.leaf(root)?)?;
        let leaf = syntax.leaf(top_left(syntax, root, 32))?;
        let prediction = InterPrediction::Single(InterModes {
            ref_frame: 0,
            mode: InterMode::Global,
            drl: 0,
            mv: MotionVector::default(),
            filter: splot_recon::InterpolationFilter::EightTap,
            precision: None,
            amvd: false,
        });
        let mut planned = Vec::new();
        for partition in syntax.luma_tx_partitions(&leaf)? {
            let blocks = syntax.plan_inter_leaf_partitioned(&leaf, prediction, partition)?;
            planned.push((partition, blocks));
        }
        Ok((planned, large))
    });
    assert_eq!(large, [TxPartition::None]);
    assert_eq!(planned.len(), 8, "{planned:?}");
    let chroma = |blocks: &[TransformBlock]| -> Vec<TransformBlock> {
        blocks.iter().filter(|tx| tx.plane > 0).copied().collect()
    };
    let none = &planned[0].1;
    let expected: [(TxPartition, &[Luma]); 8] = [
        (TxPartition::None, &[(0, 0, 32)]),
        (
            TxPartition::Split,
            &[(0, 0, 16), (16, 0, 16), (0, 16, 16), (16, 16, 16)],
        ),
        (TxPartition::Horz, &[(0, 0, 32), (0, 16, 32)]),
        (TxPartition::Vert, &[(0, 0, 16), (16, 0, 16)]),
        (
            TxPartition::Horz4,
            &[(0, 0, 32), (0, 8, 32), (0, 16, 32), (0, 24, 32)],
        ),
        (
            TxPartition::Vert4,
            &[(0, 0, 8), (8, 0, 8), (16, 0, 8), (24, 0, 8)],
        ),
        (
            TxPartition::Horz5,
            &[
                (0, 0, 16),
                (16, 0, 16),
                (0, 8, 32),
                (0, 24, 16),
                (16, 24, 16),
            ],
        ),
        (
            TxPartition::Vert5,
            &[(0, 0, 8), (0, 16, 8), (8, 0, 16), (24, 0, 8), (24, 16, 8)],
        ),
    ];
    for ((partition, blocks), (want, luma)) in planned.iter().zip(expected) {
        assert_eq!(*partition, want);
        let got: Vec<_> = blocks
            .iter()
            .filter(|tx| tx.plane == 0)
            .map(|tx| (tx.x, tx.y, 1 << tx_log2(tx.tx_size).unwrap().0))
            .collect();
        assert_eq!(got, luma, "{partition:?}");
        assert_eq!(chroma(blocks), chroma(none), "{partition:?}");
        assert_eq!(
            blocks.first().map(|tx| tx.fills_block),
            Some(want == TxPartition::None)
        );
    }
}

/// The inter leaves of `trees` that code a luma transform partition.
fn split_inter_leaves(trees: &[PartitionTree]) -> usize {
    fn count(tree: &PartitionTree) -> usize {
        match tree {
            PartitionTree::Leaf(choice) => {
                usize::from(choice.inter.is_some() && choice.tx_partition != TxPartition::None)
            }
            PartitionTree::Split { children, .. } => children.iter().map(count).sum(),
        }
    }
    trees.iter().map(count).sum()
}

/// With `inter_tx_split` the search of an inter frame over a pan with fresh
/// luma noise in strips picks inter leaves with a luma transform partition in
/// every chroma format, and finds the same tree when it searches again from
/// the restored state; without it it picks none.
#[test]
fn the_search_picks_inter_tx_partitions() {
    for chroma in CHROMAS {
        let mut config = config(136, 72, BitDepth::Eight, 6, 90);
        config.chroma = chroma;
        let max = u32::from(config.bit_depth.max_sample());
        let frame = |index: u32| {
            let mut planes = panning_frame(&config, index).planes().clone();
            for (at, sample) in planes[0].iter_mut().enumerate() {
                if at / 8 % 5 == 0 {
                    let noise = texture(at as i64, i64::from(index) * 7919, 0, max / 2);
                    *sample = (u32::from(*sample) + u32::from(noise)).min(max) as u16;
                }
            }
            SourceFrame::new(&config, u64::from(index), planes).unwrap()
        };
        for split in [24, 0] {
            let mut coder = IntraFrameCoder::with_preset(&config, with_split(split)).unwrap();
            let key = crate::gop::plan(0, config.keyint);
            coder.encode_frame(&frame(0), &key, 90).unwrap();
            let plan = crate::gop::plan(1, config.keyint);
            let trees = search_frame(&coder, &plan, &frame(1), config.bit_depth);
            let count = split_inter_leaves(&trees);
            assert_eq!(count > 0, split > 0, "{chroma:?} {count}");
        }
    }
}

/// With `inter_tx_split`, panning streams of every chroma format and both
/// bit depths decode to the encoder recon, and a tiled stream codes the same
/// bytes on 1, 2 and 8 threads.
#[test]
fn inter_tx_partition_streams_decode_alike_on_any_thread_count() {
    for (chroma, bit_depth) in [
        (PixelFormat::Yuv420, BitDepth::Ten),
        (PixelFormat::Yuv422, BitDepth::Eight),
        (PixelFormat::Yuv444, BitDepth::Ten),
    ] {
        let mut config = config(96, 72, bit_depth, 6, 110);
        config.chroma = chroma;
        let (units, expected, ..) = super::carry::planned_stream(&config, with_split(24));
        assert_same_samples(&decode(&units), &expected, &format!("{chroma:?}"));
    }
    let stream = |threads: usize| {
        let mut config = config(136, 72, BitDepth::Eight, 6, 110);
        config.tiles = TileLayout {
            cols_log2: 1,
            rows_log2: 0,
        };
        config.threads = splot_parallel::ThreadCount::from(threads);
        super::carry::planned_stream(&config, with_split(24))
    };
    let (one, expected, ..) = stream(1);
    assert_same_samples(&decode(&one), &expected, "tiled");
    for threads in [2, 8] {
        assert!(stream(threads).0 == one, "{threads} threads");
    }
}

/// The quadrant errors of the SIMD `sse` kernel equal a scalar sum over
/// asymmetric random samples, for square, wide and tall blocks.
#[test]
fn quadrant_sse_matches_a_scalar_sum() {
    let config = config(80, 72, BitDepth::Ten, 6, 110);
    let source = synthetic_frame(&config, 7);
    let padded = PaddedSource::new(&source, config.chroma, 96, 80);
    let (luma, stride, ..) = padded.luma();
    let mut state = 0x9E37_79B9_u32;
    for (x, y, w, h) in [
        (8, 4, 64, 64),
        (16, 8, 32, 8),
        (4, 0, 8, 64),
        (12, 20, 16, 32),
    ] {
        let prediction: Vec<u16> = (0..w * h)
            .map(|i| {
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
                ((state >> 8) as usize % 1024 + i % 7) as u16
            })
            .collect();
        let got = crate::mode_decision::quadrant_sse(&padded, (x, y, w, h), &prediction).unwrap();
        let mut want = [0u64; 4];
        for row in 0..h {
            for col in 0..w {
                let quadrant = usize::from(col >= w / 2) + 2 * usize::from(row >= h / 2);
                let diff = luma[(y + row) * stride + x + col].abs_diff(prediction[row * w + col]);
                want[quadrant] += u64::from(diff).pow(2);
            }
        }
        assert_eq!(got, want, "{w}x{h} at ({x}, {y})");
    }
}
