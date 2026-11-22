// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Extended SDP (§ 5.20.3.1 `region_type`): `INTRA_REGION` nodes of inter
//! frames in coded streams.

use super::*;
use crate::preset::PresetContext;
use splot_decode::encoder_syntax::{EncoderSyntaxError, IntraTileSyntax};

/// The qindex of every frame, where each format chooses regions.
const QP: u16 = 140;

/// The chroma formats with chroma planes, which extended SDP needs.
const FORMATS: [PixelFormat; 3] = [
    PixelFormat::Yuv420,
    PixelFormat::Yuv422,
    PixelFormat::Yuv444,
];

/// Frame `index` of a pan whose inter frames show a new object: a square of
/// noise that changes from frame to frame, which no reference predicts.
fn object_frame(config: &Config, index: u32) -> SourceFrame {
    let frame = panning_frame(config, index);
    if index == 0 {
        return frame;
    }
    let mut planes = frame.planes().clone();
    let max = config.bit_depth.max_sample();
    for (plane, samples) in planes.iter_mut().enumerate() {
        let (sx, sy) = match plane {
            0 => (0, 0),
            _ => (config.chroma.subsampling_x(), config.chroma.subsampling_y()),
        };
        let width = (config.width as usize + (1 << sx) - 1) >> sx;
        for y in (24 >> sy)..(56 >> sy) {
            for x in (32 >> sx)..(64 >> sx) {
                let hash = (x * 0x9E37 + y * 0x85EB + index as usize * 0x2C1B) as u32;
                samples[y * width + x] = (hash.wrapping_mul(0x2545_F491) >> 20) as u16 % (max + 1);
            }
        }
    }
    frame.with_planes(planes)
}

/// The `INTRA_REGION` nodes of `tree`.
fn regions(tree: &PartitionTree) -> usize {
    match tree {
        PartitionTree::Region { luma, .. } => 1 + luma.iter().map(regions).sum::<usize>(),
        PartitionTree::Split { children, .. } => children.iter().map(regions).sum(),
        PartitionTree::Sdp { luma, chroma } => regions(luma) + regions(chroma),
        PartitionTree::Leaf(_) => 0,
    }
}

/// A key frame and three inter frames of [`object_frame`] at `extended_sdp`
/// `level` in `chroma` on `threads` threads: the units, the encoder recon,
/// and the `INTRA_REGION` nodes the search of each inter frame chooses.
fn region_stream(level: u8, chroma: PixelFormat, threads: usize) -> (Vec<u8>, Vec<u8>, usize) {
    let mut config = config(96, 80, BitDepth::Eight, 6, QP);
    config.chroma = chroma;
    config.threads = splot_parallel::ThreadCount::from(threads);
    let preset = PresetParams {
        sdp: true,
        extended_sdp: level,
        ..*config.preset.params()
    };
    let mut coder = IntraFrameCoder::with_preset(&config, &preset).unwrap();
    assert_eq!(
        coder.sequence.partition.unwrap().enable_extended_sdp,
        level > 0
    );
    let (mut units, mut recon, mut found) = (Vec::new(), Vec::new(), 0);
    for index in 0..4 {
        let plan = crate::gop::plan(u64::from(index), config.keyint);
        let source = object_frame(&config, index);
        if plan.frame_type == FrameType::Inter {
            let trees = search_frame_at(&coder, &plan, &source, (config.bit_depth, QP)).0;
            found += trees.iter().map(regions).sum::<usize>();
        }
        let coded = coder.encode_frame(&source, &plan, QP).unwrap();
        recon.extend(recon_bytes(&coded, config.bit_depth));
        units.push(coded.temporal_unit);
    }
    (units.concat(), recon, found)
}

/// At `extended_sdp` 2 the inter frames code `INTRA_REGION` nodes over the
/// new object in 4:2:0, 4:2:2 and 4:4:4, and at 1 in some format (4:2:0
/// forces inter on the small shared leaves, so no shared subtree there is
/// intra only); the streams decode to the encoder recon and code the same
/// bytes on 1, 2 and 8 threads. At 0 the search finds no region.
#[test]
fn extended_sdp_regions_decode_on_any_thread_count() {
    let cases = [0, 1, 2]
        .into_iter()
        .flat_map(|level| FORMATS.map(|chroma| (level, chroma)));
    let found = each_case(cases, |(level, chroma)| {
        let case = format!("level {level} {chroma:?}");
        let (one, expected, found) = region_stream(level, chroma, 1);
        assert!(
            level == 1 || (found > 0) == (level == 2),
            "{case}: {found} regions"
        );
        assert_same_samples(&decode(&one), &expected, &case);
        each_case([2, 8], |threads| {
            assert!(
                region_stream(level, chroma, threads).0 == one,
                "{case} {threads}"
            );
        });
        (level, found)
    });
    let level_1: usize = found
        .iter()
        .filter(|(level, _)| *level == 1)
        .map(|(_, n)| n)
        .sum();
    assert!(level_1 > 0, "level 1 found no region");
}

/// The facade codes `region_type` only where § 5.20.3.1 reads one: not at
/// the superblock root, not after `NONE` or `SPLIT` (`PARTITION_SPLIT` is 9,
/// above `PARTITION_HORZ_4A`), not inside a region. An
/// `INTRA_REGION` node has `LUMA_PART` children and one `CHROMA_PART` block
/// of its own size.
#[test]
fn the_facade_codes_region_type_where_the_decoder_reads_it() {
    let config = config(96, 80, BitDepth::Eight, 6, QP);
    let preset = PresetParams {
        sdp: true,
        extended_sdp: 2,
        ..*config.preset.params()
    };
    let mut coder = IntraFrameCoder::with_preset(&config, &preset).unwrap();
    let key = crate::gop::plan(0, config.keyint);
    coder
        .encode_frame(&object_frame(&config, 0), &key, QP)
        .unwrap();
    let plan = crate::gop::plan(1, config.keyint);
    let tools =
        crate::gop::frame_tools((&coder.preset, PresetContext::default()), &coder.tiles, 90);
    let core = plan
        .header(
            &coder.view,
            &coder.references,
            90,
            &FrameFilterParams::default(),
            tools,
        )
        .unwrap();
    let mut syntax = IntraTileSyntax::new(&coder.sequence, &core, &coder.references, 0).unwrap();
    let mut sink = crate::rd::coeff_rate::RateSink::adapting();
    let root = syntax.superblock_root(0, 0);
    syntax.begin_superblock(root);
    assert!(!syntax.codes_region_type(root, Partition::Split).unwrap());
    assert!(matches!(
        syntax.write_region_type(root, Partition::Split, true, &mut sink),
        Err(EncoderSyntaxError::Unsupported(_))
    ));
    syntax
        .write_partition(root, Partition::Split, &mut sink)
        .unwrap();
    let node = syntax.children(root, Partition::Split).unwrap()[0];
    assert!(!syntax.codes_region_type(node, Partition::None).unwrap());
    assert!(!syntax.codes_region_type(node, Partition::Split).unwrap());
    assert!(syntax.codes_region_type(node, Partition::Horz).unwrap());
    syntax
        .write_partition(node, Partition::Horz, &mut sink)
        .unwrap();
    syntax
        .write_region_type(node, Partition::Horz, true, &mut sink)
        .unwrap();
    assert!(sink.bits > crate::rd::Bits(0));
    let (luma, chroma) = syntax.region_children(node, Partition::Horz).unwrap();
    assert_eq!(luma.len(), 2);
    assert!(chroma.is_chroma_part() && luma.iter().all(|child| !child.is_chroma_part()));
    assert_eq!(chroma.dimensions().unwrap(), node.dimensions().unwrap());
    assert!(!syntax.codes_region_type(luma[0], Partition::Split).unwrap());
}

/// AVM decodes extended SDP streams like splot.
#[test]
#[ignore = "needs an AVM decoder build at SPLOT_AVM_DECODER"]
fn avm_decodes_extended_sdp_streams_like_splot() {
    let Some(avmdec) = avm_decoder() else {
        return;
    };
    for (level, chroma) in [1, 2]
        .into_iter()
        .flat_map(|level| FORMATS.map(|chroma| (level, chroma)))
    {
        let (units, expected, _) = region_stream(level, chroma, 1);
        let avm = avm_decode(&avmdec, &[units], (96, 80), BitDepth::Eight, "ext-sdp");
        assert!(
            avm == expected,
            "level {level} {chroma:?}: AVM output differs"
        );
    }
}
