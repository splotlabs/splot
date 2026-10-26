// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Semi-decoupled luma and chroma partition trees in coded streams.

use super::*;
use crate::tile_writer::Partition;

/// `(width, height, bit depth, preset, chroma, (cols_log2, rows_log2))`: a
/// 64x64 superblock with two tiles, then 128x128 superblocks in 4:2:2 and
/// 4:4:4 at odd sizes.
type SdpCase = (u32, u32, BitDepth, u8, PixelFormat, (u8, u8));

const SDP_CASES: [SdpCase; 3] = [
    (136, 72, BitDepth::Eight, 9, PixelFormat::Yuv420, (1, 0)),
    (97, 70, BitDepth::Ten, 6, PixelFormat::Yuv422, (0, 0)),
    (72, 40, BitDepth::Eight, 6, PixelFormat::Yuv444, (0, 1)),
];

/// Codes key frame, inter frame, key frame of panning content with SDP on
/// `threads` threads; returns the units and the encoder recon.
fn sdp_stream(case: SdpCase, threads: usize) -> (Vec<Vec<u8>>, Vec<u8>) {
    let (width, height, bit_depth, preset, chroma, (cols_log2, rows_log2)) = case;
    let mut config = config(width, height, bit_depth, preset, 110);
    config.chroma = chroma;
    config.keyint = core::num::NonZeroU32::new(2).unwrap();
    config.tiles = TileLayout {
        cols_log2,
        rows_log2,
    };
    config.threads = splot_parallel::ThreadCount::from(threads);
    let preset = PresetParams {
        sdp: true,
        ..*config.preset.params()
    };
    let mut coder = IntraFrameCoder::with_preset(&config, &preset).unwrap();
    assert!(coder.sequence.partition.unwrap().enable_sdp);
    let coded: Vec<CodedFrame> = (0..3)
        .map(|index| {
            let plan = crate::gop::plan(u64::from(index), config.keyint);
            coder
                .encode_frame(&panning_frame(&config, index), &plan, 110)
                .unwrap()
        })
        .collect();
    let expected = coded
        .iter()
        .flat_map(|frame| recon_bytes(frame, bit_depth))
        .collect();
    (
        coded.into_iter().map(|f| f.temporal_unit).collect(),
        expected,
    )
}

/// SDP key frames and the inter frames between them decode to the encoder
/// recon in every chroma format and with tiles, and 1, 2 and 8 threads
/// write the same bytes.
#[test]
fn sdp_streams_decode_to_the_encoder_recon_on_any_thread_count() {
    for case in SDP_CASES {
        let (one, expected) = sdp_stream(case, 1);
        assert_same_samples(&decode(&one.concat()), &expected, &format!("{case:?}"));
        for threads in [2, 8] {
            assert!(sdp_stream(case, threads).0 == one, "{case:?} {threads} threads");
        }
    }
}

/// Whether `chroma` only merges nodes of `luma`: each chroma split is the
/// luma partition of the same node.
fn merges(chroma: &PartitionTree, luma: &PartitionTree) -> bool {
    match (chroma, luma) {
        (PartitionTree::Leaf(_), _) => true,
        (
            PartitionTree::Split { kind, children },
            PartitionTree::Split {
                kind: luma_kind,
                children: luma_children,
            },
        ) => {
            kind == luma_kind
                && children
                    .iter()
                    .zip(luma_children)
                    .all(|(chroma, luma)| merges(chroma, luma))
        }
        _ => false,
    }
}

/// The SDP search codes every 64x64 node of a key frame as a luma tree and
/// a chroma tree that merges luma nodes, and merges some; without SDP it
/// codes no SDP tree.
#[test]
fn sdp_search_codes_chroma_trees_that_merge_luma_nodes() {
    let config = config(128, 128, BitDepth::Eight, 6, 90);
    let key = crate::gop::plan(0, config.keyint);
    let source = synthetic_frame(&config, 3);
    let sdp = PresetParams {
        sdp: true,
        ..*config.preset.params()
    };
    let coder = IntraFrameCoder::with_preset(&config, &sdp).unwrap();
    let trees = search_frame(&coder, &key, &source, config.bit_depth);
    let mut roots = Vec::new();
    for tree in &trees {
        collect_sdp(tree, &mut roots);
    }
    assert_eq!(roots.len(), 4, "one SDP node per 64x64 quadrant");
    let mut merged = 0;
    for (luma, chroma) in roots {
        assert!(merges(chroma, luma), "{chroma:?} splits deeper than {luma:?}");
        merged += usize::from(shape(chroma) != shape(luma));
    }
    assert!(merged > 0, "no chroma tree merged a luma node");
    let coder = IntraFrameCoder::with_preset(&config, config.preset.params()).unwrap();
    assert!(!coder.sequence.partition.unwrap().enable_sdp);
    for tree in search_frame(&coder, &key, &source, config.bit_depth) {
        let mut roots = Vec::new();
        collect_sdp(&tree, &mut roots);
        assert!(roots.is_empty());
    }
}

fn collect_sdp<'a>(
    tree: &'a PartitionTree,
    out: &mut Vec<(&'a PartitionTree, &'a PartitionTree)>,
) {
    match tree {
        PartitionTree::Sdp { luma, chroma } => out.push((luma, chroma)),
        PartitionTree::Split { children, .. } => {
            for child in children {
                collect_sdp(child, out);
            }
        }
        PartitionTree::Leaf(_) => {}
    }
}

/// The partitions of `tree` in syntax order, `NONE` for each leaf.
fn shape(tree: &PartitionTree) -> Vec<Partition> {
    match tree {
        PartitionTree::Split { kind, children } => {
            let mut out = vec![*kind];
            out.extend(children.iter().flat_map(shape));
            out
        }
        _ => vec![Partition::None],
    }
}

/// AVM decodes SDP key frames and the inter frames between them to the
/// encoder reconstructions in every chroma format.
#[test]
#[ignore = "needs an AVM decoder build at SPLOT_AVM_DECODER"]
fn avm_decodes_sdp_streams_like_splot() {
    let Some(avmdec) = avm_decoder() else {
        return;
    };
    for case in SDP_CASES {
        let (units, expected) = sdp_stream(case, 2);
        let avm = avm_decode(&avmdec, &units, (case.0, case.1), case.2, "sdp");
        assert!(avm == expected, "{case:?}: AVM output differs");
    }
}
