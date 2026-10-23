// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::unwrap_used)]

use super::*;
use crate::annexb::parse_annex_b_obus_partial;
use crate::headers::frame::{
    CdefStrengthSet, CdefStrengthSets, EncoderStreamParams, EncoderToolFlags,
    FrameHeaderParseInput, FrameHeaderParseMode, FrameType, TxMode, build_encoder_sequence_header,
    parse_frame_header_core,
};
use crate::headers::sequence::{BitDepthIdc, ChromaFormatIdc, SequenceHeader, SuperblockSize};

/// The § 7.23 slot state after one closed-loop key frame refreshed every slot:
/// only slot 0 is valid (§ 7.20), with order hint `hint`.
struct Slots {
    valid: [bool; 8],
    hints: [u32; 8],
    widths: [u32; 8],
    heights: [u32; 8],
    base_q: [u32; 8],
    shown: [bool; 8],
    hidden: [bool; 8],
    counters: [u32; 8],
}

impl Slots {
    fn after_key(width: u32, height: u32, hint: u32) -> Self {
        let mut valid = [false; 8];
        valid[0] = true;
        Self {
            valid,
            hints: [hint; 8],
            widths: [width; 8],
            heights: [height; 8],
            base_q: [60; 8],
            shown: [true; 8],
            hidden: [false; 8],
            counters: [0; 8],
        }
    }

    fn view(&self) -> FrameReferenceStateView<'_> {
        FrameReferenceStateView::from_slots_with_base_q_idx(
            &self.valid,
            &self.hints,
            &self.widths,
            &self.heights,
            &self.base_q,
        )
        .with_single_layer_order_hint_state(&self.hints, &self.hidden, &self.shown)
        .with_primary_reference_state(&self.counters, &self.hidden)
    }
}

fn sequence(width: u32, height: u32, bit_depth: BitDepthIdc, sb: SuperblockSize) -> SequenceHeader {
    build_encoder_sequence_header(&EncoderStreamParams {
        width,
        height,
        bit_depth,
        chroma: ChromaFormatIdc::Yuv420,
        sb_size: sb,
        tools: EncoderToolFlags {
            cdef: true,
            loop_restoration: true,
            ..EncoderToolFlags::default()
        },
    })
    .unwrap()
}

fn filters(sb: SuperblockSize) -> FrameFilterParams {
    FrameFilterParams {
        deblocking: [true, true, true, false],
        deblocking_delta_q: [1, -2, 3, 0],
        cdef_damping: 4,
        cdef_strengths: CdefStrengthSets::from_slice(&[CdefStrengthSet {
            y_pri_strength: 4,
            y_sec_strength: 1,
            uv_pri_strength: 2,
            uv_sec_strength: 0,
        }])
        .unwrap(),
        lr_luma_unit_size: Some(if sb == SuperblockSize::Block64x64 {
            64
        } else {
            128
        }),
    }
}

#[test]
fn inter_headers_parse_with_the_reference_state() {
    for (width, height) in [(1, 1), (17, 9), (200, 136), (1920, 1080)] {
        for bit_depth in [BitDepthIdc::Eight, BitDepthIdc::Ten] {
            for sb in [SuperblockSize::Block64x64, SuperblockSize::Block128x128] {
                let seq = sequence(width, height, bit_depth, sb);
                let view = CoreSeqView::from_sequence(&seq).unwrap();
                let slots = Slots::after_key(width, height, 4);
                for (show_frame, filters, compound, primary_slot) in [
                    (true, FrameFilterParams::default(), false, None),
                    (false, filters(sb), true, Some(0)),
                ] {
                    let params = InterFrameParams {
                        base_q_idx: 90,
                        order_hint: 5,
                        refresh_frame_flags: 0b10,
                        show_frame,
                        filters,
                        tx: KeyFrameTxParams::default(),
                        tiles: UniformTiles::default(),
                        reference_select: compound,
                        skip_mode_present: compound,
                        primary_slot,
                    };
                    let header = build_inter_frame_header(&view, &slots.view(), &params).unwrap();
                    let core = &header.core;
                    assert_eq!(core.frame_type, Some(FrameType::Inter));
                    assert_eq!(core.immediate_output_frame, Some(show_frame));
                    assert_eq!(core.implicit_output_frame, Some(false));
                    assert_eq!(core.order_hint, Some(5));
                    assert_eq!(core.refresh_frame_flags, Some(0b10));
                    assert_eq!(core.frame_size.unwrap().width, width);
                    let inter = core.inter.as_ref().unwrap();
                    assert_eq!(&inter.ref_frame_idx[..], &[0]);
                    let primary = primary_slot.map_or(PRIMARY_REF_NONE as u8, |_| 0);
                    assert_eq!(inter.primary_ref_frame, Some(primary));
                    assert_eq!(
                        inter.disable_cross_frame_cdf_init,
                        Some(primary_slot.is_none())
                    );
                    let tail = core.inter_tail.as_ref().unwrap();
                    assert_eq!(tail.tx_mode, TxMode::Largest);
                    assert_eq!(tail.reference_select, compound);
                    assert_eq!(tail.skip_mode_present, compound);
                    assert_eq!(
                        tail.reduced_tx_set,
                        KeyFrameTxParams::default().reduced_tx_set
                    );
                    assert_eq!(core.quantization_params.unwrap().base_q_idx, 90);
                    assert_eq!(
                        core.cdef_params.as_ref().unwrap().cdef_frame_enable,
                        !show_frame
                    );
                    assert_eq!(core.consumed_bits, header.bits.num_frame_header_bits());
                }
            }
        }
    }
}

#[test]
fn order_hints_unwrap_against_the_reference_slots() {
    let seq = sequence(64, 64, BitDepthIdc::Eight, SuperblockSize::Block64x64);
    let view = CoreSeqView::from_sequence(&seq).unwrap();
    let slots = Slots::after_key(64, 64, 300);
    let params = InterFrameParams {
        base_q_idx: 90,
        order_hint: 301,
        refresh_frame_flags: 1,
        show_frame: true,
        filters: FrameFilterParams::default(),
        tx: KeyFrameTxParams::default(),
        tiles: UniformTiles::default(),
        reference_select: false,
        skip_mode_present: false,
        primary_slot: Some(3),
    };
    let header = build_inter_frame_header(&view, &slots.view(), &params).unwrap();
    let inter = header.core.inter.as_ref().unwrap();
    assert_eq!(
        inter.primary_ref_frame,
        Some(PRIMARY_REF_NONE as u8),
        "slot 3 is empty"
    );
    assert_eq!(header.core.order_hint_lsb, Some(301 - 256));
    assert_eq!(header.core.order_hint, Some(301));
}

#[test]
fn inter_obu_carries_the_recorded_header_and_tile() {
    let seq = sequence(130, 66, BitDepthIdc::Ten, SuperblockSize::Block64x64);
    let view = CoreSeqView::from_sequence(&seq).unwrap();
    let slots = Slots::after_key(130, 66, 0);
    let header = build_inter_frame_header(
        &view,
        &slots.view(),
        &InterFrameParams {
            base_q_idx: 120,
            order_hint: 1,
            refresh_frame_flags: 1,
            show_frame: true,
            filters: filters(SuperblockSize::Block64x64),
            tx: KeyFrameTxParams::default(),
            tiles: UniformTiles::default(),
            reference_select: true,
            skip_mode_present: false,
            primary_slot: Some(0),
        },
    )
    .unwrap();
    let tile = [0xab, 0xcd, 0xef];
    let obu = encode_inter_frame_obu(&header, &[&tile]).unwrap();
    let parsed = parse_annex_b_obus_partial(&obu);
    assert!(parsed.error.is_none());
    assert_eq!(parsed.obus.len(), 1);
    let frame = &parsed.obus[0];
    assert_eq!(frame.header.obu_type, ObuType::RegularTileGroup);
    assert!(frame.payload.ends_with(&tile));
    let mut reader = BitReader::new(frame.payload, ByteOffset::new(0));
    assert_eq!(reader.read_bit().unwrap(), 1, "is_first_tile_group");
    let input = FrameHeaderParseInput {
        obu_type: ObuType::RegularTileGroup,
        first_picture_in_tu: true,
        active_sequence: Some(&seq),
        mfh_record: None,
        reference_state: slots.view(),
        mode: FrameHeaderParseMode::Core,
    };
    assert_eq!(
        parse_frame_header_core(&mut reader, &input).unwrap(),
        header.core
    );
}

#[test]
fn show_existing_frame_obu_parses_complete() {
    let seq = sequence(17, 9, BitDepthIdc::Eight, SuperblockSize::Block64x64);
    let view = CoreSeqView::from_sequence(&seq).unwrap();
    let obu = encode_show_existing_frame_obu(&view, 5).unwrap();
    let parsed = parse_annex_b_obus_partial(&obu);
    assert!(parsed.error.is_none());
    assert_eq!(parsed.obus.len(), 1);
    let sef = &parsed.obus[0];
    assert_eq!(sef.header.obu_type, ObuType::RegularSef);
    let slots = Slots::after_key(17, 9, 7);
    let input = FrameHeaderParseInput {
        obu_type: ObuType::RegularSef,
        first_picture_in_tu: true,
        active_sequence: Some(&seq),
        mfh_record: None,
        reference_state: slots.view(),
        mode: FrameHeaderParseMode::Core,
    };
    let core =
        parse_frame_header_core(&mut BitReader::new(sef.payload, ByteOffset::new(0)), &input)
            .unwrap();
    assert_eq!(
        core.status,
        FrameHeaderParseStatus::ShowExistingFrameComplete
    );
    assert_eq!(core.frame_to_show_map_idx, Some(5));
    assert_eq!(core.order_hint, Some(7));
    assert!(encode_show_existing_frame_obu(&view, 8).is_err());
}

#[test]
fn rejects_out_of_range_inter_inputs() {
    let seq = sequence(64, 64, BitDepthIdc::Eight, SuperblockSize::Block64x64);
    let view = CoreSeqView::from_sequence(&seq).unwrap();
    let slots = Slots::after_key(64, 64, 0);
    let params = InterFrameParams {
        base_q_idx: 0,
        order_hint: 1,
        refresh_frame_flags: 1,
        show_frame: true,
        filters: FrameFilterParams::default(),
        tx: KeyFrameTxParams::default(),
        tiles: UniformTiles::default(),
        reference_select: false,
        skip_mode_present: false,
        primary_slot: None,
    };
    assert!(matches!(
        build_inter_frame_header(&view, &slots.view(), &params),
        Err(EncoderHeaderError::InvalidBaseQIdx { .. })
    ));
    let no_valid_slot = Slots {
        valid: [false; 8],
        ..Slots::after_key(64, 64, 0)
    };
    let params = InterFrameParams {
        base_q_idx: 60,
        ..params
    };
    assert!(build_inter_frame_header(&view, &no_valid_slot.view(), &params).is_err());
}
