// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Unit tests for the general intra luma transform-block coefficient decode
//! (`super`), split out to keep the parser source under the §5 hard cap.

#![allow(clippy::unwrap_used)]

use splot_core::segment::MAX_SEGMENTS;
use splot_core::span::ByteOffset;
use splot_core::symbol::{CdfUpdateMode, Symbol, SymbolDecoderConfig};
use splot_core::symbol_encoder::{SymbolEncoder, SymbolEncoderConfig};

use super::*;
use crate::tile_payload::TileCoeffFrameFactsInput;

const PAYLOAD: [u8; 2] = [0x00, 0x80];

fn symbol_decoder_for_payload(payload: &'static [u8]) -> SymbolDecoder<'static> {
    SymbolDecoder::with_base_and_config(
        payload,
        ByteOffset::new(0),
        SymbolDecoderConfig::new().with_cdf_update_mode(CdfUpdateMode::Disabled),
    )
    .unwrap()
}

fn tile_cdfs() -> TileCdfSubset {
    crate::tile_payload::FrameCdfSubset::from_defaults().tile_copy()
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

#[test]
fn reconstruct_with_prediction_rejects_wrong_prediction_length() {
    // A 4x4 block needs 16 prediction samples; a short buffer is rejected
    // with a structured error (no panic) before reconstruction.
    let quant = vec![0i32; 16];
    let prediction = vec![128u8; 8];
    let result = reconstruct_general_intra_block_with_prediction(
        &quant,
        &prediction,
        64,
        PlaneId::Y,
        2,
        // DCT_DCT (§3 PlaneTxType 0); rejected on length before the transform resolves.
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
    // txSzCtx = (Tx_Size_Sqr[txSz] + Tx_Size_Sqr_Up[txSz] + 1) >> 1.
    // TX_4X4 (0): (0 + 0 + 1) >> 1 == 0.
    assert_eq!(txb_skip_tx_size_ctx(0), 0);
    // TX_8X8 (1): (1 + 1 + 1) >> 1 == 1.
    assert_eq!(txb_skip_tx_size_ctx(1), 1);
    // TX_16X16 (2): (2 + 2 + 1) >> 1 == 2.
    assert_eq!(txb_skip_tx_size_ctx(2), 2);
    // TX_32X32 (3): (3 + 3 + 1) >> 1 == 3.
    assert_eq!(txb_skip_tx_size_ctx(3), 3);
    // TX_64X64 (4): (4 + 4 + 1) >> 1 == 4 (the q80 single-block luma size).
    assert_eq!(txb_skip_tx_size_ctx(TX_64X64), 4);
}

#[test]
fn txb_skip_tx_size_ctx_is_total_for_out_of_range_tx_size() {
    // Out-of-range indices saturate to 0 rather than panicking.
    assert_eq!(txb_skip_tx_size_ctx(usize::MAX), 0);
    assert_eq!(txb_skip_tx_size_ctx(TX_SIZE_SQR.len()), 0);
}

// Each bool is a distinct AV2 frame-level syntax flag the fixture toggles; bundling them would obscure the spec mapping.
#[allow(clippy::fn_params_excessive_bools)]
fn frame_facts(
    enable_idtx_intra: bool,
    enable_intra_ist: bool,
    enable_chroma_dctonly: bool,
    enable_cctx: bool,
) -> TileCoeffFrameFacts {
    TileCoeffFrameFacts::new(TileCoeffFrameFactsInput {
        enable_fsc: false,
        enable_idtx_intra,
        enable_intra_ist,
        enable_inter_ist: false,
        enable_chroma_dctonly,
        enable_cctx,
        reduced_tx_set: 0,
        lossless_array: [false; MAX_SEGMENTS],
        allow_tcq: false,
        allow_parity_hiding: false,
        base_q_idx: 128,
    })
}

fn frame_facts_with_coeff_tools(allow_tcq: bool, allow_parity_hiding: bool) -> TileCoeffFrameFacts {
    TileCoeffFrameFacts::new(TileCoeffFrameFactsInput {
        enable_fsc: false,
        enable_idtx_intra: true,
        enable_intra_ist: false,
        enable_inter_ist: false,
        enable_chroma_dctonly: false,
        enable_cctx: false,
        reduced_tx_set: 0,
        lossless_array: [false; MAX_SEGMENTS],
        allow_tcq,
        allow_parity_hiding,
        base_q_idx: 128,
    })
}

fn frame_facts_with_fsc() -> TileCoeffFrameFacts {
    TileCoeffFrameFacts::new(TileCoeffFrameFactsInput {
        enable_fsc: true,
        enable_idtx_intra: true,
        enable_intra_ist: false,
        enable_inter_ist: false,
        enable_chroma_dctonly: false,
        enable_cctx: false,
        reduced_tx_set: 0,
        lossless_array: [false; MAX_SEGMENTS],
        allow_tcq: false,
        allow_parity_hiding: false,
        base_q_idx: 128,
    })
}

// Consumes a throwaway decode `Result` to extract its reason; by-value matches the call sites that hand off ownership.
#[allow(clippy::needless_pass_by_value)]
fn unsupported_reason<T>(result: Result<T, GeneralIntraResidualError>) -> Option<&'static str> {
    match result {
        Err(GeneralIntraResidualError::UnsupportedTransformToolResidual { reason }) => Some(reason),
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
    payload: &'static [u8],
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
    payload: &'static [u8],
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
            fsc_mode,
            eob,
            luma_transform_type_context: luma,
            active_intra_ist_policy,
            active_chroma_policy,
        },
    )
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
        unsupported_reason(result),
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
        Some(LumaTransformTypeContext::new(IntraYMode::DC_PRED, 0)),
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
        Some(LumaTransformTypeContext::new(IntraYMode::DC_PRED, 0)),
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
        unsupported_reason(result),
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
        unsupported_reason(result),
        Some("unsupported_dctonly_residual_intra_sec_tx_type")
    );
}

#[test]
fn dctonly_residual_lr_handoff_admits_active_intra_ist_metadata() {
    let payload = encode_transform_symbols(&[
        (
            TileCdfSelector::SecTxType {
                is_inter: 0,
                tx_size_sqr: TX_SIZE_SQR[TX_32X32] as usize,
            },
            1,
        ),
        (TileCdfSelector::MostProbableStxSet, 2),
    ]);
    let leaked_payload: &'static [u8] = Box::leak(payload.into_boxed_slice());

    let metadata = ensure_with_test_payload_and_policy(
        frame_facts(true, true, false, false),
        0,
        TX_32X32,
        false,
        2,
        Some(LumaTransformTypeContext::new(IntraYMode::DC_PRED, 0)),
        ActiveIntraIstResidualPolicy::LrTxSkipRecordHandoff,
        ActiveChromaResidualPolicy::Reject,
        leaked_payload,
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
    let payload = encode_transform_symbols(&[
        (
            TileCdfSelector::SecTxType {
                is_inter: 0,
                tx_size_sqr: TX_SIZE_SQR[TX_32X32] as usize,
            },
            1,
        ),
        (TileCdfSelector::MostProbableStxSet, 2),
    ]);
    let leaked_payload: &'static [u8] = Box::leak(payload.into_boxed_slice());

    let result = ensure_with_test_payload_and_policy(
        frame_facts(true, true, false, false),
        0,
        TX_32X32,
        false,
        2,
        Some(LumaTransformTypeContext::new(IntraYMode::DC_PRED, 0)),
        ActiveIntraIstResidualPolicy::Reject,
        ActiveChromaResidualPolicy::Reject,
        leaked_payload,
    );

    assert_eq!(
        unsupported_reason(result),
        Some("unsupported_dctonly_residual_intra_sec_tx_type")
    );
}

#[test]
fn dctonly_residual_maps_nonzero_intra_tx_type_to_non_dct() {
    let tx_type = md_idx_luma_tx_type(
        TX_8X8,
        LumaTransformTypeContext::new(IntraYMode::DC_PRED, 0),
        1,
    )
    .unwrap();

    assert_ne!(tx_type, DCT_DCT);
}

// The ac0ej3 inter tx-set block: TX_8X16 (Tx_Size_Sqr == 1), eob 10 →
// `inter_tx_type_long_ctx` == 0 for every small-set read.
const INTER_SET_TX_SIZE: usize = TX_8X16;
const INTER_SET_EOB: usize = 10;

fn read_inter_tx_type_from_symbols(tx_set: usize, sequence: &[(TileCdfSelector, u8)]) -> usize {
    let payload = encode_transform_symbols(sequence);
    let leaked: &'static [u8] = Box::leak(payload.into_boxed_slice());
    let mut cdfs = tile_cdfs();
    let mut symbols = symbol_decoder_for_payload(leaked);
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
fn inter_tx_set_ctx_for_ac0ej3_block_is_zero() {
    // TX_8X16, eob 10: bwl 3, eoby 1, eobx 1, diag 2, max_diag 20 → ctx 0.
    assert_eq!(
        inter_tx_type_long_ctx(INTER_SET_TX_SIZE, INTER_SET_EOB).unwrap(),
        0
    );
}

#[test]
fn inter_set1_index_branch_inverts_via_tx_type_inter_inv_set1() {
    // §5.20.8.2 TX_SET_INTER_1, inter_tx_type == 0: tx_type_idx == offset.
    // offset 7 → Tx_Type_Inter_Inv_Set1[7] == DCT_DCT.
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
    // §5.20.8.2 TX_SET_INTER_1, inter_tx_type == 1: tx_type_idx == 8 + offset.
    // offset 0 → Tx_Type_Inter_Inv_Set1[8] == ADST_DCT.
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
    // §5.20.8.2 TX_SET_INTER_2 (Set2 has no sqrSz index): inter_tx_type == 0,
    // offset 3 → Tx_Type_Inter_Inv_Set2[3] == DCT_DCT.
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
    // §5.20.8.2 TX_SET_INTER_2, inter_tx_type == 1: tx_type_idx == 8 + offset.
    // offset 0 → Tx_Type_Inter_Inv_Set2[8] == ADST_ADST.
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
    // §5.20.8.2 TX_SET_DCT_IDTX: Tx_Type_Inter_Inv_Set3 == {IDTX, DCT_DCT}.
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
    // §5.20.8.2 TX_SET_DCT_IDTX_IDDCT: Set4 == {DCT_DCT, V_DCT, H_DCT, IDTX}.
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
    // A transform set outside the eight §5.20.8.3 inter sets the dispatch
    // models defers fail-closed rather than decoding garbage. (`transform_set`
    // never produces such a value on the inter path; this pins the guard.)
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
        crate::tile_payload::cdf::block_context::reconstruct_y_mode_second_set_top_left(1, 6)
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
    let payload = encode_transform_symbols(&[(
        TileCdfSelector::IntraTxTypeSet1 {
            tx_size_sqr: TX_SIZE_SQR[TX_8X8] as usize,
        },
        1,
    )]);
    let leaked_payload: &'static [u8] = Box::leak(payload.into_boxed_slice());
    let luma = LumaTransformTypeContext::new(IntraYMode::DC_PRED, 0);
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
        leaked_payload,
    )
    .unwrap();

    assert_ne!(expected, DCT_DCT);
    assert_eq!(metadata.luma_tx_type, expected);
}

#[test]
fn luma_txtype_residual_lr_handoff_skips_intra_ist_for_non_sec_tx_type() {
    let payload = encode_transform_symbols(&[(
        TileCdfSelector::IntraTxTypeSet1 {
            tx_size_sqr: TX_SIZE_SQR[TX_8X8] as usize,
        },
        2,
    )]);
    let leaked_payload: &'static [u8] = Box::leak(payload.into_boxed_slice());
    let luma = LumaTransformTypeContext::new(IntraYMode::DC_PRED, 0);
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
        leaked_payload,
    )
    .unwrap();

    assert_ne!(expected, DCT_DCT);
    assert_ne!(expected, ADST_ADST);
    assert_eq!(metadata.luma_tx_type, expected);
    assert_eq!(metadata.intra_ist, None);
}

#[test]
fn luma_txtype_residual_adst_adst_uses_reduced_ist_eob_limit() {
    let payload = encode_transform_symbols(&[(
        TileCdfSelector::IntraTxTypeSet1 {
            tx_size_sqr: TX_SIZE_SQR[TX_16X16] as usize,
        },
        1,
    )]);
    let leaked_payload: &'static [u8] = Box::leak(payload.into_boxed_slice());
    let luma = LumaTransformTypeContext::new(IntraYMode::DC_PRED, 0);
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
        leaked_payload,
    )
    .unwrap();

    assert_eq!(expected, ADST_ADST);
    assert_eq!(metadata.luma_tx_type, ADST_ADST);
    assert_eq!(metadata.intra_ist, None);
}

#[test]
fn luma_txtype_residual_staged_base_config_uses_retained_luma_tx_type() {
    let luma = LumaTransformTypeContext::new(IntraYMode::DC_PRED, 0);
    let expected = md_idx_luma_tx_type(TX_8X8, luma, 1).unwrap();

    let config = staged_transform_tool_lossless_base_config(
        frame_facts(false, false, false, false),
        0,
        0,
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
fn luma_txtype_residual_staged_base_config_derives_flags_for_2d_luma_tx_type() {
    let config = staged_transform_tool_lossless_base_config(
        frame_facts_with_coeff_tools(true, true),
        0,
        0,
        false,
        TransformToolResidualMetadata {
            luma_tx_type: ADST_DCT,
            ..TransformToolResidualMetadata::default()
        },
    );

    assert!(config.parity_hiding);
    assert!(config.use_tcq);
}

#[test]
fn luma_txtype_residual_staged_base_config_suppresses_parity_hiding_for_idtx() {
    let config = staged_transform_tool_lossless_base_config(
        frame_facts_with_coeff_tools(true, true),
        0,
        0,
        false,
        TransformToolResidualMetadata {
            luma_tx_type: IDTX,
            ..TransformToolResidualMetadata::default()
        },
    );

    assert!(!config.parity_hiding);
    assert!(config.use_tcq);
}

#[test]
fn luma_txtype_residual_staged_base_config_suppresses_tcq_for_1d_luma_tx_type() {
    let config = staged_transform_tool_lossless_base_config(
        frame_facts_with_coeff_tools(true, true),
        0,
        0,
        false,
        TransformToolResidualMetadata {
            luma_tx_type: V_DCT,
            ..TransformToolResidualMetadata::default()
        },
    );

    assert!(config.parity_hiding);
    assert!(!config.use_tcq);
}

#[test]
fn luma_txtype_residual_safe_policy_rejects_non_dct_luma_tx_type() {
    let payload = encode_transform_symbols(&[(
        TileCdfSelector::IntraTxTypeSet1 {
            tx_size_sqr: TX_SIZE_SQR[TX_8X8] as usize,
        },
        1,
    )]);
    let leaked_payload: &'static [u8] = Box::leak(payload.into_boxed_slice());

    let result = ensure_with_test_payload_and_policy(
        frame_facts(false, false, false, false),
        0,
        TX_8X8,
        false,
        2,
        Some(LumaTransformTypeContext::new(IntraYMode::DC_PRED, 0)),
        ActiveIntraIstResidualPolicy::Reject,
        ActiveChromaResidualPolicy::Reject,
        leaked_payload,
    );

    assert_eq!(
        unsupported_reason(result),
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
        unsupported_reason(eob_two),
        Some("unsupported_dctonly_residual_cctx")
    );
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
        unsupported_reason(result),
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
fn dctonly_residual_lr_handoff_reads_cctx_zero() {
    let payload = encode_transform_symbols(&[(TileCdfSelector::CctxType, 0)]);
    let leaked_payload: &'static [u8] = Box::leak(payload.into_boxed_slice());

    let metadata = ensure_with_test_payload_and_policy(
        frame_facts(false, false, false, true),
        1,
        TX_8X8,
        false,
        2,
        None,
        ActiveIntraIstResidualPolicy::Reject,
        ActiveChromaResidualPolicy::LrTxSkipRecordHandoff,
        leaked_payload,
    )
    .unwrap();

    assert_eq!(metadata.cctx_type, Some(0));
}

#[test]
fn dctonly_residual_lr_handoff_reads_nonzero_cctx_metadata() {
    let payload = encode_transform_symbols(&[(TileCdfSelector::CctxType, 1)]);
    let leaked_payload: &'static [u8] = Box::leak(payload.into_boxed_slice());

    let metadata = ensure_with_test_payload_and_policy(
        frame_facts(false, false, false, true),
        1,
        TX_8X8,
        false,
        2,
        None,
        ActiveIntraIstResidualPolicy::Reject,
        ActiveChromaResidualPolicy::LrTxSkipRecordHandoff,
        leaked_payload,
    )
    .unwrap();

    assert_eq!(metadata.cctx_type, Some(1));
}

#[test]
fn dctonly_residual_maps_intra_tx_type_zero_to_dct_dct() {
    let tx_type = md_idx_luma_tx_type(
        TX_8X8,
        LumaTransformTypeContext::new(IntraYMode::DC_PRED, 0),
        0,
    )
    .unwrap();

    assert_eq!(tx_type, DCT_DCT);
}

#[test]
fn dctonly_residual_long_set_maps_dct_symbol_only_for_long_side_dct() {
    assert_eq!(TX_TYPE_INV_LONG[1][0][0], DCT_DCT);
    assert_eq!(TX_TYPE_INV_LONG[1][1][0], DCT_DCT);
    assert_ne!(TX_TYPE_INV_LONG[0][0][0], DCT_DCT);
    assert_ne!(TX_TYPE_INV_LONG[0][1][0], DCT_DCT);
}
