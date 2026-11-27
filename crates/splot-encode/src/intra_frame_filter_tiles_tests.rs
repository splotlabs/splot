// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! In-loop filters inside their tile (`disable_loopfilters_across_tiles`,
//! § 5.4.10) under [`Config::disable_loopfilters_across_tiles`].

use super::*;
use splot_core::annexb::parse_annex_b_obus_partial;
use splot_core::bitio::BitReader;
use splot_core::headers::frame::{FrameHeaderCore, FrameType as CoreFrameType};
use splot_core::headers::sequence::parse_sequence_header;
use splot_core::span::ByteOffset;
use splot_core::types::ObuType;

/// `(preset row, (width, height), (bit depth, chroma))`.
type Case = (u8, (u32, u32), (BitDepth, PixelFormat));

/// The planned 2x2-tile stream of `case` with the flag `inside` on
/// `threads` threads: its units, the encoder recon and the frame headers.
fn tiled_stream(
    case: Case,
    inside: bool,
    threads: usize,
) -> (Vec<u8>, Vec<u8>, Vec<FrameHeaderCore>) {
    let (preset, (width, height), (bit_depth, chroma)) = case;
    let mut config = config(width, height, bit_depth, preset, 200);
    config.chroma = chroma;
    config.tiles = TileLayout::Uniform {
        cols_log2: 1,
        rows_log2: 1,
    };
    config.disable_loopfilters_across_tiles = inside;
    config.threads = splot_parallel::ThreadCount::from(threads);
    let coder = IntraFrameCoder::new(&config).unwrap();
    let (units, expected, cores, _) =
        super::carry::planned_stream_of(&config, coder, panning_frame, 200);
    (units, expected, cores)
}

/// `disable_loopfilters_across_tiles` of the sequence header of `units`.
fn sequence_flag(units: &[u8]) -> bool {
    let parsed = parse_annex_b_obus_partial(units);
    let payload = parsed
        .obus
        .iter()
        .find(|obu| obu.header.obu_type == ObuType::SequenceHeader)
        .unwrap()
        .payload;
    let seq = parse_sequence_header(&mut BitReader::new(payload, ByteOffset::new(0))).unwrap();
    seq.filter.unwrap().disable_loopfilters_across_tiles
}

/// 2x2-tile streams whose filters stay inside their tile decode to the
/// encoder recon and code the same bytes on 1, 2 and 8 threads, with a key
/// and a later inter frame, deblocking, CDEF, CCSO, loop restoration and GDF
/// on (4:2:0 8-bit and 4:4:4 10-bit). The flag changes the recon: the same
/// stream with filters across the tile edges differs.
#[test]
fn filters_inside_tiles_decode_to_the_encoder_recon_on_any_thread_count() {
    let cases: [Case; 2] = [
        (6, (256, 192), (BitDepth::Eight, PixelFormat::Yuv420)),
        (6, (200, 136), (BitDepth::Ten, PixelFormat::Yuv444)),
    ];
    for (index, &case) in cases.iter().enumerate() {
        let (units, expected, cores) = tiled_stream(case, true, 1);
        assert!(sequence_flag(&units), "case {index}");
        let kinds: Vec<_> = cores.iter().map(|core| core.frame_type).collect();
        assert!(kinds.contains(&Some(CoreFrameType::Key)), "case {index}");
        assert!(kinds.contains(&Some(CoreFrameType::Inter)), "case {index}");
        assert!(
            cores.iter().all(|core| core
                .tile_info
                .as_ref()
                .is_some_and(|t| t.tile_cols * t.tile_rows == 4)),
            "case {index}"
        );
        let any = |on: fn(&FrameHeaderCore) -> bool| cores.iter().any(on);
        assert!(any(|c| c
            .deblocking_filter_params
            .is_some_and(|d| d.apply_deblocking_filter[0])));
        assert!(any(|c| c
            .cdef_params
            .as_ref()
            .is_some_and(|d| d.cdef_frame_enable)));
        assert!(any(|c| c
            .ccso_params
            .as_ref()
            .is_some_and(|p| p.ccso_frame_flag == Some(true))));
        assert!(any(|c| c.lr_params.as_ref().is_some_and(|p| p.uses_lr)));
        assert!(any(|c| c
            .gdf_params
            .as_ref()
            .is_some_and(|p| p.gdf_frame_enable)));
        assert_same_samples(&decode(&units), &expected, &format!("case {index}"));
        for threads in [2, 8] {
            let other = tiled_stream(case, true, threads).0;
            assert!(other == units, "case {index}: {threads} threads");
        }
        let (across, across_expected, _) = tiled_stream(case, false, 1);
        assert!(!sequence_flag(&across), "case {index}");
        assert!(
            across_expected != expected,
            "case {index}: the flag changes nothing"
        );
    }
}

/// AVM decodes a stream whose filters stay inside their tile like splot.
#[test]
#[ignore = "needs an AVM decoder build at SPLOT_AVM_DECODER"]
fn avm_decodes_filters_inside_tiles_like_splot() {
    let Some(avmdec) = avm_decoder() else {
        return;
    };
    let mut config = config(256, 192, BitDepth::Eight, 6, 200);
    config.tiles = TileLayout::Uniform {
        cols_log2: 1,
        rows_log2: 1,
    };
    config.disable_loopfilters_across_tiles = true;
    let (units, expected) = encode_stream(&config, 9);
    let avm = avm_decode(
        &avmdec,
        &units,
        (256, 192),
        BitDepth::Eight,
        "filters_inside_tiles",
    );
    assert!(avm == expected, "AVM output differs");
}
