// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::unwrap_used, clippy::panic)]

use super::super::cdf::FrameCdfSubset;
use super::ordinary_pass::CoeffOrdinaryBranchError;
use super::ordinary_pass::geometry::{
    CoeffOrdinaryBranchLosslessBaseConfig, CoeffOrdinaryBranchLosslessInput,
    CoeffOrdinaryBranchLosslessNonZeroInput, CoeffOrdinaryBranchTxSetBaseConfig,
    CoeffOrdinaryBranchTxSetInput, CoeffOrdinaryBranchTxSetNonZeroInput,
    CoeffOrdinaryBranchTxSizeDimensionsBaseConfig, CoeffOrdinaryBranchTxSizeDimensionsInput,
    CoeffOrdinaryBranchTxSizeDimensionsNonZeroInput, CoeffOrdinaryTxSizeGeometryConfig,
    apply_coeff_ordinary_branch_from_lossless, apply_coeff_ordinary_branch_from_tx_set,
    apply_coeff_ordinary_branch_from_tx_size_dimensions,
};
use super::test_support::{
    OrdinaryBranchRun, run_ordinary_branch, seeded_context_state, symbol_decoder,
};

const TX_8X8: usize = 1;
const UV_SMOOTH_PRED: usize = 9;
const DCT_DCT: usize = 0;
const ADST_ADST: usize = 3;
const PAYLOAD_SUFFIXES: [[u8; 3]; 4] = [
    [0x00, 0x00, 0x80],
    [0xff, 0x00, 0x80],
    [0x55, 0xaa, 0x80],
    [0xff, 0xff, 0x80],
];

fn payload_from(first: u8, second: u8, suffix: [u8; 3]) -> [u8; 12] {
    [
        first, second, suffix[0], suffix[1], suffix[2], 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x80,
    ]
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
        chroma_inter_tx_type: 0,
        parity_hiding: false,
        use_tcq: false,
    }
}

fn lossless_base_config(
    uv_mode: usize,
    reduced_tx_set: usize,
    enable_chroma_dctonly: bool,
) -> CoeffOrdinaryBranchLosslessBaseConfig {
    lossless_base_config_with_entropy_flags(
        uv_mode,
        reduced_tx_set,
        enable_chroma_dctonly,
        false,
        false,
    )
}

fn lossless_base_config_with_entropy_flags(
    uv_mode: usize,
    reduced_tx_set: usize,
    enable_chroma_dctonly: bool,
    parity_hiding: bool,
    use_tcq: bool,
) -> CoeffOrdinaryBranchLosslessBaseConfig {
    CoeffOrdinaryBranchLosslessBaseConfig {
        reduced_tx_set,
        enable_chroma_dctonly,
        uv_mode,
        angle_delta_uv: 0,
        luma_tx_type: 0,
        chroma_inter_tx_type: 0,
        parity_hiding,
        use_tcq,
    }
}

fn explicit_input(
    tx_size: usize,
    plane_tx_type: usize,
    lossless: bool,
) -> CoeffOrdinaryBranchTxSizeDimensionsInput {
    explicit_input_with_inter(tx_size, plane_tx_type, lossless, false)
}

fn explicit_input_with_inter(
    tx_size: usize,
    plane_tx_type: usize,
    lossless: bool,
    is_inter: bool,
) -> CoeffOrdinaryBranchTxSizeDimensionsInput {
    CoeffOrdinaryBranchTxSizeDimensionsInput::NonZero(
        CoeffOrdinaryBranchTxSizeDimensionsNonZeroInput {
            geometry: tx_size_geometry(tx_size),
            coeff_cdf_q_ctx: 0,
            is_inter,
            base_config: explicit_base_config(plane_tx_type),
            lossless,
        },
    )
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

fn lossless_input(
    tx_size: usize,
    uv_mode: usize,
    reduced_tx_set: usize,
    enable_chroma_dctonly: bool,
    lossless: bool,
) -> CoeffOrdinaryBranchLosslessInput {
    CoeffOrdinaryBranchLosslessInput::NonZero(CoeffOrdinaryBranchLosslessNonZeroInput {
        geometry: tx_size_geometry(tx_size),
        coeff_cdf_q_ctx: 0,
        is_inter: false,
        base_config: lossless_base_config(uv_mode, reduced_tx_set, enable_chroma_dctonly),
        lossless,
    })
}

fn lossless_input_with_entropy_flags(
    tx_size: usize,
    uv_mode: usize,
    reduced_tx_set: usize,
    enable_chroma_dctonly: bool,
    parity_hiding: bool,
    use_tcq: bool,
) -> CoeffOrdinaryBranchLosslessInput {
    CoeffOrdinaryBranchLosslessInput::NonZero(CoeffOrdinaryBranchLosslessNonZeroInput {
        geometry: tx_size_geometry(tx_size),
        coeff_cdf_q_ctx: 0,
        is_inter: false,
        base_config: lossless_base_config_with_entropy_flags(
            uv_mode,
            reduced_tx_set,
            enable_chroma_dctonly,
            parity_hiding,
            use_tcq,
        ),
        lossless: true,
    })
}

fn lossless_inter_input(tx_size: usize) -> CoeffOrdinaryBranchLosslessInput {
    CoeffOrdinaryBranchLosslessInput::NonZero(CoeffOrdinaryBranchLosslessNonZeroInput {
        geometry: tx_size_geometry(tx_size),
        coeff_cdf_q_ctx: 0,
        is_inter: true,
        base_config: lossless_base_config(UV_SMOOTH_PRED, 0, false),
        lossless: true,
    })
}

fn run_explicit(
    payload: &[u8],
    input: CoeffOrdinaryBranchTxSizeDimensionsInput,
) -> OrdinaryBranchRun {
    run_ordinary_branch(payload, |context_state, tile, symbols| {
        apply_coeff_ordinary_branch_from_tx_size_dimensions(context_state, tile, symbols, input)
            .unwrap()
    })
}

fn run_tx_set(payload: &[u8], input: CoeffOrdinaryBranchTxSetInput) -> OrdinaryBranchRun {
    run_ordinary_branch(payload, |context_state, tile, symbols| {
        apply_coeff_ordinary_branch_from_tx_set(context_state, tile, symbols, input).unwrap()
    })
}

fn run_lossless(payload: &[u8], input: CoeffOrdinaryBranchLosslessInput) -> OrdinaryBranchRun {
    run_ordinary_branch(payload, |context_state, tile, symbols| {
        apply_coeff_ordinary_branch_from_lossless(context_state, tile, symbols, input).unwrap()
    })
}

fn find_payload_for_explicit(
    tx_size: usize,
    plane_tx_type: usize,
    lossless: bool,
    is_inter: bool,
) -> [u8; 12] {
    for first in u8::MIN..=u8::MAX {
        for second in u8::MIN..=u8::MAX {
            for suffix in PAYLOAD_SUFFIXES {
                let payload = payload_from(first, second, suffix);
                let result = std::panic::catch_unwind(|| {
                    run_explicit(
                        &payload,
                        explicit_input_with_inter(tx_size, plane_tx_type, lossless, is_inter),
                    );
                });
                if result.is_ok() {
                    return payload;
                }
            }
        }
    }
    panic!("no ordinary coefficient payload found");
}

fn assert_lossless_matches_dct_dct(input: CoeffOrdinaryBranchLosslessInput) {
    let payload = find_payload_for_explicit(TX_8X8, DCT_DCT, true, false);
    assert_eq!(
        run_lossless(&payload, input),
        run_explicit(&payload, explicit_input(TX_8X8, DCT_DCT, true))
    );
}

#[test]
fn coefficient_ordinary_branch_lossless_selects_dct_dct() {
    assert_lossless_matches_dct_dct(lossless_input(TX_8X8, UV_SMOOTH_PRED, 0, false, true));
}

#[test]
fn coefficient_ordinary_branch_lossless_bypasses_lower_validation() {
    assert_lossless_matches_dct_dct(lossless_input(TX_8X8, 99, 4, false, true));
}

#[test]
fn coefficient_ordinary_branch_lossless_disables_non_lossless_entropy_flags() {
    assert_lossless_matches_dct_dct(lossless_input_with_entropy_flags(
        TX_8X8,
        UV_SMOOTH_PRED,
        0,
        false,
        true,
        true,
    ));
}

#[test]
fn coefficient_ordinary_branch_lossless_delegates_non_lossless_to_tx_set() {
    let payload = find_payload_for_explicit(TX_8X8, ADST_ADST, false, false);

    let explicit = run_tx_set(&payload, tx_set_input(TX_8X8, UV_SMOOTH_PRED, 0, false));
    let derived = run_lossless(
        &payload,
        lossless_input(TX_8X8, UV_SMOOTH_PRED, 0, false, false),
    );

    assert_eq!(derived, explicit);
}

fn assert_lossless_error_preserves_state<F>(
    input: CoeffOrdinaryBranchLosslessInput,
    assert_error: F,
) where
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

    let err = apply_coeff_ordinary_branch_from_lossless(
        &mut context_state,
        &mut tile,
        &mut symbols,
        input,
    )
    .unwrap_err();

    assert_error(err);
    assert_eq!(context_state, context_before);
    assert_eq!(tile, tile_before);
    assert_eq!(symbols.consumed_bits(), consumed_before);
    assert_eq!(symbols.symbol_count(), symbols_before);
}

#[test]
fn coefficient_ordinary_branch_lossless_rejects_invalid_tx_size_atomically() {
    assert_lossless_error_preserves_state(
        lossless_input(25, UV_SMOOTH_PRED, 0, false, true),
        |err| {
            assert!(matches!(
                err,
                CoeffOrdinaryBranchError::InvalidTransformSize { tx_size: 25 }
            ));
        },
    );
}

#[test]
fn coefficient_ordinary_branch_lossless_inter_selects_dct_dct() {
    let payload = find_payload_for_explicit(TX_8X8, DCT_DCT, true, true);

    assert_eq!(
        run_lossless(&payload, lossless_inter_input(TX_8X8)),
        run_explicit(
            &payload,
            explicit_input_with_inter(TX_8X8, DCT_DCT, true, true)
        )
    );
}

#[test]
fn coefficient_ordinary_branch_lossless_all_zero_preserves_tx_set_branch() {
    let geometry = tx_size_geometry(TX_8X8);

    let explicit = run_tx_set(&[0x80], CoeffOrdinaryBranchTxSetInput::AllZero(geometry));
    let derived = run_lossless(&[0x80], CoeffOrdinaryBranchLosslessInput::AllZero(geometry));

    assert_eq!(derived, explicit);
}
