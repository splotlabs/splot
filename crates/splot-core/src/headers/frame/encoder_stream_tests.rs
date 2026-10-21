// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::unwrap_used)]

use super::*;
use crate::annexb::parse_annex_b_obus_partial;
use crate::headers::frame::{FrameSize, FrameType, TxMode};
use crate::write::write_frame_header_core;

const SIZES: [(u32, u32); 9] = [
    (200, 136),
    (1, 1),
    (2, 2),
    (17, 9),
    (64, 64),
    (65, 65),
    (352, 288),
    (1920, 1080),
    (4096, 2160),
];

fn params(
    width: u32,
    height: u32,
    bit_depth: BitDepthIdc,
    sb: SuperblockSize,
) -> EncoderStreamParams {
    EncoderStreamParams {
        width,
        height,
        bit_depth,
        chroma: ChromaFormatIdc::Yuv420,
        sb_size: sb,
        tools: EncoderToolFlags::default(),
    }
}

fn reparse_sequence(seq: &SequenceHeader) -> SequenceHeader {
    let mut w = BitWriter::new();
    write_obu_payload(
        &mut w,
        &ParsedObu::SequenceHeader(Box::new(seq.clone())),
        ObuType::SequenceHeader.is_extensible_obu(),
        &[],
    )
    .unwrap();
    let bytes = w.into_bytes();
    parse_sequence_header(&mut BitReader::new(&bytes, ByteOffset::new(0))).unwrap()
}

fn reparse_core(core: &FrameHeaderCore, view: &CoreSeqView) -> FrameHeaderCore {
    let mut w = BitWriter::new();
    write_frame_header_core(&mut w, core, view, None, true).unwrap();
    let bytes = w.into_bytes();
    let mut reader = BitReader::new(&bytes, ByteOffset::new(0));
    let prefix =
        parse_frame_header_prefix(&mut reader, ObuType::ClosedLoopKey, Some(true)).unwrap();
    let mut reparsed = init_core_from_prefix(&prefix, ObuType::ClosedLoopKey, true);
    parse_core_body(
        &mut reader,
        &mut reparsed,
        view,
        None,
        &FrameReferenceStateView::unknown(),
    )
    .unwrap();
    reparsed.consumed_bits = reader.consumed_bits();
    reparsed
}

#[test]
fn headers_round_trip_for_every_size_bit_depth_and_superblock() {
    for (width, height) in SIZES {
        for bit_depth in [BitDepthIdc::Eight, BitDepthIdc::Ten] {
            for sb in [
                SuperblockSize::Block64x64,
                SuperblockSize::Block128x128,
                SuperblockSize::Block256x256,
            ] {
                let seq =
                    build_encoder_sequence_header(&params(width, height, bit_depth, sb)).unwrap();
                assert_eq!(seq.general.max_frame_width.get(), width);
                assert_eq!(seq.general.max_frame_height.get(), height);
                assert_eq!(seq.general.bit_depth_idc, bit_depth);
                assert!(!seq.general.single_picture_header_flag);
                assert!(!seq.general.still_picture);
                assert_eq!(seq.partition.unwrap().seq_sb_size(), sb);
                assert_eq!(reparse_sequence(&seq), seq);

                let view = CoreSeqView::from_sequence(&seq).unwrap();
                let max_q = if bit_depth == BitDepthIdc::Eight {
                    255
                } else {
                    511
                };
                for (base_q_idx, order_hint) in [(1, 0), (80, 7), (max_q, 255)] {
                    let core = build_key_frame_header(&view, base_q_idx, order_hint).unwrap();
                    assert_eq!(core.frame_type, Some(FrameType::Key));
                    assert_eq!(core.frame_size, Some(FrameSize::new(width, height)));
                    assert_eq!(core.immediate_output_frame, Some(true));
                    assert_eq!(core.order_hint_lsb, Some(order_hint));
                    assert_eq!(core.quantization_params.unwrap().base_q_idx, base_q_idx);
                    let tiles = core.tile_info.as_ref().unwrap();
                    assert_eq!((tiles.tile_cols, tiles.tile_rows), (1, 1));
                    let tail = core.intra_tail.unwrap();
                    assert_eq!(tail.tx_mode, TxMode::Largest);
                    assert_eq!(u32::from(tail.reduced_tx_set), KEY_FRAME_REDUCED_TX_SET);
                    assert_eq!(reparse_core(&core, &view), core);
                }
            }
        }
    }
}

#[test]
fn every_tool_flag_reaches_the_parsed_header() {
    let tools = EncoderToolFlags {
        cdef: true,
        loop_restoration: true,
        ccso: true,
        gdf: true,
        sdp: true,
        extended_sdp: true,
        ext_partitions: true,
        uneven_4way_partitions: true,
        cfl: true,
        mhccp: true,
        screen_content: true,
        ibp: true,
        mrls: true,
        fsc: true,
        dip: true,
        idtx_intra: true,
        intra_edge_filter: true,
        intra_ist: true,
        cctx: true,
        tcq: true,
        parity_hiding: false,
    };
    let mut p = params(640, 360, BitDepthIdc::Ten, SuperblockSize::Block64x64);
    p.tools = tools;
    let seq = build_encoder_sequence_header(&p).unwrap();
    let part = seq.partition.unwrap();
    assert!(part.enable_sdp && part.enable_extended_sdp);
    assert!(part.enable_ext_partitions && part.enable_uneven_4way_partitions);
    let intra = seq.intra.unwrap();
    assert!(intra.enable_dip && intra.enable_mrls && intra.enable_cfl_intra);
    assert!(intra.enable_mhccp && intra.enable_ibp && intra.enable_intra_edge_filter);
    let tq = seq.transform_quant_entropy.unwrap();
    assert!(
        tq.enable_fsc && tq.enable_idtx_intra && tq.enable_intra_ist,
        "{tq:?}"
    );
    assert!(tq.enable_cctx && tq.enable_tcq && !tq.enable_parity_hiding);
    let filter = seq.filter.unwrap();
    assert!(filter.enable_cdef && filter.enable_gdf && filter.enable_restoration);
    assert!(filter.enable_ccso);
    assert_eq!(
        seq.screen_content.unwrap().seq_force_screen_content_tools,
        2
    );
    assert_eq!(reparse_sequence(&seq), seq);

    let view = CoreSeqView::from_sequence(&seq).unwrap();
    let core = build_key_frame_header(&view, 100, 1).unwrap();
    assert_eq!(core.allow_screen_content_tools, Some(false));
    assert_eq!(reparse_core(&core, &view), core);
}

#[test]
fn default_tools_are_all_off() {
    let seq = build_encoder_sequence_header(&params(
        64,
        64,
        BitDepthIdc::Eight,
        SuperblockSize::Block64x64,
    ))
    .unwrap();
    let part = seq.partition.unwrap();
    assert!(!part.enable_sdp && !part.enable_ext_partitions);
    let intra = seq.intra.unwrap();
    assert!(!intra.enable_cfl_intra && !intra.enable_mrls && !intra.enable_dip);
    let tq = seq.transform_quant_entropy.unwrap();
    assert!(!tq.enable_tcq && !tq.enable_parity_hiding && !tq.enable_fsc);
    let filter = seq.filter.unwrap();
    assert!(!filter.enable_cdef && !filter.enable_restoration && !filter.enable_ccso);
    assert_eq!(seq.film_grain_params_present, Some(false));
}

#[test]
fn rejects_out_of_range_inputs() {
    assert!(matches!(
        build_encoder_sequence_header(&params(
            0,
            8,
            BitDepthIdc::Eight,
            SuperblockSize::Block64x64
        )),
        Err(EncoderHeaderError::InvalidDimension { value: 0 })
    ));
    let seq = build_encoder_sequence_header(&params(
        8,
        8,
        BitDepthIdc::Eight,
        SuperblockSize::Block64x64,
    ))
    .unwrap();
    let view = CoreSeqView::from_sequence(&seq).unwrap();
    assert!(matches!(
        build_key_frame_header(&view, 0, 0),
        Err(EncoderHeaderError::InvalidBaseQIdx { .. })
    ));
    assert!(matches!(
        build_key_frame_header(&view, 256, 0),
        Err(EncoderHeaderError::InvalidBaseQIdx { .. })
    ));
}

#[test]
fn temporal_unit_carries_delimiter_sequence_and_frame() {
    let seq = build_encoder_sequence_header(&params(
        17,
        9,
        BitDepthIdc::Eight,
        SuperblockSize::Block64x64,
    ))
    .unwrap();
    let view = CoreSeqView::from_sequence(&seq).unwrap();
    let core = build_key_frame_header(&view, 60, 0).unwrap();
    let tile = [0x12, 0x34, 0x56];
    for (with_seq, expected) in [
        (
            true,
            vec![
                ObuType::TemporalDelimiter,
                ObuType::SequenceHeader,
                ObuType::ClosedLoopKey,
            ],
        ),
        (
            false,
            vec![ObuType::TemporalDelimiter, ObuType::ClosedLoopKey],
        ),
    ] {
        let unit = encode_key_frame_temporal_unit(&seq, &core, with_seq, &tile).unwrap();
        let parsed = parse_annex_b_obus_partial(&unit);
        assert!(parsed.error.is_none());
        let types: Vec<_> = parsed.obus.iter().map(|o| o.header.obu_type).collect();
        assert_eq!(types, expected);
        let frame = parsed.obus.last().unwrap();
        assert!(frame.payload.ends_with(&tile));
    }
    assert!(encode_key_frame_temporal_unit(&seq, &core, true, &[]).is_err());
}
