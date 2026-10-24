// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::unwrap_used)]

use super::*;
use crate::annexb::parse_annex_b_obus_partial;
use crate::headers::frame::{
    CdefStrengthSet, CdefStrengthSets, FrameRestorationType, FrameSize, FrameType, TxMode,
};
use crate::headers::sequence::DrlReorder;
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
                for (index, (base_q_idx, order_hint)) in
                    [(1, 0), (80, 7), (max_q, 255)].into_iter().enumerate()
                {
                    let tx = KeyFrameTxParams {
                        tx_mode: [TxMode::Largest, TxMode::Select][index % 2],
                        reduced_tx_set: (index + usize::from(sb == SuperblockSize::Block64x64))
                            as u8,
                        allow_tcq: false,
                    };
                    let core = build_key_frame_header(
                        &view,
                        base_q_idx,
                        order_hint,
                        &FrameFilterParams::default(),
                        tx,
                        UniformTiles::default(),
                    )
                    .unwrap();
                    assert_eq!(core.frame_type, Some(FrameType::Key));
                    assert_eq!(core.frame_size, Some(FrameSize::new(width, height)));
                    assert_eq!(core.immediate_output_frame, Some(true));
                    assert_eq!(core.order_hint_lsb, Some(order_hint));
                    assert_eq!(core.quantization_params.unwrap().base_q_idx, base_q_idx);
                    let tiles = core.tile_info.as_ref().unwrap();
                    assert_eq!((tiles.tile_cols, tiles.tile_rows), (1, 1));
                    let tail = core.intra_tail.unwrap();
                    assert_eq!(tail.tx_mode, tx.tx_mode);
                    assert_eq!(tail.reduced_tx_set, tx.reduced_tx_set);
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
        avg_cdf: Some(false),
        ref_frame_mvs: true,
        refmvbank: true,
        drl_reorder: Some(true),
        max_drl_bits_minus_1: 3,
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
    assert!(tq.enable_cctx && tq.enable_tcq && tq.choose_tcq_per_frame);
    assert!(tq.enable_avg_cdf && tq.avg_cdf_type == 0);
    let inter = seq.inter.unwrap();
    assert!(inter.enable_ref_frame_mvs && !inter.reduced_ref_frame_mvs_mode);
    assert!(inter.enable_refmvbank && inter.drl_reorder == DrlReorder::Constraint);
    assert_eq!(inter.seq_max_drl_bits_minus_1, 3);
    let filter = seq.filter.unwrap();
    assert!(filter.enable_cdef && filter.enable_gdf && filter.enable_restoration);
    assert!(filter.enable_ccso);
    assert_eq!(
        seq.screen_content.unwrap().seq_force_screen_content_tools,
        2
    );
    assert_eq!(reparse_sequence(&seq), seq);

    let view = CoreSeqView::from_sequence(&seq).unwrap();
    let core = build_key_frame_header(
        &view,
        100,
        1,
        &FrameFilterParams::default(),
        KeyFrameTxParams::default(),
        UniformTiles::default(),
    )
    .unwrap();
    assert_eq!(core.allow_screen_content_tools, Some(false));
    assert_eq!(reparse_core(&core, &view), core);
}

/// An edge-classified CCSO plane with asymmetric offsets.
fn ccso_plane() -> CcsoPlaneParams {
    CcsoPlaneParams {
        ccso_planes: true,
        ccso_bo_only: Some(false),
        ccso_scale_idx: Some(1),
        ccso_quant_idx: Some(2),
        ccso_ext_filter: Some(4),
        ccso_edge_clf: Some(true),
        ccso_max_band_log2: Some(1),
        ccso_offset_idx: crate::headers::frame::CcsoOffsets::from_iter_checked([
            3, 0, 7, 1, 2, 6, 5, 4,
        ])
        .unwrap(),
        ..CcsoPlaneParams::default()
    }
}

#[test]
fn filter_params_reach_the_parsed_header() {
    let set = |y_pri, y_sec, uv_pri, uv_sec| CdefStrengthSet {
        y_pri_strength: y_pri,
        y_sec_strength: y_sec,
        uv_pri_strength: uv_pri,
        uv_sec_strength: uv_sec,
    };
    for sb in [SuperblockSize::Block64x64, SuperblockSize::Block128x128] {
        let mut p = params(352, 288, BitDepthIdc::Ten, sb);
        p.tools.cdef = true;
        p.tools.loop_restoration = true;
        p.tools.ccso = true;
        let seq = build_encoder_sequence_header(&p).unwrap();
        let view = CoreSeqView::from_sequence(&seq).unwrap();
        let lr_sizes: &[u32] = if sb == SuperblockSize::Block64x64 {
            &[64, 128, 256, 512]
        } else {
            &[128, 256, 512]
        };
        for &lr_size in lr_sizes {
            let filters = FrameFilterParams {
                deblocking: [true, true, true, false],
                deblocking_delta_q: [-2, 1, 0, 0],
                cdef_damping: 5,
                cdef_strengths: CdefStrengthSets::from_slice(&[
                    set(0, 0, 0, 0),
                    set(4, 2, 1, 4),
                    set(15, 1, 0, 0),
                ])
                .unwrap(),
                lr_luma_unit_size: Some(lr_size),
                lr_luma_wiener_ns: None,
                lr_chroma_unit_size: None,
                lr_chroma_wiener_ns: [None; 2],
                ccso: [ccso_plane(), CcsoPlaneParams::default(), ccso_plane()],
            };
            let core = build_key_frame_header(
                &view,
                120,
                3,
                &filters,
                KeyFrameTxParams::default(),
                UniformTiles::default(),
            )
            .unwrap();
            let deblock = core.deblocking_filter_params.unwrap();
            assert_eq!(deblock.apply_deblocking_filter, filters.deblocking);
            assert_eq!(deblock.df_delta_q, filters.deblocking_delta_q);
            let cdef = core.cdef_params.unwrap();
            assert!(cdef.cdef_frame_enable);
            assert_eq!(cdef.cdef_damping, Some(5));
            assert_eq!(cdef.strengths, filters.cdef_strengths);
            let lr = core.lr_params.as_ref().unwrap();
            assert_eq!(
                lr.planes[0].restoration_type,
                FrameRestorationType::PcWiener
            );
            assert_eq!(lr.planes[1].restoration_type, FrameRestorationType::None);
            assert_eq!(lr.loop_restoration_size[0], lr_size);
            let ccso = core.ccso_params.as_ref().unwrap();
            assert_eq!(ccso.ccso_frame_flag, Some(true));
            assert_eq!(ccso.planes[..], filters.ccso[..]);
            assert_eq!(reparse_core(&core, &view), core);
        }
        let off = build_key_frame_header(
            &view,
            120,
            3,
            &FrameFilterParams::default(),
            KeyFrameTxParams::default(),
            UniformTiles::default(),
        )
        .unwrap();
        assert!(!off.cdef_params.unwrap().cdef_frame_enable);
        assert_eq!(off.ccso_params.unwrap().ccso_frame_flag, Some(false));
        assert!(!off.lr_params.unwrap().uses_lr);
        assert_eq!(
            off.deblocking_filter_params
                .unwrap()
                .apply_deblocking_filter,
            [false; 4]
        );
    }
}

/// A luma bank of two coded classes and one class merged into the first,
/// switchable or not, parses back to the same coefficients.
#[test]
fn wiener_ns_bank_reaches_the_parsed_header() {
    use crate::headers::frame::{WienerNsCoeffs, WienerNsFrameFilterClass};
    let class = |match_index, merged, subset, coeffs: [i16; 16]| WienerNsFrameFilterClass {
        match_index,
        merged,
        ref_bank: 0,
        subset,
        wiener_ns_uv_sym: false,
        coeffs: WienerNsCoeffs::from_iter_checked(coeffs).unwrap(),
    };
    let first = [9, -3, 4, 0, -7, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
    let second = [-24, 39, -14, 17, -16, 15, -8, 7, 1, -1, 2, 3, -5, 6, 0, -8];
    let bank = WienerNsFrameFilterBank {
        classes: crate::tile::InlineVec::from_iter_checked([
            class(0, false, Some(0), first),
            class(1, false, Some(3), second),
            class(1, true, None, first),
        ])
        .unwrap(),
    };
    let mut p = params(200, 136, BitDepthIdc::Eight, SuperblockSize::Block64x64);
    p.tools.loop_restoration = true;
    let seq = build_encoder_sequence_header(&p).unwrap();
    let view = CoreSeqView::from_sequence(&seq).unwrap();
    for switchable in [false, true] {
        let filters = FrameFilterParams {
            lr_luma_unit_size: Some(64),
            lr_luma_wiener_ns: Some(FrameWienerNs {
                switchable,
                rst_ref_pic_idx: None,
                bank,
            }),
            ..FrameFilterParams::default()
        };
        let build = |q| {
            build_key_frame_header(
                &view,
                q,
                0,
                &filters,
                KeyFrameTxParams::default(),
                UniformTiles::default(),
            )
        };
        let core = build(120).unwrap();
        let plane = &core.lr_params.as_ref().unwrap().planes[0];
        assert!(plane.frame_filters_on);
        assert_eq!(plane.num_filter_classes, Some(3));
        assert_eq!(plane.frame_filter_bank, Some(bank));
        assert_eq!(
            plane.restoration_type,
            if switchable {
                FrameRestorationType::Switchable
            } else {
                FrameRestorationType::WienerNonsep
            }
        );
        assert_eq!(reparse_core(&core, &view), core);
    }
    let mut wrong = bank;
    wrong.classes[2].coeffs[0] = 1;
    let filters = FrameFilterParams {
        lr_luma_unit_size: Some(64),
        lr_luma_wiener_ns: Some(FrameWienerNs {
            switchable: true,
            rst_ref_pic_idx: None,
            bank: wrong,
        }),
        ..FrameFilterParams::default()
    };
    assert!(
        build_key_frame_header(
            &view,
            120,
            0,
            &filters,
            KeyFrameTxParams::default(),
            UniformTiles::default()
        )
        .is_err()
    );
}

/// Chroma banks with and without `wiener_ns_uv_sym` parse back on U and V
/// with luma restoration off.
#[test]
fn chroma_wiener_ns_banks_reach_the_parsed_header() {
    use crate::headers::frame::{WienerNsCoeffs, WienerNsFrameFilterClass};
    let bank = |subset, sym, coeffs: [i16; 18]| WienerNsFrameFilterBank {
        classes: crate::tile::InlineVec::from_iter_checked([WienerNsFrameFilterClass {
            match_index: 0,
            merged: false,
            ref_bank: 0,
            subset: Some(subset),
            wiener_ns_uv_sym: sym,
            coeffs: WienerNsCoeffs::from_iter_checked(coeffs).unwrap(),
        }])
        .unwrap(),
    };
    let u = bank(
        2,
        true,
        [9, -3, 4, 0, -7, 2, 3, 3, -2, -2, 1, 1, 0, 0, 5, 5, -8, -8],
    );
    let v = bank(
        1,
        false,
        [
            -24, 39, -14, 15, 2, 1, -16, 15, 3, -4, 0, 0, 0, 0, 0, 0, 0, 0,
        ],
    );
    let mut p = params(200, 136, BitDepthIdc::Eight, SuperblockSize::Block64x64);
    p.tools.loop_restoration = true;
    let seq = build_encoder_sequence_header(&p).unwrap();
    let view = CoreSeqView::from_sequence(&seq).unwrap();
    let choice = |bank| {
        Some(FrameWienerNs {
            switchable: false,
            rst_ref_pic_idx: None,
            bank,
        })
    };
    let filters = FrameFilterParams {
        lr_chroma_unit_size: Some(32),
        lr_chroma_wiener_ns: [choice(u), choice(v)],
        ..FrameFilterParams::default()
    };
    let core = build_key_frame_header(
        &view,
        120,
        0,
        &filters,
        KeyFrameTxParams::default(),
        UniformTiles::default(),
    )
    .unwrap();
    let lr = core.lr_params.as_ref().unwrap();
    assert!(lr.uses_lr);
    assert_eq!(lr.planes[0].restoration_type, FrameRestorationType::None);
    assert_eq!(lr.loop_restoration_size[1..], [32, 32]);
    for (plane, bank) in [(1, u), (2, v)] {
        assert_eq!(
            lr.planes[plane].restoration_type,
            FrameRestorationType::WienerNonsep
        );
        assert_eq!(lr.planes[plane].frame_filter_bank, Some(bank));
    }
    assert_eq!(reparse_core(&core, &view), core);
}

#[test]
fn rejects_filters_the_sequence_cannot_carry() {
    let seq = build_encoder_sequence_header(&params(
        64,
        64,
        BitDepthIdc::Eight,
        SuperblockSize::Block64x64,
    ))
    .unwrap();
    let view = CoreSeqView::from_sequence(&seq).unwrap();
    let cdef = FrameFilterParams {
        cdef_damping: 3,
        cdef_strengths: CdefStrengthSets::from_slice(&[CdefStrengthSet::default()]).unwrap(),
        ..FrameFilterParams::default()
    };
    let lr = FrameFilterParams {
        lr_luma_unit_size: Some(64),
        ..FrameFilterParams::default()
    };
    let chroma_only = FrameFilterParams {
        deblocking: [false, false, true, true],
        ..FrameFilterParams::default()
    };
    for filters in [cdef, lr, chroma_only] {
        assert!(matches!(
            build_key_frame_header(
                &view,
                60,
                0,
                &filters,
                KeyFrameTxParams::default(),
                UniformTiles::default()
            ),
            Err(EncoderHeaderError::Write(_))
        ));
    }
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
        build_key_frame_header(
            &view,
            0,
            0,
            &FrameFilterParams::default(),
            KeyFrameTxParams::default(),
            UniformTiles::default()
        ),
        Err(EncoderHeaderError::InvalidBaseQIdx { .. })
    ));
    assert!(matches!(
        build_key_frame_header(
            &view,
            256,
            0,
            &FrameFilterParams::default(),
            KeyFrameTxParams::default(),
            UniformTiles::default()
        ),
        Err(EncoderHeaderError::InvalidBaseQIdx { .. })
    ));
    for tx in [
        KeyFrameTxParams {
            tx_mode: TxMode::Only4x4,
            reduced_tx_set: 0,
            allow_tcq: false,
        },
        KeyFrameTxParams {
            tx_mode: TxMode::Select,
            reduced_tx_set: 4,
            allow_tcq: false,
        },
    ] {
        assert!(matches!(
            build_key_frame_header(
                &view,
                60,
                0,
                &FrameFilterParams::default(),
                tx,
                UniformTiles::default()
            ),
            Err(EncoderHeaderError::InvalidTxParams { .. })
        ));
    }
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
    let core = build_key_frame_header(
        &view,
        60,
        0,
        &FrameFilterParams::default(),
        KeyFrameTxParams::default(),
        UniformTiles::default(),
    )
    .unwrap();
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
        let unit = encode_key_frame_temporal_unit(&seq, &core, with_seq, &[&tile]).unwrap();
        let parsed = parse_annex_b_obus_partial(&unit);
        assert!(parsed.error.is_none());
        let types: Vec<_> = parsed.obus.iter().map(|o| o.header.obu_type).collect();
        assert_eq!(types, expected);
        let frame = parsed.obus.last().unwrap();
        assert!(frame.payload.ends_with(&tile));
    }
    assert!(encode_key_frame_temporal_unit(&seq, &core, true, &[&[]]).is_err());
}

/// A 200x136 frame with 64x64 superblocks has 4 superblock columns and 3 rows,
/// so the last layout is capped at 4x3 tiles.
#[test]
fn uniform_tiles_round_trip_through_header_and_tile_group() {
    let seq = build_encoder_sequence_header(&params(
        200,
        136,
        BitDepthIdc::Eight,
        SuperblockSize::Block64x64,
    ))
    .unwrap();
    let view = CoreSeqView::from_sequence(&seq).unwrap();
    for (cols_log2, rows_log2, cols, rows) in [
        (0, 0, 1, 1),
        (1, 0, 2, 1),
        (0, 1, 1, 2),
        (1, 1, 2, 2),
        (2, 1, 4, 2),
        (6, 6, 4, 3),
    ] {
        let tiles = UniformTiles {
            cols_log2,
            rows_log2,
            size_bytes: 2,
        };
        let core = build_key_frame_header(
            &view,
            60,
            0,
            &FrameFilterParams::default(),
            KeyFrameTxParams::default(),
            tiles,
        )
        .unwrap();
        let info = core.tile_info.as_ref().unwrap();
        assert_eq!((info.tile_cols, info.tile_rows), (cols, rows));
        assert_eq!(info.context_update_tile_id, 0);
        let multi = cols * rows > 1;
        assert_eq!(info.tile_size_bytes, multi.then_some(2));
        assert_eq!(reparse_core(&core, &view), core);

        let data: Vec<Vec<u8>> = (0..cols * rows)
            .map(|tile| vec![tile as u8 + 1; 3 + 70 * tile as usize])
            .collect();
        let slices: Vec<&[u8]> = data.iter().map(Vec::as_slice).collect();
        let unit = encode_key_frame_temporal_unit(&seq, &core, false, &slices).unwrap();
        let parsed = parse_annex_b_obus_partial(&unit);
        assert!(parsed.error.is_none());
        let payload = parsed.obus.last().unwrap().payload;
        let mut reader = BitReader::new(payload, ByteOffset::new(0));
        assert_eq!(reader.read_bit().unwrap(), 1, "is_first_tile_group");
        let prefix =
            parse_frame_header_prefix(&mut reader, ObuType::ClosedLoopKey, Some(true)).unwrap();
        let mut header = init_core_from_prefix(&prefix, ObuType::ClosedLoopKey, true);
        parse_core_body(
            &mut reader,
            &mut header,
            &view,
            None,
            &FrameReferenceStateView::unknown(),
        )
        .unwrap();
        assert_eq!(header.tile_info, core.tile_info);
        let layout = crate::headers::tile_group::TileGroupLayout::new(
            cols,
            rows,
            info.tile_cols_log2,
            info.tile_rows_log2,
        );
        let structure = crate::headers::tile_group::parse_tile_group_structure(
            &mut reader,
            layout,
            payload.len() as u64,
        )
        .unwrap();
        assert_eq!((structure.tg_start, structure.tg_end), (0, cols * rows - 1));
        let start = structure.header_bytes.unwrap() as usize;
        let framing = crate::headers::tile_group::parse_tile_group_framing(
            &payload[start..],
            0,
            cols * rows - 1,
            info.tile_size_bytes.unwrap_or(1),
            false,
        );
        assert!(framing.defect.is_none());
        let sizes: Vec<u64> = framing.tiles.iter().map(|tile| tile.tile_size).collect();
        let expected: Vec<u64> = data.iter().map(|tile| tile.len() as u64).collect();
        assert_eq!(sizes, expected);
        assert_eq!(
            framing,
            crate::headers::tile_group::TileGroupFraming::tiles(
                &expected,
                2.min(1 + u32::from(multi))
            )
        );
    }
}
