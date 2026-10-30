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

/// The frame filter tools some coded frame turned on: deblocking, CDEF with
/// more than one strength, CCSO, GDF, luma and chroma LR, and `delta_q`.
type Tools = [bool; 7];

/// The qp at which the cases code every frame filter tool between them, and
/// luma and chroma restoration with 64x64 superblocks.
const QP: u16 = 160;

/// Codes key frame, inter frame, key frame of panning content with SDP and
/// asymmetric superblock qindex offsets on `threads` threads; returns the
/// units, the encoder recon and the frame filter tools coded.
fn sdp_stream(case: SdpCase, threads: usize) -> (Vec<Vec<u8>>, Vec<u8>, Tools) {
    let (width, height, bit_depth, preset, chroma, (cols_log2, rows_log2)) = case;
    let mut config = config(width, height, bit_depth, preset, QP);
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
    let sb = if preset.max_block_log2 >= 7 { 128 } else { 64 };
    let superblocks = (width.div_ceil(sb) * height.div_ceil(sb)) as usize;
    let (mut units, mut recon, mut tools) = (Vec::new(), Vec::new(), [false; 7]);
    for index in 0..3 {
        let plan = crate::gop::plan(u64::from(index), config.keyint);
        let offsets: Vec<(i16, i16)> = (0..superblocks)
            .map(|sb| [-23, 0, 17, -6, 29][(sb * 3 + index as usize) % 5])
            .map(|offset| (offset, offset * 256 / 12))
            .collect();
        let mut coded = coder
            .code_frame(&panning_frame(&config, index), &plan, QP, &offsets)
            .unwrap();
        let core = &coded.reference.as_ref().unwrap().0;
        let lr = |plane: usize| {
            core.lr_params.as_ref().is_some_and(|lr| {
                lr.planes.get(plane).is_some_and(|p| {
                    p.restoration_type != splot_core::headers::frame::FrameRestorationType::None
                })
            })
        };
        let coded_tools = [
            core.deblocking_filter_params
                .as_ref()
                .is_some_and(|df| df.apply_deblocking_filter.iter().any(|&on| on)),
            core.cdef_params
                .as_ref()
                .is_some_and(|cdef| cdef.cdef_strengths.unwrap_or(0) > 1),
            core.ccso_params
                .as_ref()
                .is_some_and(|ccso| ccso.ccso_frame_flag == Some(true)),
            core.gdf_params
                .as_ref()
                .is_some_and(|gdf| gdf.gdf_frame_enable),
            lr(0),
            lr(1) || lr(2),
            core.delta_q_params.is_some_and(|dq| dq.delta_q_present),
        ];
        for (tool, on) in tools.iter_mut().zip(coded_tools) {
            *tool |= on;
        }
        coder.commit(&mut coded).unwrap();
        recon.extend(recon_bytes(&coded, bit_depth));
        units.push(coded.temporal_unit);
    }
    (units, recon, tools)
}

/// SDP key frames and the inter frames between them decode to the encoder
/// recon in every chroma format, with tiles and with every frame filter tool
/// and `delta_q`, and 1, 2 and 8 threads write the same bytes.
#[test]
fn sdp_streams_decode_to_the_encoder_recon_on_any_thread_count() {
    let mut coded = [false; 7];
    for (index, case) in SDP_CASES.into_iter().enumerate() {
        let (one, expected, tools) = sdp_stream(case, 1);
        if index == 0 {
            assert!(
                tools[4] && tools[5],
                "64x64 superblocks code luma and chroma LR"
            );
        }
        for (all, on) in coded.iter_mut().zip(tools) {
            *all |= on;
        }
        assert_same_samples(&decode(&one.concat()), &expected, &format!("{case:?}"));
        for threads in [2, 8] {
            assert!(
                sdp_stream(case, threads).0 == one,
                "{case:?} {threads} threads"
            );
        }
    }
    assert_eq!(coded, [true; 7], "some frame filter tool is never coded");
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
        assert!(
            merges(chroma, luma),
            "{chroma:?} splits deeper than {luma:?}"
        );
        merged += usize::from(shape(chroma) != shape(luma));
    }
    assert!(merged > 0, "no chroma tree merged a luma node");
    let off = PresetParams { sdp: false, ..sdp };
    let coder = IntraFrameCoder::with_preset(&config, &off).unwrap();
    assert!(!coder.sequence.partition.unwrap().enable_sdp);
    for tree in search_frame(&coder, &key, &source, config.bit_depth) {
        let mut roots = Vec::new();
        collect_sdp(&tree, &mut roots);
        assert!(roots.is_empty());
    }
}

fn collect_sdp<'a>(tree: &'a PartitionTree, out: &mut Vec<(&'a PartitionTree, &'a PartitionTree)>) {
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
        let (units, expected, _) = sdp_stream(case, 2);
        let avm = avm_decode(&avmdec, &units, (case.0, case.1), case.2, "sdp");
        assert!(avm == expected, "{case:?}: AVM output differs");
    }
}

/// The screen-content row of preset 9 (palette, IntraBC, FSC, whole-sample
/// motion) with SDP.
static SCREEN_SDP: PresetParams = PresetParams {
    sdp: true,
    ..crate::preset::SCREEN_PRESETS[9]
};

/// Key, inter and key frames of panning repeated text at 256x128 with
/// [`SCREEN_SDP`] on `threads` threads: the units and the encoder recon.
fn screen_stream(threads: usize, chroma: PixelFormat, bit_depth: BitDepth) -> (Vec<u8>, Vec<u8>) {
    let mut config = config(256, 128, bit_depth, 9, 120);
    config.chroma = chroma;
    config.keyint = core::num::NonZeroU32::new(2).unwrap();
    config.threads = splot_parallel::ThreadCount::from(threads);
    let preset = &SCREEN_SDP;
    assert!(preset.palette && preset.intrabc && preset.fsc);
    let mut coder = IntraFrameCoder::with_preset(&config, preset).unwrap();
    let (mut units, mut recon) = (Vec::new(), Vec::new());
    for index in 0..3 {
        let plan = crate::gop::plan(u64::from(index), config.keyint);
        let source = super::intra_tools::tiled_text(&config, index);
        let coded = coder.encode_frame(&source, &plan, 120).unwrap();
        recon.extend(recon_bytes(&coded, bit_depth));
        units.push(coded.temporal_unit);
    }
    (units.concat(), recon)
}

/// The SDP luma trees of a screen-content key frame code palette and IntraBC
/// leaves, so chroma trees code over palette and IntraBC luma; the key,
/// inter and key frames decode to the encoder recon with the frame filters
/// in every chroma format, and 1, 2 and 8 threads write the same bytes.
#[test]
fn sdp_screen_streams_decode_to_the_encoder_recon_on_any_thread_count() {
    let mut config = config(256, 128, BitDepth::Eight, 9, 120);
    config.keyint = core::num::NonZeroU32::new(2).unwrap();
    let coder = IntraFrameCoder::with_preset(&config, &SCREEN_SDP).unwrap();
    let key = crate::gop::plan(0, config.keyint);
    let source = super::intra_tools::tiled_text(&config, 0);
    let trees = search_frame(&coder, &key, &source, config.bit_depth);
    let mut roots = Vec::new();
    for tree in &trees {
        collect_sdp(tree, &mut roots);
    }
    let count = |keep: fn(&LeafChoice) -> bool| -> usize {
        roots.iter().map(|(luma, _)| leaves(luma, keep)).sum()
    };
    assert!(
        count(|choice| choice.intrabc.is_some()) > 0,
        "no IntraBC luma leaf"
    );
    assert!(
        count(|choice| choice.modes.palette.is_some()) > 0,
        "no palette luma leaf"
    );
    for chroma in CHROMAS {
        let (units, recon) = screen_stream(2, chroma, BitDepth::Ten);
        assert_same_samples(&decode(&units), &recon, &format!("{chroma:?}"));
    }
    let (one, _) = screen_stream(1, PixelFormat::Yuv420, BitDepth::Eight);
    for threads in [2, 8] {
        let (units, _) = screen_stream(threads, PixelFormat::Yuv420, BitDepth::Eight);
        assert!(units == one, "{threads} threads");
    }
}

/// AVM decodes SDP screen-content streams to the encoder recon.
#[test]
#[ignore = "needs an AVM decoder build at SPLOT_AVM_DECODER"]
fn avm_decodes_sdp_screen_streams_like_splot() {
    let Some(avmdec) = avm_decoder() else {
        return;
    };
    for chroma in CHROMAS {
        let (units, recon) = screen_stream(2, chroma, BitDepth::Eight);
        let avm = avm_decode(&avmdec, &[units], (256, 128), BitDepth::Eight, "sdp-screen");
        assert!(avm == recon, "{chroma:?}: AVM output differs");
    }
}
