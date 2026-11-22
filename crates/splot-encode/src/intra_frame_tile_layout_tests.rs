// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Explicit tile sizes (`uniform_tile_spacing_flag = 0`, § 5.18.7.3) under
//! [`TileLayout::Explicit`].

use super::*;
use splot_core::headers::frame::{FrameHeaderCore, FrameType as CoreFrameType};
use splot_core::types::ObuType;

/// `(preset row, (width, height), (bit depth, chroma), (widths, heights),
/// tile groups, tile column starts in luma samples)`.
type ExplicitCase = (
    &'static PresetParams,
    (u32, u32),
    (BitDepth, PixelFormat),
    (&'static [u32], &'static [u32]),
    u32,
    &'static [u32],
);

/// The planned stream of `case` on `threads` threads: its units, the encoder
/// recon and the frame headers.
fn explicit_stream(
    case: &ExplicitCase,
    threads: usize,
) -> (Vec<u8>, Vec<u8>, Vec<FrameHeaderCore>) {
    let (preset, (width, height), (bit_depth, chroma), (widths, heights), groups, _) = *case;
    let mut config = config(width, height, bit_depth, 6, 200);
    config.chroma = chroma;
    config.tiles = TileLayout::Explicit {
        widths: widths.to_vec(),
        heights: heights.to_vec(),
    };
    config.tile_groups = core::num::NonZeroU32::new(groups).unwrap();
    config.threads = splot_parallel::ThreadCount::from(threads);
    let coder = IntraFrameCoder::with_preset(&config, preset).unwrap();
    let (units, expected, cores, _) =
        super::carry::planned_stream_of(&config, coder, panning_frame, 200);
    (units, expected, cores)
}

/// `row` of `preset` with the changes of `edit`.
fn row(preset: u8, edit: fn(&mut PresetParams)) -> &'static PresetParams {
    let mut params = *Preset::new(preset).unwrap().params();
    edit(&mut params);
    Box::leak(Box::new(params))
}

/// Streams of unequal tiles decode to the encoder recon and code the same
/// bytes on 1, 2 and 8 threads; every frame with a tile layout codes
/// `uniform_tile_spacing_flag = 0` with the same tile edges in luma samples:
/// a 1-superblock column beside a 3-superblock one with the wavefront search,
/// intra-only frames and 3 tile groups (64x64, 4:2:0 8-bit); a 2-superblock
/// column beside the rest with TIP output frames (128x128, 4:2:0 10-bit);
/// three columns and two rows in a 256x256 sequence, whose key frames code
/// 128x128 superblocks (4:4:4 8-bit).
#[test]
fn explicit_tiles_decode_to_the_encoder_recon_on_any_thread_count() {
    let cases: [ExplicitCase; 3] = [
        (
            row(7, |p| {
                p.wavefront = true;
                p.intra_only = 1;
            }),
            (200, 136),
            (BitDepth::Eight, PixelFormat::Yuv420),
            (&[1], &[2]),
            3,
            &[0, 64],
        ),
        (
            row(6, |p| {
                p.tip = 1;
                p.tip_output = 200;
                p.tip_refinemv = false;
            }),
            (280, 72),
            (BitDepth::Ten, PixelFormat::Yuv420),
            (&[2], &[]),
            1,
            &[0, 256],
        ),
        (
            row(9, |p| {
                p.max_block_log2 = 8;
                p.sb256_min_height = 64;
            }),
            (520, 300),
            (BitDepth::Eight, PixelFormat::Yuv444),
            (&[1, 1], &[1]),
            1,
            &[0, 256, 512],
        ),
    ];
    for (index, case) in cases.iter().enumerate() {
        let (units, expected, cores) = explicit_stream(case, 1);
        let mut kinds = Vec::new();
        for core in &cores {
            let Some(info) = core.tile_info.as_ref() else {
                continue;
            };
            assert!(!info.tile_params.unwrap().uniform_spacing, "case {index}");
            let starts = &info.mi_col_starts[..info.tile_cols as usize];
            let starts: Vec<u32> = starts.iter().map(|&mi| mi * 4).collect();
            assert_eq!(starts, case.5, "case {index}");
            kinds.push((core.frame_type, core.obu_type));
        }
        assert!(
            kinds
                .iter()
                .any(|&(kind, _)| kind == Some(CoreFrameType::Key))
        );
        assert!(
            kinds
                .iter()
                .any(|&(kind, _)| kind == Some(CoreFrameType::Inter))
        );
        match index {
            0 => {
                assert!(
                    kinds
                        .iter()
                        .any(|&(kind, _)| kind == Some(CoreFrameType::IntraOnly))
                );
                let groups = super::tile_group::groups_per_frame(&units);
                assert!(
                    !groups.is_empty() && groups.iter().all(|&n| n == 3),
                    "{groups:?}"
                );
            }
            1 => assert!(
                cores
                    .iter()
                    .any(|core| core.obu_type == ObuType::RegularTip)
            ),
            _ => {}
        }
        assert_same_samples(&decode(&units), &expected, &format!("case {index}"));
        for threads in [2, 8] {
            let other = explicit_stream(case, threads).0;
            assert!(other == units, "case {index}: {threads} threads");
        }
    }
}

/// The encoder refuses explicit sizes that end beyond the frame edge or give
/// more than 64 columns, and sizes that end on the edge code no extra column.
#[test]
fn explicit_tiles_outside_the_frame_are_refused() {
    let preset = Preset::new(7).unwrap().params();
    let coder = |widths: Vec<u32>| {
        let mut config = config(200, 136, BitDepth::Eight, 7, 200);
        config.tiles = TileLayout::Explicit {
            widths,
            heights: Vec::new(),
        };
        IntraFrameCoder::with_preset(&config, preset).map(|coder| coder.tiles)
    };
    for (widths, reason) in [
        (vec![4, 1], "tiles beyond the frame edge"),
        (vec![5], "tiles beyond the frame edge"),
    ] {
        let refused = coder(widths);
        assert!(matches!(refused, Err(Error::InvalidTiles { reason: r, .. }) if r == reason));
    }
    let edge = coder(vec![1, 3]).unwrap().explicit.unwrap();
    assert_eq!((&edge[0][..], &edge[1][..]), (&[0, 1][..], &[0][..]));
    let mut config = config(64 * 66, 64, BitDepth::Eight, 7, 200);
    config.tiles = TileLayout::Explicit {
        widths: vec![1; 65],
        heights: Vec::new(),
    };
    let refused = IntraFrameCoder::with_preset(&config, preset).map(|_| ());
    assert!(matches!(
        refused,
        Err(Error::InvalidTiles {
            reason: "more than 64 tile columns or rows",
            ..
        })
    ));
}

/// AVM decodes a stream of explicit tiles like splot.
#[test]
#[ignore = "needs an AVM decoder build at SPLOT_AVM_DECODER"]
fn avm_decodes_explicit_tiles_like_splot() {
    let Some(avmdec) = avm_decoder() else {
        return;
    };
    let mut config = config(200, 136, BitDepth::Eight, 7, 150);
    config.tiles = TileLayout::Explicit {
        widths: vec![1],
        heights: vec![2],
    };
    let (units, expected) = encode_stream(&config, 9);
    let avm = avm_decode(
        &avmdec,
        &units,
        (200, 136),
        BitDepth::Eight,
        "explicit_tiles",
    );
    assert!(avm == expected, "AVM output differs");
}
