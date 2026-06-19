// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::unwrap_used, clippy::panic)]

use splot_core::span::ByteOffset;
use splot_core::symbol::{CdfUpdateMode, SymbolBitPosition, SymbolDecoder, SymbolDecoderConfig};

use super::super::cdf::{FrameCdfSubset, TileCdfSubset};
use super::super::coeff_state::{CoeffContextUpdate, TileCoeffContextState};
use super::ordinary_pass::geometry::{
    CoeffOrdinaryBranchModeToTxfmBaseConfig, CoeffOrdinaryBranchModeToTxfmInput,
    CoeffOrdinaryBranchModeToTxfmNonZeroInput, CoeffOrdinaryBranchTxSetBaseConfig,
    CoeffOrdinaryBranchTxSetInput, CoeffOrdinaryBranchTxSetNonZeroInput,
    CoeffOrdinaryBranchTxSizeDimensionsBaseConfig, CoeffOrdinaryBranchTxSizeDimensionsInput,
    CoeffOrdinaryBranchTxSizeDimensionsNonZeroInput, CoeffOrdinaryTxSizeGeometryConfig,
    apply_coeff_ordinary_branch_from_mode_to_txfm, apply_coeff_ordinary_branch_from_tx_set,
    apply_coeff_ordinary_branch_from_tx_size_dimensions,
};
use super::ordinary_pass::{CoeffOrdinaryBranch, CoeffOrdinaryBranchError};

const TX_8X8: usize = 1;
const TX_32X32: usize = 3;
const TX_SET_DCTONLY: usize = 0;
const TX_SET_INTRA_1: usize = 5;
const TX_SET_INTRA_2: usize = 6;
const UV_SMOOTH_PRED: usize = 9;
const DCT_DCT: usize = 0;
const ADST_ADST: usize = 3;
const PAYLOAD_SUFFIXES: [[u8; 3]; 4] = [
    [0x00, 0x00, 0x80],
    [0xff, 0x00, 0x80],
    [0x55, 0xaa, 0x80],
    [0xff, 0xff, 0x80],
];

fn symbol_decoder(payload: &[u8]) -> SymbolDecoder<'_> {
    SymbolDecoder::with_base_and_config(
        payload,
        ByteOffset::new(0),
        SymbolDecoderConfig::new().with_cdf_update_mode(CdfUpdateMode::Enabled),
    )
    .unwrap()
}

fn payload_from(first: u8, second: u8, suffix: [u8; 3]) -> [u8; 12] {
    [
        first, second, suffix[0], suffix[1], suffix[2], 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x80,
    ]
}

fn seeded_context_state() -> TileCoeffContextState {
    let mut state = TileCoeffContextState::new(32, 32).unwrap();
    state
        .update_after_coeffs(CoeffContextUpdate {
            plane: 0,
            x4: 0,
            y4: 0,
            w4: 6,
            h4: 6,
            cul_level: 1,
            dc_category: 1,
        })
        .unwrap();
    state
}

fn tx_size_geometry(tx_size: usize) -> CoeffOrdinaryTxSizeGeometryConfig {
    CoeffOrdinaryTxSizeGeometryConfig {
        plane: 1,
        start_x: 4,
        start_y: 4,
        tx_size,
    }
}

fn explicit_base_config(plane_tx_type: usize) -> CoeffOrdinaryBranchTxSizeDimensionsBaseConfig {
    CoeffOrdinaryBranchTxSizeDimensionsBaseConfig {
        plane_tx_type,
        parity_hiding: false,
        use_tcq: false,
    }
}

fn mode_to_txfm_base_config(
    uv_mode: usize,
    tx_set: usize,
    enable_chroma_dctonly: bool,
) -> CoeffOrdinaryBranchModeToTxfmBaseConfig {
    CoeffOrdinaryBranchModeToTxfmBaseConfig {
        tx_set,
        uv_mode,
        angle_delta_uv: 0,
        luma_tx_type: 0,
        enable_chroma_dctonly,
        parity_hiding: false,
        use_tcq: false,
    }
}

fn tx_set_base_config(
    uv_mode: usize,
    reduced_tx_set: usize,
    enable_chroma_dctonly: bool,
) -> CoeffOrdinaryBranchTxSetBaseConfig {
    CoeffOrdinaryBranchTxSetBaseConfig {
        reduced_tx_set,
        enable_chroma_dctonly,
        uv_mode,
        angle_delta_uv: 0,
        luma_tx_type: 0,
        parity_hiding: false,
        use_tcq: false,
    }
}

fn explicit_input(
    tx_size: usize,
    plane_tx_type: usize,
) -> CoeffOrdinaryBranchTxSizeDimensionsInput {
    CoeffOrdinaryBranchTxSizeDimensionsInput::NonZero(
        CoeffOrdinaryBranchTxSizeDimensionsNonZeroInput {
            geometry: tx_size_geometry(tx_size),
            coeff_cdf_q_ctx: 0,
            is_inter: false,
            base_config: explicit_base_config(plane_tx_type),
            lossless: false,
        },
    )
}

fn mode_to_txfm_input(
    tx_size: usize,
    uv_mode: usize,
    tx_set: usize,
    enable_chroma_dctonly: bool,
) -> CoeffOrdinaryBranchModeToTxfmInput {
    CoeffOrdinaryBranchModeToTxfmInput::NonZero(CoeffOrdinaryBranchModeToTxfmNonZeroInput {
        geometry: tx_size_geometry(tx_size),
        coeff_cdf_q_ctx: 0,
        is_inter: false,
        base_config: mode_to_txfm_base_config(uv_mode, tx_set, enable_chroma_dctonly),
        lossless: false,
    })
}

fn tx_set_input(
    tx_size: usize,
    uv_mode: usize,
    reduced_tx_set: usize,
    enable_chroma_dctonly: bool,
) -> CoeffOrdinaryBranchTxSetInput {
    CoeffOrdinaryBranchTxSetInput::NonZero(CoeffOrdinaryBranchTxSetNonZeroInput {
        geometry: tx_size_geometry(tx_size),
        coeff_cdf_q_ctx: 0,
        is_inter: false,
        base_config: tx_set_base_config(uv_mode, reduced_tx_set, enable_chroma_dctonly),
        lossless: false,
    })
}

fn run_explicit(
    payload: &[u8],
    input: CoeffOrdinaryBranchTxSizeDimensionsInput,
) -> (
    CoeffOrdinaryBranch,
    TileCoeffContextState,
    TileCdfSubset,
    SymbolBitPosition,
    u64,
) {
    let frame = FrameCdfSubset::from_defaults();
    let mut tile = frame.tile_copy();
    let mut symbols = symbol_decoder(payload);
    let mut context_state = seeded_context_state();
    let branch = apply_coeff_ordinary_branch_from_tx_size_dimensions(
        &mut context_state,
        &mut tile,
        &mut symbols,
        input,
    )
    .unwrap();
    (
        branch,
        context_state,
        tile,
        symbols.consumed_bits(),
        symbols.symbol_count(),
    )
}

fn run_mode_to_txfm(
    payload: &[u8],
    input: CoeffOrdinaryBranchModeToTxfmInput,
) -> (
    CoeffOrdinaryBranch,
    TileCoeffContextState,
    TileCdfSubset,
    SymbolBitPosition,
    u64,
) {
    let frame = FrameCdfSubset::from_defaults();
    let mut tile = frame.tile_copy();
    let mut symbols = symbol_decoder(payload);
    let mut context_state = seeded_context_state();
    let branch = apply_coeff_ordinary_branch_from_mode_to_txfm(
        &mut context_state,
        &mut tile,
        &mut symbols,
        input,
    )
    .unwrap();
    (
        branch,
        context_state,
        tile,
        symbols.consumed_bits(),
        symbols.symbol_count(),
    )
}

fn run_tx_set(
    payload: &[u8],
    input: CoeffOrdinaryBranchTxSetInput,
) -> (
    CoeffOrdinaryBranch,
    TileCoeffContextState,
    TileCdfSubset,
    SymbolBitPosition,
    u64,
) {
    let frame = FrameCdfSubset::from_defaults();
    let mut tile = frame.tile_copy();
    let mut symbols = symbol_decoder(payload);
    let mut context_state = seeded_context_state();
    let branch =
        apply_coeff_ordinary_branch_from_tx_set(&mut context_state, &mut tile, &mut symbols, input)
            .unwrap();
    (
        branch,
        context_state,
        tile,
        symbols.consumed_bits(),
        symbols.symbol_count(),
    )
}

fn find_payload_for_explicit(tx_size: usize, plane_tx_type: usize) -> [u8; 12] {
    for first in u8::MIN..=u8::MAX {
        for second in u8::MIN..=u8::MAX {
            for suffix in PAYLOAD_SUFFIXES {
                let payload = payload_from(first, second, suffix);
                let result = std::panic::catch_unwind(|| {
                    run_explicit(&payload, explicit_input(tx_size, plane_tx_type));
                });
                if result.is_ok() {
                    return payload;
                }
            }
        }
    }
    panic!("no ordinary coefficient payload found");
}

#[test]
fn coefficient_ordinary_branch_tx_set_derives_default_intra_set() {
    let payload = find_payload_for_explicit(TX_8X8, ADST_ADST);

    let explicit = run_mode_to_txfm(
        &payload,
        mode_to_txfm_input(TX_8X8, UV_SMOOTH_PRED, TX_SET_INTRA_1, false),
    );
    let derived = run_tx_set(&payload, tx_set_input(TX_8X8, UV_SMOOTH_PRED, 0, false));

    assert_eq!(derived, explicit);
}

#[test]
fn coefficient_ordinary_branch_tx_set_derives_reduced_chroma_set() {
    let payload = find_payload_for_explicit(TX_8X8, DCT_DCT);

    let explicit = run_mode_to_txfm(
        &payload,
        mode_to_txfm_input(TX_8X8, UV_SMOOTH_PRED, TX_SET_INTRA_2, true),
    );
    let derived = run_tx_set(&payload, tx_set_input(TX_8X8, UV_SMOOTH_PRED, 0, true));

    assert_eq!(derived, explicit);
}

#[test]
fn coefficient_ordinary_branch_tx_set_derives_large_dctonly_set() {
    let payload = find_payload_for_explicit(TX_32X32, DCT_DCT);

    let explicit = run_mode_to_txfm(
        &payload,
        mode_to_txfm_input(TX_32X32, UV_SMOOTH_PRED, TX_SET_DCTONLY, false),
    );
    let derived = run_tx_set(&payload, tx_set_input(TX_32X32, UV_SMOOTH_PRED, 0, false));

    assert_eq!(derived, explicit);
}

fn assert_tx_set_error_preserves_state<F>(input: CoeffOrdinaryBranchTxSetInput, assert_error: F)
where
    F: FnOnce(CoeffOrdinaryBranchError),
{
    let frame = FrameCdfSubset::from_defaults();
    let mut tile = frame.tile_copy();
    let tile_before = tile.clone();
    let mut symbols = symbol_decoder(&[0x80]);
    let consumed_before = symbols.consumed_bits();
    let symbols_before = symbols.symbol_count();
    let mut context_state = seeded_context_state();
    let context_before = context_state.clone();

    let err =
        apply_coeff_ordinary_branch_from_tx_set(&mut context_state, &mut tile, &mut symbols, input)
            .unwrap_err();

    assert_error(err);
    assert_eq!(context_state, context_before);
    assert_eq!(tile, tile_before);
    assert_eq!(symbols.consumed_bits(), consumed_before);
    assert_eq!(symbols.symbol_count(), symbols_before);
}

#[test]
fn coefficient_ordinary_branch_tx_set_rejects_invalid_reduced_set_atomically() {
    assert_tx_set_error_preserves_state(tx_set_input(TX_8X8, UV_SMOOTH_PRED, 4, false), |err| {
        assert!(matches!(
            err,
            CoeffOrdinaryBranchError::InvalidReducedTxSet { reduced_tx_set: 4 }
        ));
    });
}

#[test]
fn coefficient_ordinary_branch_tx_set_rejects_invalid_tx_size_atomically() {
    assert_tx_set_error_preserves_state(tx_set_input(25, UV_SMOOTH_PRED, 0, false), |err| {
        assert!(matches!(
            err,
            CoeffOrdinaryBranchError::InvalidTransformSize { tx_size: 25 }
        ));
    });
}

#[test]
fn coefficient_ordinary_branch_tx_set_all_zero_preserves_direct_branch() {
    let geometry = tx_size_geometry(TX_8X8);

    let explicit = run_mode_to_txfm(
        &[0x80],
        CoeffOrdinaryBranchModeToTxfmInput::AllZero(geometry),
    );
    let derived = run_tx_set(&[0x80], CoeffOrdinaryBranchTxSetInput::AllZero(geometry));

    assert_eq!(derived, explicit);
}
