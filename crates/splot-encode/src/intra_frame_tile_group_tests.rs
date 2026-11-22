// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Frames whose tiles split into several tile group OBUs (§ 5.19) under
//! [`Config::tile_groups`].

use super::*;
use splot_core::annexb::parse_annex_b_obus_partial;
use splot_core::bitio::BitReader;
use splot_core::span::ByteOffset;
use splot_core::types::ObuType;

/// A 2x2-tile mini-GOP stream at preset 7 (64x64 superblocks, filters on)
/// in `format` with `tile_groups` tile groups, coded on `threads`.
fn grouped_stream(
    (chroma, bit_depth): (PixelFormat, BitDepth),
    tile_groups: u32,
    threads: usize,
) -> (Vec<Vec<u8>>, Vec<u8>) {
    let mut config = config(72, 72, bit_depth, 7, 150);
    config.chroma = chroma;
    config.tiles = TileLayout::Uniform {
        cols_log2: 1,
        rows_log2: 1,
    };
    config.tile_groups = core::num::NonZeroU32::new(tile_groups).unwrap();
    config.threads = splot_parallel::ThreadCount::from(threads);
    encode_stream(&config, 9)
}

/// The tile group OBUs of each tiled frame of `stream`: a group with
/// `is_first_tile_group = 1` opens a frame, and each later group codes
/// `is_first_tile_group = 0` and `frame_header_present_flag = 0`.
pub(super) fn groups_per_frame(stream: &[u8]) -> Vec<usize> {
    let mut counts = Vec::new();
    for obu in parse_annex_b_obus_partial(stream).obus {
        if !matches!(
            obu.header.obu_type,
            ObuType::ClosedLoopKey | ObuType::RegularTileGroup
        ) {
            continue;
        }
        let mut reader = BitReader::new(obu.payload, ByteOffset::new(0));
        if reader.read_bit().unwrap() == 1 {
            counts.push(1);
        } else {
            assert_eq!(reader.read_bit().unwrap(), 0, "frame_header_present_flag");
            *counts.last_mut().unwrap() += 1;
        }
    }
    counts
}

/// 4:2:0 8-bit and 4:2:2 10-bit streams of 4 tiles in 3 groups (an uneven
/// split), 4 groups and 8 groups (clamped to 4) decode to the encoder recon,
/// which the groups do not change, and code the same bytes on 1, 2 and 8
/// threads; one group stays one tile group OBU per frame.
#[test]
fn tile_groups_decode_to_the_encoder_recon_on_any_thread_count() {
    for format in [
        (PixelFormat::Yuv420, BitDepth::Eight),
        (PixelFormat::Yuv422, BitDepth::Ten),
    ] {
        let (one, expected) = grouped_stream(format, 1, 1);
        let frames = groups_per_frame(&one.concat());
        assert!(frames.len() >= 5 && frames.iter().all(|&n| n == 1));
        for (groups, per_frame) in [(3, 3), (4, 4), (8, 4)] {
            let case = format!("{format:?} {groups} groups");
            let (units, recon) = grouped_stream(format, groups, 1);
            assert_eq!(
                groups_per_frame(&units.concat()),
                vec![per_frame; frames.len()]
            );
            assert_eq!(recon, expected, "{case}: the groups change the recon");
            assert_same_samples(&decode(&units.concat()), &expected, &case);
            for threads in [2, 8] {
                let other = grouped_stream(format, groups, threads).0;
                assert!(other == units, "{case}: {threads} threads");
            }
        }
    }
}

/// AVM decodes streams with several tile groups like splot.
#[test]
#[ignore = "needs an AVM decoder build at SPLOT_AVM_DECODER"]
fn avm_decodes_tile_groups_like_splot() {
    let Some(avmdec) = avm_decoder() else {
        return;
    };
    for groups in [3, 8] {
        let format = (PixelFormat::Yuv420, BitDepth::Eight);
        let (units, expected) = grouped_stream(format, groups, 1);
        let name = format!("tilegroups{groups}");
        let avm = avm_decode(&avmdec, &units, (72, 72), BitDepth::Eight, &name);
        assert!(avm == expected, "{name}: AVM output differs");
    }
}
