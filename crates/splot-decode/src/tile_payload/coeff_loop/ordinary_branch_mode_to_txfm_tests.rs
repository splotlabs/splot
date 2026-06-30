// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::unwrap_used, clippy::panic)]

use super::super::cdf::FrameCdfSubset;
use super::branch::NonZeroCoeffBlockStartInput;
use super::ordinary_pass::CoeffOrdinaryBranchError;
use super::ordinary_pass::geometry::{
    CoeffOrdinaryBranchModeToTxfmBaseConfig, CoeffOrdinaryBranchModeToTxfmInput,
    CoeffOrdinaryBranchModeToTxfmNonZeroInput, CoeffOrdinaryBranchTxSizeDimensionsBaseConfig,
    CoeffOrdinaryBranchTxSizeDimensionsInput, CoeffOrdinaryBranchTxSizeDimensionsNonZeroInput,
    CoeffOrdinaryTxSizeGeometryConfig, apply_coeff_ordinary_branch_from_mode_to_txfm,
    apply_coeff_ordinary_branch_from_tx_size_dimensions,
};
use super::test_support::{
    OrdinaryBranchRun, run_ordinary_branch, seeded_context_state, symbol_decoder,
};
use super::{AllZeroCoeffBlockInput, NonZeroCoeffEobContextInput};

const TX_8X8: usize = 3;
const TX_4X8: usize = 5;
const TX_SET_DCTONLY: usize = 0;
const TX_SET_INTRA_1: usize = 5;
const TX_SET_INTRA_2: usize = 6;
const TX_SET_INTER_1: usize = 5;
const TX_SET_DCT_IDTX: usize = 7;
const UV_SMOOTH_PRED: usize = 9;
const UV_SMOOTH_V_PRED: usize = 10;
const UV_V_PRED: usize = 1;
const UV_D45_PRED: usize = 3;
const DCT_DCT: usize = 0;
const ADST_ADST: usize = 3;
const ADST_DCT: usize = 1;
const DCT_ADST: usize = 2;
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

fn start_input() -> NonZeroCoeffBlockStartInput {
    NonZeroCoeffBlockStartInput {
        block: AllZeroCoeffBlockInput {
            plane: 1,
            x4: 1,
            y4: 1,
            w4: 2,
            h4: 2,
        },
        eob: NonZeroCoeffEobContextInput {
            plane: 1,
            is_inter: false,
            tx_width_log2: 3,
            tx_height_log2: 3,
            coeff_cdf_q_ctx: 0,
        },
    }
}

fn tx_size_geometry(start: NonZeroCoeffBlockStartInput) -> CoeffOrdinaryTxSizeGeometryConfig {
    CoeffOrdinaryTxSizeGeometryConfig {
        plane: start.block.plane,
        start_x: start.block.x4 << 2,
        start_y: start.block.y4 << 2,
        tx_size: TX_8X8,
    }
}

fn mode_to_txfm_base_config(
    uv_mode: usize,
    tx_set: usize,
) -> CoeffOrdinaryBranchModeToTxfmBaseConfig {
    CoeffOrdinaryBranchModeToTxfmBaseConfig {
        tx_set,
        uv_mode,
        angle_delta_uv: 0,
        luma_tx_type: DCT_DCT,
        chroma_inter_tx_type: DCT_DCT,
        enable_chroma_dctonly: false,
        parity_hiding: false,
        use_tcq: false,
    }
}

fn mode_to_txfm_nonzero_input(
    start: NonZeroCoeffBlockStartInput,
    tx_size: usize,
    is_inter: bool,
    base_config: CoeffOrdinaryBranchModeToTxfmBaseConfig,
) -> CoeffOrdinaryBranchModeToTxfmInput {
    CoeffOrdinaryBranchModeToTxfmInput::NonZero(CoeffOrdinaryBranchModeToTxfmNonZeroInput {
        geometry: CoeffOrdinaryTxSizeGeometryConfig {
            tx_size,
            ..tx_size_geometry(start)
        },
        coeff_cdf_q_ctx: 0,
        is_inter,
        base_config,
        lossless: false,
    })
}

fn explicit_input_with_tx_size(
    start: NonZeroCoeffBlockStartInput,
    tx_size: usize,
    plane_tx_type: usize,
) -> CoeffOrdinaryBranchTxSizeDimensionsInput {
    CoeffOrdinaryBranchTxSizeDimensionsInput::NonZero(
        CoeffOrdinaryBranchTxSizeDimensionsNonZeroInput {
            geometry: CoeffOrdinaryTxSizeGeometryConfig {
                tx_size,
                ..tx_size_geometry(start)
            },
            coeff_cdf_q_ctx: 0,
            is_inter: start.eob.is_inter,
            base_config: CoeffOrdinaryBranchTxSizeDimensionsBaseConfig {
                plane_tx_type,
                parity_hiding: false,
                use_tcq: false,
            },
            lossless: false,
        },
    )
}

fn mode_to_txfm_input(
    start: NonZeroCoeffBlockStartInput,
    uv_mode: usize,
    tx_set: usize,
) -> CoeffOrdinaryBranchModeToTxfmInput {
    mode_to_txfm_nonzero_input(
        start,
        TX_8X8,
        false,
        mode_to_txfm_base_config(uv_mode, tx_set),
    )
}

fn mode_to_txfm_input_with_angle(
    start: NonZeroCoeffBlockStartInput,
    tx_size: usize,
    uv_mode: usize,
    tx_set: usize,
    angle_delta_uv: i32,
) -> CoeffOrdinaryBranchModeToTxfmInput {
    mode_to_txfm_nonzero_input(
        start,
        tx_size,
        false,
        CoeffOrdinaryBranchModeToTxfmBaseConfig {
            angle_delta_uv,
            ..mode_to_txfm_base_config(uv_mode, tx_set)
        },
    )
}

fn mode_to_txfm_luma_input(
    start: NonZeroCoeffBlockStartInput,
    luma_tx_type: usize,
    enable_chroma_dctonly: bool,
) -> CoeffOrdinaryBranchModeToTxfmInput {
    mode_to_txfm_nonzero_input(
        start,
        TX_8X8,
        start.eob.is_inter,
        CoeffOrdinaryBranchModeToTxfmBaseConfig {
            luma_tx_type,
            enable_chroma_dctonly,
            ..mode_to_txfm_base_config(UV_SMOOTH_PRED, TX_SET_DCTONLY)
        },
    )
}

fn luma_start_input(is_inter: bool) -> NonZeroCoeffBlockStartInput {
    let mut start = start_input();
    start.block.plane = 0;
    start.eob.plane = 0;
    start.eob.is_inter = is_inter;
    start
}

fn mode_to_txfm_chroma_inter_input(
    start: NonZeroCoeffBlockStartInput,
    tx_set: usize,
    chroma_inter_tx_type: usize,
) -> CoeffOrdinaryBranchModeToTxfmInput {
    mode_to_txfm_nonzero_input(
        start,
        TX_8X8,
        true,
        CoeffOrdinaryBranchModeToTxfmBaseConfig {
            chroma_inter_tx_type,
            ..mode_to_txfm_base_config(UV_SMOOTH_PRED, tx_set)
        },
    )
}

fn mode_to_txfm_chroma_dctonly_input(
    start: NonZeroCoeffBlockStartInput,
    uv_mode: usize,
    tx_set: usize,
) -> CoeffOrdinaryBranchModeToTxfmInput {
    mode_to_txfm_nonzero_input(
        start,
        TX_8X8,
        start.eob.is_inter,
        CoeffOrdinaryBranchModeToTxfmBaseConfig {
            enable_chroma_dctonly: true,
            ..mode_to_txfm_base_config(uv_mode, tx_set)
        },
    )
}

fn chroma_inter_start_input() -> NonZeroCoeffBlockStartInput {
    let mut start = start_input();
    start.eob.is_inter = true;
    start
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

fn run_mode_to_txfm(
    payload: &[u8],
    input: CoeffOrdinaryBranchModeToTxfmInput,
) -> OrdinaryBranchRun {
    run_ordinary_branch(payload, |context_state, tile, symbols| {
        apply_coeff_ordinary_branch_from_mode_to_txfm(context_state, tile, symbols, input).unwrap()
    })
}

fn find_payload_for_explicit_start(
    start: NonZeroCoeffBlockStartInput,
    tx_size: usize,
    plane_tx_type: usize,
) -> [u8; 12] {
    for first in u8::MIN..=u8::MAX {
        for second in u8::MIN..=u8::MAX {
            for suffix in PAYLOAD_SUFFIXES {
                let payload = payload_from(first, second, suffix);
                let result = std::panic::catch_unwind(|| {
                    run_explicit(
                        &payload,
                        explicit_input_with_tx_size(start, tx_size, plane_tx_type),
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

fn assert_mode_to_txfm_matches_explicit(
    start: NonZeroCoeffBlockStartInput,
    tx_size: usize,
    plane_tx_type: usize,
    input: CoeffOrdinaryBranchModeToTxfmInput,
) {
    let payload = find_payload_for_explicit_start(start, tx_size, plane_tx_type);
    let explicit = run_explicit(
        &payload,
        explicit_input_with_tx_size(start, tx_size, plane_tx_type),
    );
    let derived = run_mode_to_txfm(&payload, input);

    assert_eq!(derived, explicit);
}

#[test]
fn coefficient_ordinary_branch_mode_to_txfm_accepts_mapped_transform() {
    let start = start_input();
    assert_mode_to_txfm_matches_explicit(
        start,
        TX_8X8,
        ADST_ADST,
        mode_to_txfm_input(start, UV_SMOOTH_PRED, TX_SET_INTRA_1),
    );
}

#[test]
fn coefficient_ordinary_branch_mode_to_txfm_uses_second_mapped_transform() {
    let start = start_input();
    assert_mode_to_txfm_matches_explicit(
        start,
        TX_8X8,
        ADST_DCT,
        mode_to_txfm_input(start, UV_SMOOTH_V_PRED, TX_SET_INTRA_1),
    );
}

#[test]
fn coefficient_ordinary_branch_mode_to_txfm_falls_back_to_dct() {
    let start = start_input();
    assert_mode_to_txfm_matches_explicit(
        start,
        TX_8X8,
        DCT_DCT,
        mode_to_txfm_input(start, UV_SMOOTH_PRED, TX_SET_DCTONLY),
    );
}

#[test]
fn coefficient_ordinary_branch_mode_to_txfm_chroma_dctonly_short_circuits() {
    let start = start_input();
    assert_mode_to_txfm_matches_explicit(
        start,
        TX_8X8,
        DCT_DCT,
        mode_to_txfm_chroma_dctonly_input(start, UV_SMOOTH_PRED, TX_SET_INTRA_2),
    );
}

#[test]
fn coefficient_ordinary_branch_mode_to_txfm_maps_directional_uv_without_remap() {
    let start = start_input();
    assert_mode_to_txfm_matches_explicit(
        start,
        TX_8X8,
        ADST_DCT,
        mode_to_txfm_input(start, UV_V_PRED, TX_SET_INTRA_1),
    );
}

#[test]
fn coefficient_ordinary_branch_mode_to_txfm_maps_directional_uv_with_wide_angle_remap() {
    let start = start_input();
    assert_mode_to_txfm_matches_explicit(
        start,
        TX_4X8,
        DCT_ADST,
        mode_to_txfm_input_with_angle(start, TX_4X8, UV_D45_PRED, TX_SET_INTRA_1, 0),
    );
}

#[test]
fn coefficient_ordinary_branch_mode_to_txfm_directional_uv_falls_back_to_dct() {
    let start = start_input();
    assert_mode_to_txfm_matches_explicit(
        start,
        TX_4X8,
        DCT_DCT,
        mode_to_txfm_input_with_angle(start, TX_4X8, UV_D45_PRED, TX_SET_DCTONLY, 0),
    );
}

#[test]
fn coefficient_ordinary_branch_mode_to_txfm_maps_luma_txtypes() {
    let start = luma_start_input(false);
    assert_mode_to_txfm_matches_explicit(
        start,
        TX_8X8,
        ADST_DCT,
        mode_to_txfm_luma_input(start, ADST_DCT, false),
    );
}

#[test]
fn coefficient_ordinary_branch_mode_to_txfm_luma_ignores_chroma_dctonly() {
    let start = luma_start_input(false);
    assert_mode_to_txfm_matches_explicit(
        start,
        TX_8X8,
        DCT_ADST,
        mode_to_txfm_luma_input(start, DCT_ADST, true),
    );
}

#[test]
fn coefficient_ordinary_branch_mode_to_txfm_luma_inter_uses_txtypes() {
    let start = luma_start_input(true);
    assert_mode_to_txfm_matches_explicit(
        start,
        TX_8X8,
        ADST_DCT,
        mode_to_txfm_luma_input(start, ADST_DCT, false),
    );
}

#[test]
fn coefficient_ordinary_branch_mode_to_txfm_chroma_inter_dctonly_short_circuits() {
    let start = chroma_inter_start_input();
    assert_mode_to_txfm_matches_explicit(
        start,
        TX_8X8,
        DCT_DCT,
        mode_to_txfm_chroma_dctonly_input(start, UV_SMOOTH_PRED, TX_SET_DCTONLY),
    );
}

#[test]
fn coefficient_ordinary_branch_mode_to_txfm_chroma_inter_txtypes_maps_when_in_set() {
    let start = chroma_inter_start_input();
    assert_mode_to_txfm_matches_explicit(
        start,
        TX_8X8,
        ADST_DCT,
        mode_to_txfm_chroma_inter_input(start, TX_SET_INTER_1, ADST_DCT),
    );
}

#[test]
fn coefficient_ordinary_branch_mode_to_txfm_chroma_inter_txtypes_falls_back_to_dct() {
    let start = chroma_inter_start_input();
    assert_mode_to_txfm_matches_explicit(
        start,
        TX_8X8,
        DCT_DCT,
        mode_to_txfm_chroma_inter_input(start, TX_SET_DCT_IDTX, ADST_DCT),
    );
}

fn assert_mode_to_txfm_error_preserves_state<F>(
    input: CoeffOrdinaryBranchModeToTxfmInput,
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

    let err = apply_coeff_ordinary_branch_from_mode_to_txfm(
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
fn coefficient_ordinary_branch_mode_to_txfm_rejects_unsupported_subset_atomically() {
    let start = start_input();

    let cases = [(
        CoeffOrdinaryBranchModeToTxfmInput::NonZero(CoeffOrdinaryBranchModeToTxfmNonZeroInput {
            geometry: tx_size_geometry(start),
            coeff_cdf_q_ctx: 0,
            is_inter: false,
            base_config: mode_to_txfm_base_config(UV_SMOOTH_PRED, TX_SET_INTRA_1),
            lossless: true,
        }),
        "lossless",
    )];

    for (input, reason) in cases {
        assert_mode_to_txfm_error_preserves_state(input, |err| {
            assert!(matches!(
                err,
                CoeffOrdinaryBranchError::UnsupportedModeToTxfmSubset { reason: got }
                    if got == reason
            ));
        });
    }
}

#[test]
fn coefficient_ordinary_branch_mode_to_txfm_rejects_luma_domains_atomically() {
    let start = luma_start_input(false);

    assert_mode_to_txfm_error_preserves_state(mode_to_txfm_luma_input(start, 16, false), |err| {
        assert!(matches!(
            err,
            CoeffOrdinaryBranchError::InvalidLumaTxType { tx_type: 16 }
        ));
    });
}

#[test]
fn coefficient_ordinary_branch_mode_to_txfm_rejects_chroma_inter_tx_type_domains_atomically() {
    let start = chroma_inter_start_input();

    assert_mode_to_txfm_error_preserves_state(
        mode_to_txfm_chroma_inter_input(start, TX_SET_INTER_1, 16),
        |err| {
            assert!(matches!(
                err,
                CoeffOrdinaryBranchError::InvalidChromaInterTxType { tx_type: 16 }
            ));
        },
    );
}

#[test]
fn coefficient_ordinary_branch_mode_to_txfm_rejects_chroma_inter_tx_set_domains_atomically() {
    let start = chroma_inter_start_input();

    assert_mode_to_txfm_error_preserves_state(
        mode_to_txfm_chroma_inter_input(start, 9, DCT_DCT),
        |err| {
            assert!(matches!(
                err,
                CoeffOrdinaryBranchError::InvalidInterTransformSet { tx_set: 9 }
            ));
        },
    );
}

#[test]
fn coefficient_ordinary_branch_mode_to_txfm_rejects_directional_domains_atomically() {
    let start = start_input();

    assert_mode_to_txfm_error_preserves_state(
        mode_to_txfm_input_with_angle(start, 25, UV_D45_PRED, TX_SET_INTRA_1, 0),
        |err| {
            assert!(matches!(
                err,
                CoeffOrdinaryBranchError::InvalidTransformSize { tx_size: 25 }
            ));
        },
    );
    assert_mode_to_txfm_error_preserves_state(
        mode_to_txfm_input_with_angle(start, TX_4X8, UV_D45_PRED, TX_SET_INTRA_1, i32::MAX),
        |err| {
            assert!(matches!(
                err,
                CoeffOrdinaryBranchError::DirectionalAngleOverflow {
                    uv_mode: UV_D45_PRED,
                    angle_delta_uv: i32::MAX
                }
            ));
        },
    );
}

#[test]
fn coefficient_ordinary_branch_mode_to_txfm_rejects_domains_atomically() {
    let start = start_input();

    assert_mode_to_txfm_error_preserves_state(
        mode_to_txfm_input(start, 14, TX_SET_INTRA_1),
        |err| {
            assert!(matches!(
                err,
                CoeffOrdinaryBranchError::InvalidUvMode { uv_mode: 14 }
            ));
        },
    );
    assert_mode_to_txfm_error_preserves_state(
        mode_to_txfm_input(start, UV_SMOOTH_PRED, 7),
        |err| {
            assert!(matches!(
                err,
                CoeffOrdinaryBranchError::InvalidIntraTransformSet { tx_set: 7 }
            ));
        },
    );
}

#[test]
fn coefficient_ordinary_branch_mode_to_txfm_all_zero_preserves_direct_branch() {
    let block = AllZeroCoeffBlockInput {
        plane: 1,
        x4: 1,
        y4: 1,
        w4: 2,
        h4: 2,
    };
    let geometry = CoeffOrdinaryTxSizeGeometryConfig {
        plane: block.plane,
        start_x: block.x4 << 2,
        start_y: block.y4 << 2,
        tx_size: TX_8X8,
    };

    let explicit = run_explicit(
        &[0x80],
        CoeffOrdinaryBranchTxSizeDimensionsInput::AllZero(geometry),
    );
    let derived = run_mode_to_txfm(
        &[0x80],
        CoeffOrdinaryBranchModeToTxfmInput::AllZero(geometry),
    );

    assert_eq!(derived, explicit);
}
