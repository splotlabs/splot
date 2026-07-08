// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Unit tests for general intra transform-block coefficient decode helpers.

#![allow(clippy::unwrap_used)]

use splot_core::segment::MAX_SEGMENTS;
use splot_core::span::ByteOffset;
use splot_core::symbol::{CdfUpdateMode, Symbol, SymbolDecoderConfig};
use splot_core::symbol_encoder::{SymbolEncoder, SymbolEncoderConfig};

use super::*;
use crate::bitstream::tile_payload::TileCoeffFrameFactsInput;

const PAYLOAD: [u8; 2] = [0x00, 0x80];

fn symbol_decoder_for_payload(payload: &[u8]) -> SymbolDecoder<'_> {
    SymbolDecoder::with_base_and_config(
        payload,
        ByteOffset::new(0),
        SymbolDecoderConfig::new().with_cdf_update_mode(CdfUpdateMode::Disabled),
    )
    .unwrap()
}

fn tile_cdfs() -> TileCdfSubset {
    crate::bitstream::tile_payload::FrameCdfSubset::from_defaults().tile_copy()
}

fn encode_transform_symbols(sequence: &[(TileCdfSelector, u8)]) -> Vec<u8> {
    let mut cdfs = tile_cdfs();
    let mut encoder = SymbolEncoder::with_config(
        SymbolEncoderConfig::new().with_cdf_update_mode(CdfUpdateMode::Disabled),
    );
    for &(selector, value) in sequence {
        cdfs.with_row_mut(selector, |row| {
            encoder.write_symbol(row, Symbol::new(value))
        })
        .unwrap()
        .unwrap();
    }
    encoder.finish().unwrap().into_bytes()
}

fn sec_tx_type_payload(
    tx_size: usize,
    sec_tx_type: u8,
    most_probable_stx_set: Option<u8>,
) -> Vec<u8> {
    let tx_size_sqr = TX_SIZE_SQR[tx_size] as usize;
    let mut sequence = vec![(
        TileCdfSelector::SecTxType {
            is_inter: 0,
            tx_size_sqr,
        },
        sec_tx_type,
    )];
    if let Some(stx_set) = most_probable_stx_set {
        sequence.push((TileCdfSelector::MostProbableStxSet, stx_set));
    }
    encode_transform_symbols(&sequence)
}

fn intra_tx_type_set1_payload(tx_size: usize, intra_tx_type: u8) -> Vec<u8> {
    encode_transform_symbols(&[(
        TileCdfSelector::IntraTxTypeSet1 {
            tx_size_sqr: TX_SIZE_SQR[tx_size] as usize,
        },
        intra_tx_type,
    )])
}

fn intra_tx_type_set1_with_sec_tx_payload(
    tx_size: usize,
    intra_tx_type: u8,
    sec_tx_type: u8,
    stx_selector: TileCdfSelector,
    most_probable_stx_set: u8,
) -> Vec<u8> {
    encode_transform_symbols(&[
        (
            TileCdfSelector::IntraTxTypeSet1 {
                tx_size_sqr: TX_SIZE_SQR[tx_size] as usize,
            },
            intra_tx_type,
        ),
        (
            TileCdfSelector::SecTxType {
                is_inter: 0,
                tx_size_sqr: TX_SIZE_SQR[tx_size] as usize,
            },
            sec_tx_type,
        ),
        (stx_selector, most_probable_stx_set),
    ])
}

fn dc_luma_context() -> LumaTransformTypeContext {
    LumaTransformTypeContext::new(IntraYMode::DC_PRED, 0)
}

#[test]
fn reconstruct_with_prediction_rejects_wrong_prediction_length() {
    let quant = vec![0i32; 16];
    let prediction = vec![128u8; 8];
    let result = reconstruct_general_intra_block_rect_with_prediction(
        &quant,
        &prediction,
        64,
        PlaneId::Y,
        2,
        2,
        0,
        false,
        BitDepth::Eight,
    );
    assert!(matches!(
        result,
        Err(GeneralIntraResidualError::PredictionLength {
            expected: 16,
            actual: 8
        })
    ));
}

#[test]
fn txb_skip_tx_size_ctx_matches_spec_formula_for_square_sizes() {
    for (tx_size, expected) in [(0, 0), (1, 1), (2, 2), (3, 3), (TX_64X64, 4)] {
        assert_eq!(txb_skip_tx_size_ctx(tx_size), expected);
    }
}

#[test]
fn txb_skip_tx_size_ctx_is_total_for_out_of_range_tx_size() {
    assert_eq!(txb_skip_tx_size_ctx(usize::MAX), 0);
    assert_eq!(txb_skip_tx_size_ctx(TX_SIZE_SQR.len()), 0);
}

#[test]
fn split_luma_transform_partition_records_follow_raster_order() {
    let records =
        luma_transform_records_for_partition(64, 32, TX_16X16, TX_PARTITION_SPLIT).unwrap();
    let coords: Vec<(usize, usize, usize)> = records
        .iter()
        .map(|record| (record.x, record.y, record.tx_size))
        .collect();

    assert_eq!(
        coords,
        vec![
            (64, 32, TX_8X8),
            (72, 32, TX_8X8),
            (64, 40, TX_8X8),
            (72, 40, TX_8X8),
        ]
    );
}

#[test]
fn partitioned_luma_transform_record_does_not_fill_block_for_txb_skip_ctx() {
    let records =
        luma_transform_records_for_partition(64, 32, TX_16X16, TX_PARTITION_SPLIT).unwrap();
    assert!(!luma_partition_record_fills_block(
        true,
        records.len(),
        records[0],
        64,
        32
    ));
    assert_eq!(txb_skip_ctx_luma(0, 0, false, false), 1);
    assert_eq!(txb_skip_ctx_luma(0, 0, true, false), 0);

    let unpartitioned = [LumaTransformPartitionRecord {
        middle: false,
        x: 64,
        y: 32,
        tx_size: TX_16X16,
    }];
    assert!(luma_partition_record_fills_block(
        true,
        unpartitioned.len(),
        unpartitioned[0],
        64,
        32
    ));
}

#[allow(clippy::fn_params_excessive_bools)]
fn frame_facts(
    enable_idtx_intra: bool,
    enable_intra_ist: bool,
    enable_chroma_dctonly: bool,
    enable_cctx: bool,
) -> TileCoeffFrameFacts {
    TileCoeffFrameFacts::new(TileCoeffFrameFactsInput {
        enable_idtx_intra,
        enable_intra_ist,
        enable_chroma_dctonly,
        enable_cctx,
        ..frame_facts_input()
    })
}

fn frame_facts_with_coeff_tools(allow_tcq: bool, allow_parity_hiding: bool) -> TileCoeffFrameFacts {
    TileCoeffFrameFacts::new(TileCoeffFrameFactsInput {
        enable_idtx_intra: true,
        allow_tcq,
        allow_parity_hiding,
        ..frame_facts_input()
    })
}

fn frame_facts_with_fsc() -> TileCoeffFrameFacts {
    TileCoeffFrameFacts::new(TileCoeffFrameFactsInput {
        enable_fsc: true,
        enable_idtx_intra: true,
        ..frame_facts_input()
    })
}

fn lossless_frame_facts() -> TileCoeffFrameFacts {
    let mut input = frame_facts_input();
    input.enable_idtx_intra = true;
    input.enable_intra_ist = true;
    input.lossless_array[0] = true;
    TileCoeffFrameFacts::new(input)
}

fn frame_facts_input() -> TileCoeffFrameFactsInput {
    TileCoeffFrameFactsInput {
        enable_fsc: false,
        enable_idtx_intra: false,
        enable_intra_ist: false,
        enable_inter_ist: false,
        enable_chroma_dctonly: false,
        enable_cctx: false,
        reduced_tx_set: 0,
        lossless_array: [false; MAX_SEGMENTS],
        allow_tcq: false,
        allow_parity_hiding: false,
        base_q_idx: 128,
    }
}

fn unsupported_reason<T>(result: &Result<T, GeneralIntraResidualError>) -> Option<&'static str> {
    match result {
        Err(GeneralIntraResidualError::UnsupportedTransformToolResidual { reason }) => {
            Some(*reason)
        }
        _ => None,
    }
}

fn ensure_with_test_state(
    facts: TileCoeffFrameFacts,
    plane: usize,
    tx_size: usize,
    is_inter: bool,
    eob: usize,
    luma: Option<LumaTransformTypeContext>,
) -> Result<TransformToolResidualMetadata, GeneralIntraResidualError> {
    ensure_with_test_payload_and_policy(
        facts,
        plane,
        tx_size,
        is_inter,
        eob,
        luma,
        ActiveIntraIstResidualPolicy::Reject,
        ActiveChromaResidualPolicy::Reject,
        &PAYLOAD,
    )
}

#[allow(clippy::too_many_arguments)]
fn ensure_with_test_payload_and_policy(
    facts: TileCoeffFrameFacts,
    plane: usize,
    tx_size: usize,
    is_inter: bool,
    eob: usize,
    luma: Option<LumaTransformTypeContext>,
    active_intra_ist_policy: ActiveIntraIstResidualPolicy,
    active_chroma_policy: ActiveChromaResidualPolicy,
    payload: &[u8],
) -> Result<TransformToolResidualMetadata, GeneralIntraResidualError> {
    ensure_with_test_payload_fsc_and_policy(
        facts,
        plane,
        tx_size,
        is_inter,
        false,
        eob,
        luma,
        active_intra_ist_policy,
        active_chroma_policy,
        payload,
    )
}

#[allow(clippy::too_many_arguments)]
fn ensure_with_test_payload_fsc_and_policy(
    facts: TileCoeffFrameFacts,
    plane: usize,
    tx_size: usize,
    is_inter: bool,
    fsc_mode: bool,
    eob: usize,
    luma: Option<LumaTransformTypeContext>,
    active_intra_ist_policy: ActiveIntraIstResidualPolicy,
    active_chroma_policy: ActiveChromaResidualPolicy,
    payload: &[u8],
) -> Result<TransformToolResidualMetadata, GeneralIntraResidualError> {
    let mut cdfs = tile_cdfs();
    let mut symbols = symbol_decoder_for_payload(payload);
    ensure_transform_tool_residual_handoff(
        &mut cdfs,
        &mut symbols,
        TransformToolResidualInput {
            frame_facts: facts,
            plane,
            tx_size,
            is_inter,
            lossless: facts
                .lossless_for_segment(current_segment_id())
                .unwrap_or(false),
            fsc_mode,
            eob,
            luma_transform_type_context: luma,
            active_intra_ist_policy,
            active_chroma_policy,
        },
    )
}

#[test]
fn lossless_intra_transform_handoff_forces_dct_without_tx_type_or_ist_reads() {
    let payload = intra_tx_type_set1_with_sec_tx_payload(
        TX_8X8,
        1,
        1,
        TileCdfSelector::MostProbableStxSet,
        2,
    );
    let mut cdfs = tile_cdfs();
    let mut symbols = symbol_decoder_for_payload(&payload);

    let metadata = ensure_transform_tool_residual_handoff(
        &mut cdfs,
        &mut symbols,
        TransformToolResidualInput {
            frame_facts: lossless_frame_facts(),
            plane: 0,
            tx_size: TX_8X8,
            is_inter: false,
            lossless: true,
            fsc_mode: false,
            eob: 2,
            luma_transform_type_context: Some(dc_luma_context()),
            active_intra_ist_policy: ActiveIntraIstResidualPolicy::LrTxSkipRecordHandoff,
            active_chroma_policy: ActiveChromaResidualPolicy::Reject,
        },
    )
    .unwrap();

    assert_eq!(metadata.luma_tx_type, DCT_DCT);
    assert_eq!(metadata.intra_ist, None);
    assert_eq!(symbols.symbol_count(), 0);
}

#[test]
fn fsc_mode_luma_transform_handoff_derives_idtx_without_luma_context() {
    let metadata = ensure_with_test_payload_fsc_and_policy(
        frame_facts_with_fsc(),
        0,
        TX_8X8,
        false,
        true,
        2,
        None,
        ActiveIntraIstResidualPolicy::LrTxSkipRecordHandoff,
        ActiveChromaResidualPolicy::Reject,
        &PAYLOAD,
    )
    .unwrap();

    assert_eq!(metadata.luma_tx_type, IDTX);
    assert_eq!(metadata.intra_ist, None);
}

#[test]
fn non_fsc_luma_transform_handoff_still_requires_luma_context() {
    let result = ensure_with_test_payload_fsc_and_policy(
        frame_facts_with_fsc(),
        0,
        TX_8X8,
        false,
        false,
        2,
        None,
        ActiveIntraIstResidualPolicy::LrTxSkipRecordHandoff,
        ActiveChromaResidualPolicy::Reject,
        &PAYLOAD,
    );

    assert_eq!(
        unsupported_reason(&result),
        Some("unsupported_dctonly_residual_luma_transform_context")
    );
}

#[test]
fn dctonly_residual_admits_luma_when_ist_cannot_read_after_eob_limit() {
    let result = ensure_with_test_state(
        frame_facts(true, true, false, false),
        0,
        TX_32X32,
        false,
        IST_8X8_HEIGHT + 1,
        Some(dc_luma_context()),
    );

    assert!(result.is_ok());
}

#[test]
fn dctonly_residual_admits_luma_when_intra_ist_reads_zero_sec_tx_type() {
    let result = ensure_with_test_state(
        frame_facts(true, true, false, false),
        0,
        TX_32X32,
        false,
        2,
        Some(dc_luma_context()),
    );

    assert!(result.is_ok());
}

#[test]
fn dctonly_residual_rejects_intra_ist_without_luma_context() {
    let result = ensure_with_test_state(
        frame_facts(true, true, false, false),
        0,
        TX_32X32,
        false,
        2,
        None,
    );

    assert_eq!(
        unsupported_reason(&result),
        Some("unsupported_dctonly_residual_intra_ist_context")
    );
}

#[test]
fn dctonly_residual_rejects_active_intra_ist_sec_tx_type() {
    let result = ensure_supported_intra_ist_sec_tx_type(
        IntraIstSyntax {
            sec_tx_type: 1,
            most_probable_stx_set: Some(2),
        },
        ActiveIntraIstResidualPolicy::Reject,
    );

    assert_eq!(
        unsupported_reason(&result),
        Some("unsupported_dctonly_residual_intra_sec_tx_type")
    );
}

#[test]
fn dctonly_residual_lr_handoff_admits_active_intra_ist_metadata() {
    let payload = sec_tx_type_payload(TX_32X32, 1, Some(2));

    let metadata = ensure_with_test_payload_and_policy(
        frame_facts(true, true, false, false),
        0,
        TX_32X32,
        false,
        2,
        Some(dc_luma_context()),
        ActiveIntraIstResidualPolicy::LrTxSkipRecordHandoff,
        ActiveChromaResidualPolicy::Reject,
        &payload,
    )
    .unwrap();

    assert_eq!(
        metadata.intra_ist,
        Some(IntraIstSyntax {
            sec_tx_type: 1,
            most_probable_stx_set: Some(2),
        })
    );
}

#[test]
fn dctonly_residual_safe_policy_rejects_encoded_active_intra_ist() {
    let payload = sec_tx_type_payload(TX_32X32, 1, Some(2));

    let result = ensure_with_test_payload_and_policy(
        frame_facts(true, true, false, false),
        0,
        TX_32X32,
        false,
        2,
        Some(dc_luma_context()),
        ActiveIntraIstResidualPolicy::Reject,
        ActiveChromaResidualPolicy::Reject,
        &payload,
    );

    assert_eq!(
        unsupported_reason(&result),
        Some("unsupported_dctonly_residual_intra_sec_tx_type")
    );
}

#[test]
fn dctonly_residual_maps_nonzero_intra_tx_type_to_non_dct() {
    let tx_type = md_idx_luma_tx_type(TX_8X8, dc_luma_context(), 1).unwrap();

    assert_ne!(tx_type, DCT_DCT);
}

const INTER_SET_TX_SIZE: usize = TX_8X16;
const INTER_SET_EOB: usize = 10;

fn read_inter_tx_type_from_symbols(tx_set: usize, sequence: &[(TileCdfSelector, u8)]) -> usize {
    let payload = encode_transform_symbols(sequence);
    let mut cdfs = tile_cdfs();
    let mut symbols = symbol_decoder_for_payload(&payload);
    read_active_inter_transform_type(
        &mut cdfs,
        &mut symbols,
        INTER_SET_TX_SIZE,
        tx_set,
        INTER_SET_EOB,
    )
    .unwrap()
}

#[test]
fn inter_tx_set_ctx_for_frontier_block_is_zero() {
    assert_eq!(
        inter_tx_type_long_ctx(INTER_SET_TX_SIZE, INTER_SET_EOB).unwrap(),
        0
    );
}

#[test]
fn inter_set1_index_branch_inverts_via_tx_type_inter_inv_set1() {
    let tx_size_sqr = TX_SIZE_SQR[INTER_SET_TX_SIZE] as usize;
    let tx_type = read_inter_tx_type_from_symbols(
        TX_SET_INTER_1,
        &[
            (
                TileCdfSelector::InterTxTypeSet1 {
                    ctx: 0,
                    tx_size_sqr,
                },
                0,
            ),
            (TileCdfSelector::InterTxTypeIndexSet1 { ctx: 0 }, 7),
        ],
    );
    assert_eq!(tx_type, TX_TYPE_INTER_INV_SET1[7]);
    assert_eq!(tx_type, DCT_DCT);
}

#[test]
fn inter_set1_offset_branch_inverts_via_tx_type_inter_inv_set1() {
    let tx_size_sqr = TX_SIZE_SQR[INTER_SET_TX_SIZE] as usize;
    let tx_type = read_inter_tx_type_from_symbols(
        TX_SET_INTER_1,
        &[
            (
                TileCdfSelector::InterTxTypeSet1 {
                    ctx: 0,
                    tx_size_sqr,
                },
                1,
            ),
            (TileCdfSelector::InterTxTypeOffsetSet1 { ctx: 0 }, 0),
        ],
    );
    assert_eq!(tx_type, TX_TYPE_INTER_INV_SET1[8]);
    assert_eq!(tx_type, ADST_DCT);
}

#[test]
fn inter_set2_index_branch_inverts_via_tx_type_inter_inv_set2() {
    let tx_type = read_inter_tx_type_from_symbols(
        TX_SET_INTER_2,
        &[
            (TileCdfSelector::InterTxTypeSet2 { ctx: 0 }, 0),
            (TileCdfSelector::InterTxTypeIndexSet2 { ctx: 0 }, 3),
        ],
    );
    assert_eq!(tx_type, TX_TYPE_INTER_INV_SET2[3]);
    assert_eq!(tx_type, DCT_DCT);
}

#[test]
fn inter_set2_offset_branch_inverts_via_tx_type_inter_inv_set2() {
    let tx_type = read_inter_tx_type_from_symbols(
        TX_SET_INTER_2,
        &[
            (TileCdfSelector::InterTxTypeSet2 { ctx: 0 }, 1),
            (TileCdfSelector::InterTxTypeOffsetSet2 { ctx: 0 }, 0),
        ],
    );
    assert_eq!(tx_type, TX_TYPE_INTER_INV_SET2[8]);
    assert_eq!(tx_type, ADST_ADST);
}

#[test]
fn inter_dct_idtx_set3_inverts_idtx_and_dct_dct() {
    let tx_size_sqr = TX_SIZE_SQR[INTER_SET_TX_SIZE] as usize;
    let idtx = read_inter_tx_type_from_symbols(
        TX_SET_DCT_IDTX,
        &[(
            TileCdfSelector::InterTxTypeSet3 {
                ctx: 0,
                tx_size_sqr,
            },
            0,
        )],
    );
    let dct = read_inter_tx_type_from_symbols(
        TX_SET_DCT_IDTX,
        &[(
            TileCdfSelector::InterTxTypeSet3 {
                ctx: 0,
                tx_size_sqr,
            },
            1,
        )],
    );
    assert_eq!(idtx, IDTX);
    assert_eq!(dct, DCT_DCT);
}

#[test]
fn inter_dct_idtx_iddct_set4_inverts_per_spec_table() {
    let tx_size_sqr = TX_SIZE_SQR[INTER_SET_TX_SIZE] as usize;
    for (symbol, expected) in [(0u8, DCT_DCT), (1, V_DCT), (2, H_DCT), (3, IDTX)] {
        let tx_type = read_inter_tx_type_from_symbols(
            TX_SET_DCT_IDTX_IDDCT,
            &[(
                TileCdfSelector::InterTxTypeSet4 {
                    ctx: 0,
                    tx_size_sqr,
                },
                symbol,
            )],
        );
        assert_eq!(tx_type, expected);
    }
}

#[test]
fn read_active_inter_transform_type_rejects_unmodeled_set() {
    const UNMODELED_TX_SET: usize = 99;
    let mut cdfs = tile_cdfs();
    let mut symbols = symbol_decoder_for_payload(&PAYLOAD);
    let result = read_active_inter_transform_type(
        &mut cdfs,
        &mut symbols,
        INTER_SET_TX_SIZE,
        UNMODELED_TX_SET,
        INTER_SET_EOB,
    );
    assert!(matches!(
        result,
        Err(
            GeneralIntraResidualError::UnsupportedTransformToolResidual {
                reason: "unsupported_dctonly_residual_inter_tx_set"
            }
        )
    ));
}

#[test]
fn luma_transform_context_applies_mrl_delta_before_wide_angle_mapping() {
    let luma =
        crate::bitstream::tile_payload::cdf::block_context::reconstruct_y_mode_second_set_top_left(
            1, 6,
        )
        .unwrap();
    assert_eq!(luma.y_mode.value(), 8);
    assert_eq!(luma.angle_delta_y, -2);

    let no_mrl =
        md_idx_luma_tx_type(TX_8X16, LumaTransformTypeContext::new(luma.y_mode, -2), 4).unwrap();
    let active_mrl = md_idx_luma_tx_type(
        TX_8X16,
        LumaTransformTypeContext::with_mrl_index(luma.y_mode, -2, 2),
        4,
    )
    .unwrap();

    assert_ne!(no_mrl, active_mrl);
    assert_eq!(no_mrl, DCT_FLIPADST);
    assert_eq!(active_mrl, FLIPADST_DCT);
}

#[test]
fn luma_txtype_residual_lr_handoff_retains_non_dct_luma_tx_type() {
    let payload = intra_tx_type_set1_payload(TX_8X8, 1);
    let luma = dc_luma_context();
    let expected = md_idx_luma_tx_type(TX_8X8, luma, 1).unwrap();

    let metadata = ensure_with_test_payload_and_policy(
        frame_facts(false, false, false, false),
        0,
        TX_8X8,
        false,
        2,
        Some(luma),
        ActiveIntraIstResidualPolicy::LrTxSkipRecordHandoff,
        ActiveChromaResidualPolicy::Reject,
        &payload,
    )
    .unwrap();

    assert_ne!(expected, DCT_DCT);
    assert_eq!(metadata.luma_tx_type, expected);
}

#[test]
fn luma_txtype_residual_lr_handoff_skips_intra_ist_for_non_sec_tx_type() {
    let payload = intra_tx_type_set1_payload(TX_8X8, 2);
    let luma = dc_luma_context();
    let expected = md_idx_luma_tx_type(TX_8X8, luma, 2).unwrap();

    let metadata = ensure_with_test_payload_and_policy(
        frame_facts(false, true, false, false),
        0,
        TX_8X8,
        false,
        2,
        Some(luma),
        ActiveIntraIstResidualPolicy::LrTxSkipRecordHandoff,
        ActiveChromaResidualPolicy::Reject,
        &payload,
    )
    .unwrap();

    assert_ne!(expected, DCT_DCT);
    assert_ne!(expected, ADST_ADST);
    assert_eq!(metadata.luma_tx_type, expected);
    assert_eq!(metadata.intra_ist, None);
}

#[test]
fn luma_txtype_residual_adst_adst_uses_reduced_ist_eob_limit() {
    let payload = intra_tx_type_set1_payload(TX_16X16, 1);
    let luma = dc_luma_context();
    let expected = md_idx_luma_tx_type(TX_16X16, luma, 1).unwrap();

    let metadata = ensure_with_test_payload_and_policy(
        frame_facts(false, true, false, false),
        0,
        TX_16X16,
        false,
        IST_8X8_HEIGHT_RED + 1,
        Some(luma),
        ActiveIntraIstResidualPolicy::LrTxSkipRecordHandoff,
        ActiveChromaResidualPolicy::Reject,
        &payload,
    )
    .unwrap();

    assert_eq!(expected, ADST_ADST);
    assert_eq!(metadata.luma_tx_type, ADST_ADST);
    assert_eq!(metadata.intra_ist, None);
}

#[test]
fn luma_txtype_residual_adst_adst_uses_reduced_ist_stx_cdf() {
    let payload = intra_tx_type_set1_with_sec_tx_payload(
        TX_8X8,
        1,
        1,
        TileCdfSelector::MostProbableStxSetAdst,
        2,
    );
    let luma = dc_luma_context();
    let expected = md_idx_luma_tx_type(TX_8X8, luma, 1).unwrap();

    let metadata = ensure_with_test_payload_and_policy(
        frame_facts(false, true, false, false),
        0,
        TX_8X8,
        false,
        5,
        Some(luma),
        ActiveIntraIstResidualPolicy::LrTxSkipRecordHandoff,
        ActiveChromaResidualPolicy::Reject,
        &payload,
    )
    .unwrap();

    assert_eq!(expected, ADST_ADST);
    assert_eq!(metadata.luma_tx_type, ADST_ADST);
    assert_eq!(
        metadata.intra_ist,
        Some(IntraIstSyntax {
            sec_tx_type: 1,
            most_probable_stx_set: Some(2),
        })
    );
}

#[test]
fn intra_ist_stx_selector_uses_reduced_adst_cdf_for_every_large_size() {
    for tx_size in 0..TX_WIDTH.len() {
        let (width, height) = tx_size_dimensions(tx_size).unwrap();
        let expected = if width >= 8 && height >= 8 {
            TileCdfSelector::MostProbableStxSetAdst
        } else {
            TileCdfSelector::MostProbableStxSet
        };
        assert_eq!(
            intra_ist_most_probable_stx_selector(tx_size, ADST_ADST).unwrap(),
            expected,
            "tx_size={tx_size} width={width} height={height}"
        );
        assert_eq!(
            intra_ist_most_probable_stx_selector(tx_size, DCT_DCT).unwrap(),
            TileCdfSelector::MostProbableStxSet,
            "tx_size={tx_size} width={width} height={height}"
        );
    }
}

#[test]
fn luma_txtype_residual_staged_base_config_uses_retained_luma_tx_type() {
    let luma = dc_luma_context();
    let expected = md_idx_luma_tx_type(TX_8X8, luma, 1).unwrap();

    let config = staged_transform_tool_lossless_base_config(
        frame_facts(false, false, false, false),
        0,
        0,
        0,
        DCT_DCT,
        false,
        TransformToolResidualMetadata {
            luma_tx_type: expected,
            ..TransformToolResidualMetadata::default()
        },
    );

    assert_ne!(expected, DCT_DCT);
    assert_eq!(config.luma_tx_type, expected);
}

#[test]
fn luma_txtype_residual_staged_base_config_derives_coeff_tool_flags() {
    for (luma_tx_type, parity_hiding, use_tcq) in [
        (ADST_DCT, true, true),
        (IDTX, false, true),
        (V_DCT, true, false),
    ] {
        let config = staged_transform_tool_lossless_base_config(
            frame_facts_with_coeff_tools(true, true),
            0,
            0,
            0,
            DCT_DCT,
            false,
            TransformToolResidualMetadata {
                luma_tx_type,
                ..TransformToolResidualMetadata::default()
            },
        );

        assert_eq!(config.parity_hiding, parity_hiding);
        assert_eq!(config.use_tcq, use_tcq);
    }
}

#[test]
fn coefficient_block_use_tcq_suppresses_fsc() {
    let facts = frame_facts_with_coeff_tools(true, true);

    assert!(coefficient_block_use_tcq(facts, 0, IDTX, false, false));
    assert!(!coefficient_block_use_tcq(facts, 0, IDTX, false, true));
}

#[test]
fn luma_txtype_residual_safe_policy_rejects_non_dct_luma_tx_type() {
    let payload = intra_tx_type_set1_payload(TX_8X8, 1);

    let result = ensure_with_test_payload_and_policy(
        frame_facts(false, false, false, false),
        0,
        TX_8X8,
        false,
        2,
        Some(dc_luma_context()),
        ActiveIntraIstResidualPolicy::Reject,
        ActiveChromaResidualPolicy::Reject,
        &payload,
    );

    assert_eq!(
        unsupported_reason(&result),
        Some("unsupported_dctonly_residual_luma_tx_type")
    );
}

#[test]
fn dctonly_residual_rejects_u_plane_cctx_only_when_eob_requires_cctx_type() {
    let facts = frame_facts(false, false, false, true);
    let eob_one = ensure_with_test_state(facts, 1, TX_32X32, false, 1, None);
    let eob_two = ensure_with_test_state(facts, 1, TX_32X32, false, 2, None);

    assert!(eob_one.is_ok());
    assert_eq!(
        unsupported_reason(&eob_two),
        Some("unsupported_dctonly_residual_cctx")
    );
}

#[test]
fn lossless_chroma_transform_handoff_skips_cctx_read() {
    let mut input = frame_facts_input();
    input.enable_cctx = true;
    input.lossless_array[0] = true;
    let facts = TileCoeffFrameFacts::new(input);
    let payload = encode_transform_symbols(&[(TileCdfSelector::CctxType, 1)]);
    let mut cdfs = tile_cdfs();
    let mut symbols = symbol_decoder_for_payload(&payload);

    let metadata = ensure_transform_tool_residual_handoff(
        &mut cdfs,
        &mut symbols,
        TransformToolResidualInput {
            frame_facts: facts,
            plane: 1,
            tx_size: TX_8X8,
            is_inter: false,
            lossless: true,
            fsc_mode: false,
            eob: 2,
            luma_transform_type_context: None,
            active_intra_ist_policy: ActiveIntraIstResidualPolicy::Reject,
            active_chroma_policy: ActiveChromaResidualPolicy::LrTxSkipRecordHandoff,
        },
    )
    .unwrap();

    assert_eq!(metadata.cctx_type, None);
    assert_eq!(symbols.symbol_count(), 0);
}

#[test]
fn dctonly_residual_safe_policy_rejects_chroma_non_dct_tx_set() {
    let result = ensure_with_test_state(
        frame_facts(false, false, false, false),
        1,
        TX_8X8,
        false,
        1,
        None,
    );

    assert_eq!(
        unsupported_reason(&result),
        Some("unsupported_dctonly_residual_tx_set")
    );
}

#[test]
fn dctonly_residual_lr_handoff_admits_chroma_non_dct_tx_set() {
    let result = ensure_with_test_payload_and_policy(
        frame_facts(false, false, false, false),
        1,
        TX_8X8,
        false,
        1,
        None,
        ActiveIntraIstResidualPolicy::Reject,
        ActiveChromaResidualPolicy::LrTxSkipRecordHandoff,
        &PAYLOAD,
    );

    assert!(result.is_ok());
}

#[test]
fn dctonly_residual_lr_handoff_admits_inter_chroma_non_dct_tx_set() {
    let result = ensure_with_test_payload_and_policy(
        frame_facts(false, false, false, false),
        2,
        TX_8X8,
        true,
        1,
        None,
        ActiveIntraIstResidualPolicy::Reject,
        ActiveChromaResidualPolicy::LrTxSkipRecordHandoff,
        &PAYLOAD,
    );

    assert!(result.is_ok());
}

#[test]
fn dctonly_residual_lr_handoff_reads_cctx_metadata() {
    for cctx_type in [0u8, 1] {
        let payload = encode_transform_symbols(&[(TileCdfSelector::CctxType, cctx_type)]);

        let metadata = ensure_with_test_payload_and_policy(
            frame_facts(false, false, false, true),
            1,
            TX_8X8,
            false,
            2,
            None,
            ActiveIntraIstResidualPolicy::Reject,
            ActiveChromaResidualPolicy::LrTxSkipRecordHandoff,
            &payload,
        )
        .unwrap();

        assert_eq!(metadata.cctx_type, Some(usize::from(cctx_type)));
    }
}

#[test]
fn dctonly_residual_maps_intra_tx_type_zero_to_dct_dct() {
    let tx_type = md_idx_luma_tx_type(TX_8X8, dc_luma_context(), 0).unwrap();

    assert_eq!(tx_type, DCT_DCT);
}

#[test]
fn dctonly_residual_long_set_maps_dct_symbol_only_for_long_side_dct() {
    assert_eq!(TX_TYPE_INV_LONG[1][0][0], DCT_DCT);
    assert_eq!(TX_TYPE_INV_LONG[1][1][0], DCT_DCT);
    assert_ne!(TX_TYPE_INV_LONG[0][0][0], DCT_DCT);
    assert_ne!(TX_TYPE_INV_LONG[0][1][0], DCT_DCT);
}

#[test]
fn fsc_idtx_block_reconstructs_without_tcq_dequant_shift() {
    let block = LumaCoeffBlock {
        all_zero: false,
        eob: 16,
        quant: vec![0, 0, 0, 3, 0, 0, 2, 9, 0, 0, 0, 6, 0, 0, 0, 6],
        intra_ist: None,
        plane_tx_type: IDTX,
        use_tcq: false,
        lossless: false,
    };
    let prediction = vec![
        38u8, 40, 42, 43, 40, 41, 42, 43, 41, 41, 42, 42, 42, 42, 42, 42,
    ];

    let fsc = reconstruct_general_intra_coeff_block_rect_with_prediction(
        &block,
        &prediction,
        78,
        PlaneId::Y,
        2,
        2,
        true,
        None,
        BitDepth::Eight,
    )
    .unwrap();

    assert_eq!(
        fsc,
        vec![
            38, 40, 42, 61, 40, 41, 54, 96, 41, 41, 42, 77, 42, 42, 42, 77
        ]
    );

    let mut ordinary_tcq = block;
    ordinary_tcq.use_tcq = true;
    let ordinary = reconstruct_general_intra_coeff_block_rect_with_prediction(
        &ordinary_tcq,
        &prediction,
        78,
        PlaneId::Y,
        2,
        2,
        true,
        None,
        BitDepth::Eight,
    )
    .unwrap();

    assert_eq!(
        ordinary,
        vec![
            38, 40, 42, 52, 40, 41, 48, 69, 41, 41, 42, 60, 42, 42, 42, 60
        ]
    );
}

#[test]
fn residual_scratch_reuse_leaks_nothing_between_consecutive_blocks() {
    let reconstruct_a = || {
        let mut quant = vec![0i32; 16];
        quant[0] = 37;
        quant[1] = -21;
        quant[5] = 9;
        let prediction = vec![301u16; 16];
        reconstruct_general_intra_block_rect_with_prediction(
            &quant,
            &prediction,
            80,
            PlaneId::Y,
            2,
            2,
            0,
            false,
            BitDepth::Ten,
        )
        .unwrap()
    };
    let reconstruct_b = || {
        let mut quant = vec![0i32; 64];
        quant[0] = -5;
        quant[9] = 61;
        let prediction = vec![144u16; 64];
        reconstruct_general_intra_block_rect_with_prediction(
            &quant,
            &prediction,
            96,
            PlaneId::U,
            3,
            3,
            0,
            false,
            BitDepth::Ten,
        )
        .unwrap()
    };

    let first_a = reconstruct_a();
    let b_after_a = reconstruct_b();
    let a_after_b = reconstruct_a();
    let (fresh_a, fresh_b) = std::thread::spawn(move || (reconstruct_a(), reconstruct_b()))
        .join()
        .unwrap();

    assert_eq!(first_a, fresh_a, "4x4 result depends on scratch history");
    assert_eq!(b_after_a, fresh_b, "8x8 result depends on scratch history");
    assert_eq!(a_after_b, fresh_a, "4x4 rerun depends on scratch history");
}

/// § 7.14.4 `segLvl` / `useQm` / `Qm_Offset` resolution: `tw > 8` selects the
/// `levels_gt8` set, `tw <= 8 && th <= 8` selects `levels_le8`, and the chroma
/// plane row is selected for U/V.
#[test]
fn resolve_block_qm_selects_level_plane_and_offset() {
    let _scope = FrameQmScope::install(Some(QmFrameLevels {
        levels_gt8: [8, 3, 4],
        levels_le8: {
            let mut levels = [[0u8; 3]; 16];
            levels[0] = [5, 2, 6];
            levels[3] = [9, 10, 11];
            levels
        },
    }));
    let luma = resolve_block_qm(PlaneId::Y, DCT_DCT, 32, 32, 5, 5).unwrap();
    assert_eq!(luma.seg_level, 8);
    assert!(!luma.plane_is_chroma);
    assert_eq!(
        luma.qm_offset,
        usize::try_from(QM_OFFSET[tx_size_index(5, 5).unwrap()]).unwrap()
    );
    let chroma_v = resolve_block_qm(PlaneId::V, DCT_DCT, 8, 8, 3, 3).unwrap();
    assert_eq!(chroma_v.seg_level, 6);
    assert!(chroma_v.plane_is_chroma);
    let _segment = FrameQmSegmentScope::install(3);
    let luma_segment = resolve_block_qm(PlaneId::Y, DCT_DCT, 8, 8, 3, 3).unwrap();
    assert_eq!(luma_segment.seg_level, 9);
}

/// `resolve_block_qm` is the flat path (`None`) with no installed scope, for
/// `PlaneTxType >= IDTX`, and for `segLvl >= NUM_CUSTOM_QMS`.
#[test]
fn resolve_block_qm_none_for_flat_paths() {
    assert!(resolve_block_qm(PlaneId::Y, DCT_DCT, 32, 32, 5, 5).is_none());
    let _scope = FrameQmScope::install(Some(QmFrameLevels {
        levels_gt8: [8, 8, 8],
        levels_le8: [[8, 8, 8]; 16],
    }));
    assert!(resolve_block_qm(PlaneId::Y, IDTX, 32, 32, 5, 5).is_none());
    let _flat = FrameQmScope::install(Some(QmFrameLevels {
        levels_gt8: [NUM_CUSTOM_QMS as u8, 0, 0],
        levels_le8: [[0, 0, 0]; 16],
    }));
    assert!(resolve_block_qm(PlaneId::Y, DCT_DCT, 32, 32, 5, 5).is_none());
}
