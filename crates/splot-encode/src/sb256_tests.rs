// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! 256x256 superblocks in coded streams (§ 6.4.3).

use super::*;
use splot_core::headers::sequence::SuperblockSize;

/// Preset 9 that codes 256x256 superblocks in any frame.
static SB256: PresetParams = PresetParams {
    max_block_log2: 8,
    sb256_min_height: 1,
    ..PRESETS[9]
};

/// `(width, height, bit depth, chroma, (cols_log2, rows_log2), still)`: a
/// still case codes 256x256 skipped leaves over a finer key frame.
type Sb256Case = (u32, u32, BitDepth, PixelFormat, (u8, u8), bool);

const SB256_CASES: [Sb256Case; 4] = [
    (
        320,
        200,
        BitDepth::Eight,
        PixelFormat::Yuv420,
        (1, 0),
        false,
    ),
    (300, 264, BitDepth::Ten, PixelFormat::Yuv422, (0, 0), false),
    (
        264,
        136,
        BitDepth::Eight,
        PixelFormat::Yuv444,
        (0, 0),
        false,
    ),
    (520, 300, BitDepth::Eight, PixelFormat::Yuv420, (0, 0), true),
];

/// Key, inter, inter and key frames of panning content with [`SB256`] on
/// `threads` threads: the units and the encoder recon.
fn sb256_stream(case: Sb256Case, threads: usize) -> (Vec<Vec<u8>>, Vec<u8>) {
    let (width, height, bit_depth, chroma, (cols_log2, rows_log2), still) = case;
    let mut config = config(width, height, bit_depth, 9, 140);
    config.chroma = chroma;
    config.keyint = core::num::NonZeroU32::new(3).unwrap();
    config.tiles = TileLayout {
        cols_log2,
        rows_log2,
    };
    config.threads = splot_parallel::ThreadCount::from(threads);
    let mut coder = IntraFrameCoder::with_preset(&config, &SB256).unwrap();
    let sb_size = coder.sequence.partition.unwrap().seq_sb_size();
    assert_eq!(sb_size, SuperblockSize::Block256x256);
    let (mut units, mut recon) = (Vec::new(), Vec::new());
    for index in 0..4 {
        let plan = crate::gop::plan(u64::from(index), config.keyint);
        let (source, qp) = if still {
            (ramp(&config), if index == 0 { 60 } else { 160 })
        } else {
            (panning_frame(&config, index), 140)
        };
        let coded = coder.encode_frame(&source, &plan, qp).unwrap();
        recon.extend(recon_bytes(&coded, bit_depth));
        units.push(coded.temporal_unit);
    }
    (units, recon)
}

/// Inter frames with 256x256 superblocks between 128x128-superblock key
/// frames decode to the encoder recon with the frame filters, in every
/// chroma format, at frame edges, with tiles and with 256x256 leaves, and 1,
/// 2 and 8 threads write the same bytes.
#[test]
fn sb256_streams_decode_to_the_encoder_recon_on_any_thread_count() {
    for case in SB256_CASES {
        let (one, expected) = sb256_stream(case, 1);
        assert_same_samples(&decode(&one.concat()), &expected, &format!("{case:?}"));
        for threads in [2, 8] {
            assert!(
                sb256_stream(case, threads).0 == one,
                "{case:?} {threads} threads"
            );
        }
    }
}

/// A still diagonal ramp.
fn ramp(config: &Config) -> SourceFrame {
    let sizes = config.plane_sizes().unwrap();
    let planes = core::array::from_fn(|plane| {
        let size = sizes[plane];
        (0..size.height())
            .flat_map(|y| (0..size.width()).map(move |x| (40 + (x + y) / 4 + plane * 9) as u16))
            .collect()
    });
    SourceFrame::new(config, 0, planes).unwrap()
}

/// The search of an inter frame of a still ramp over a finer key frame codes
/// leaves wider than 128 in its 256x256 superblocks; a short frame or
/// `max_block_log2` 7 codes 128x128 superblocks.
#[test]
fn sb256_search_codes_256_wide_inter_leaves() {
    let config = config(512, 256, BitDepth::Eight, 9, 140);
    let mut coder = IntraFrameCoder::with_preset(&config, &SB256).unwrap();
    let key = crate::gop::plan(0, config.keyint);
    coder.encode_frame(&ramp(&config), &key, 60).unwrap();
    let plan = crate::gop::plan(1, config.keyint);
    let trees = search_frame(&coder, &plan, &ramp(&config), config.bit_depth);
    assert!(widest_leaf(&trees) > 128, "no leaf wider than 128");
    for preset in [
        PresetParams {
            sb256_min_height: 257,
            ..SB256
        },
        PresetParams {
            max_block_log2: 7,
            ..SB256
        },
    ] {
        let coder = IntraFrameCoder::with_preset(&config, &preset).unwrap();
        let sb_size = coder.sequence.partition.unwrap().seq_sb_size();
        assert_eq!(sb_size, SuperblockSize::Block128x128);
    }
}

/// The widest leaf of `trees` of 256-wide superblocks.
fn widest_leaf(trees: &[PartitionTree]) -> usize {
    fn widest(tree: &PartitionTree, width: usize) -> usize {
        match tree {
            PartitionTree::Leaf(_) => width,
            PartitionTree::Split { kind, children } => {
                let child = match kind {
                    Partition::Horz => width,
                    _ => width / 2,
                };
                children.iter().map(|c| widest(c, child)).max().unwrap_or(0)
            }
            PartitionTree::Sdp { luma, .. } => widest(luma, width),
        }
    }
    trees
        .iter()
        .map(|tree| widest(tree, 256))
        .max()
        .unwrap_or(0)
}

/// AVM decodes streams with 256x256 superblocks to the encoder recon.
#[test]
#[ignore = "needs an AVM decoder build at SPLOT_AVM_DECODER"]
fn avm_decodes_sb256_streams_like_splot() {
    let Some(avmdec) = avm_decoder() else {
        return;
    };
    for case in SB256_CASES {
        let (units, expected) = sb256_stream(case, 2);
        let avm = avm_decode(&avmdec, &units, (case.0, case.1), case.2, "sb256");
        assert!(avm == expected, "{case:?}: AVM output differs");
    }
}
