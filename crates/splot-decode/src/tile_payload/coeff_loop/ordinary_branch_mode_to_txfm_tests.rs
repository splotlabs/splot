// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::unwrap_used, clippy::panic)]

use splot_core::span::ByteOffset;
use splot_core::symbol::{CdfUpdateMode, SymbolBitPosition, SymbolDecoder, SymbolDecoderConfig};

use super::super::cdf::{FrameCdfSubset, TileCdfSubset};
use super::super::coeff_state::{CoeffContextUpdate, TileCoeffContextState};
use super::branch::NonZeroCoeffBlockStartInput;
use super::ordinary_pass::geometry::{
    CoeffOrdinaryBranchModeToTxfmBaseConfig, CoeffOrdinaryBranchModeToTxfmInput,
    CoeffOrdinaryBranchModeToTxfmNonZeroInput, CoeffOrdinaryBranchTxSizeDimensionsBaseConfig,
    CoeffOrdinaryBranchTxSizeDimensionsInput, CoeffOrdinaryBranchTxSizeDimensionsNonZeroInput,
    CoeffOrdinaryTxSizeGeometryConfig, apply_coeff_ordinary_branch_from_mode_to_txfm,
    apply_coeff_ordinary_branch_from_tx_size_dimensions,
};
use super::ordinary_pass::{CoeffOrdinaryBranch, CoeffOrdinaryBranchError};
use super::{AllZeroCoeffBlockInput, NonZeroCoeffEobContextInput};

const TX_8X8: usize = 3;
const TX_4X8: usize = 5;
const TX_SET_DCTONLY: usize = 0;
const TX_SET_INTRA_1: usize = 5;
const TX_SET_INTRA_2: usize = 6;
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
) -> CoeffOrdinaryBranchModeToTxfmBaseConfig {
    CoeffOrdinaryBranchModeToTxfmBaseConfig {
        tx_set,
        uv_mode,
        angle_delta_uv: 0,
        enable_chroma_dctonly: false,
        parity_hiding: false,
        use_tcq: false,
    }
}

fn mode_to_txfm_base_config_with_angle(
    uv_mode: usize,
    tx_set: usize,
    angle_delta_uv: i32,
) -> CoeffOrdinaryBranchModeToTxfmBaseConfig {
    CoeffOrdinaryBranchModeToTxfmBaseConfig {
        angle_delta_uv,
        ..mode_to_txfm_base_config(uv_mode, tx_set)
    }
}

fn mode_to_txfm_base_config_with_chroma_dctonly(
    uv_mode: usize,
    tx_set: usize,
) -> CoeffOrdinaryBranchModeToTxfmBaseConfig {
    CoeffOrdinaryBranchModeToTxfmBaseConfig {
        enable_chroma_dctonly: true,
        ..mode_to_txfm_base_config(uv_mode, tx_set)
    }
}

fn explicit_input(
    start: NonZeroCoeffBlockStartInput,
    plane_tx_type: usize,
) -> CoeffOrdinaryBranchTxSizeDimensionsInput {
    explicit_input_with_tx_size(start, TX_8X8, plane_tx_type)
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
            is_inter: false,
            base_config: explicit_base_config(plane_tx_type),
            lossless: false,
        },
    )
}

fn mode_to_txfm_input(
    start: NonZeroCoeffBlockStartInput,
    uv_mode: usize,
    tx_set: usize,
) -> CoeffOrdinaryBranchModeToTxfmInput {
    CoeffOrdinaryBranchModeToTxfmInput::NonZero(CoeffOrdinaryBranchModeToTxfmNonZeroInput {
        geometry: tx_size_geometry(start),
        coeff_cdf_q_ctx: 0,
        is_inter: false,
        base_config: mode_to_txfm_base_config(uv_mode, tx_set),
        lossless: false,
    })
}

fn mode_to_txfm_input_with_angle(
    start: NonZeroCoeffBlockStartInput,
    tx_size: usize,
    uv_mode: usize,
    tx_set: usize,
    angle_delta_uv: i32,
) -> CoeffOrdinaryBranchModeToTxfmInput {
    CoeffOrdinaryBranchModeToTxfmInput::NonZero(CoeffOrdinaryBranchModeToTxfmNonZeroInput {
        geometry: CoeffOrdinaryTxSizeGeometryConfig {
            tx_size,
            ..tx_size_geometry(start)
        },
        coeff_cdf_q_ctx: 0,
        is_inter: false,
        base_config: mode_to_txfm_base_config_with_angle(uv_mode, tx_set, angle_delta_uv),
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

fn find_payload_for_explicit(plane_tx_type: usize) -> [u8; 12] {
    find_payload_for_explicit_tx_size(TX_8X8, plane_tx_type)
}

fn find_payload_for_explicit_tx_size(tx_size: usize, plane_tx_type: usize) -> [u8; 12] {
    let start = start_input();
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

#[test]
fn coefficient_ordinary_branch_mode_to_txfm_accepts_mapped_transform() {
    // UV_SMOOTH_PRED maps through Mode_To_Txfm to ADST_ADST, and intra set 1
    // allows every TxType in AV2 §5.20.7.29.
    let start = start_input();
    let payload = find_payload_for_explicit(ADST_ADST);

    let explicit = run_explicit(&payload, explicit_input(start, ADST_ADST));
    let derived = run_mode_to_txfm(
        &payload,
        mode_to_txfm_input(start, UV_SMOOTH_PRED, TX_SET_INTRA_1),
    );

    assert_eq!(derived, explicit);
}

#[test]
fn coefficient_ordinary_branch_mode_to_txfm_uses_second_mapped_transform() {
    // UV_SMOOTH_V_PRED maps through Mode_To_Txfm to ADST_DCT, proving the
    // wrapper is not hardwired to DCT_DCT or ADST_ADST.
    let start = start_input();
    let payload = find_payload_for_explicit(ADST_DCT);

    let explicit = run_explicit(&payload, explicit_input(start, ADST_DCT));
    let derived = run_mode_to_txfm(
        &payload,
        mode_to_txfm_input(start, UV_SMOOTH_V_PRED, TX_SET_INTRA_1),
    );

    assert_eq!(derived, explicit);
}

#[test]
fn coefficient_ordinary_branch_mode_to_txfm_falls_back_to_dct() {
    // UV_SMOOTH_PRED maps to ADST_ADST, but TX_SET_DCTONLY rejects it, so
    // §5.20.7.29 falls back to DCT_DCT.
    let start = start_input();
    let payload = find_payload_for_explicit(DCT_DCT);

    let explicit = run_explicit(&payload, explicit_input(start, DCT_DCT));
    let derived = run_mode_to_txfm(
        &payload,
        mode_to_txfm_input(start, UV_SMOOTH_PRED, TX_SET_DCTONLY),
    );

    assert_eq!(derived, explicit);
}

#[test]
fn coefficient_ordinary_branch_mode_to_txfm_chroma_dctonly_short_circuits() {
    // AV2 §5.20.7.29 returns DCT_DCT before the Mode_To_Txfm lookup when
    // enable_chroma_dctonly is set, even though TX_SET_INTRA_2 accepts all
    // TxType values.
    let start = start_input();
    let payload = find_payload_for_explicit(DCT_DCT);
    let input =
        CoeffOrdinaryBranchModeToTxfmInput::NonZero(CoeffOrdinaryBranchModeToTxfmNonZeroInput {
            geometry: tx_size_geometry(start),
            coeff_cdf_q_ctx: 0,
            is_inter: false,
            base_config: mode_to_txfm_base_config_with_chroma_dctonly(
                UV_SMOOTH_PRED,
                TX_SET_INTRA_2,
            ),
            lossless: false,
        });

    let explicit = run_explicit(&payload, explicit_input(start, DCT_DCT));
    let derived = run_mode_to_txfm(&payload, input);

    assert_eq!(derived, explicit);
}

#[test]
fn coefficient_ordinary_branch_mode_to_txfm_maps_directional_uv_without_remap() {
    // Square transforms do not trigger AV2 §5.20.7.29 wide_angle_mapping, so
    // V_PRED maps directly through Mode_To_Txfm to ADST_DCT.
    let start = start_input();
    let payload = find_payload_for_explicit(ADST_DCT);

    let explicit = run_explicit(&payload, explicit_input(start, ADST_DCT));
    let derived = run_mode_to_txfm(
        &payload,
        mode_to_txfm_input(start, UV_V_PRED, TX_SET_INTRA_1),
    );

    assert_eq!(derived, explicit);
}

#[test]
fn coefficient_ordinary_branch_mode_to_txfm_maps_directional_uv_with_wide_angle_remap() {
    // TX_4X8 has h == 2*w. D45_PRED has pAngle 45, below
    // WAIP_WH_RATIO_2_THRES, so wide_angle_mapping remaps it to D203_PRED,
    // which maps through Mode_To_Txfm to DCT_ADST.
    let start = start_input();
    let payload = find_payload_for_explicit_tx_size(TX_4X8, DCT_ADST);

    let explicit = run_explicit(
        &payload,
        explicit_input_with_tx_size(start, TX_4X8, DCT_ADST),
    );
    let derived = run_mode_to_txfm(
        &payload,
        mode_to_txfm_input_with_angle(start, TX_4X8, UV_D45_PRED, TX_SET_INTRA_1, 0),
    );

    assert_eq!(derived, explicit);
}

#[test]
fn coefficient_ordinary_branch_mode_to_txfm_directional_uv_falls_back_to_dct() {
    let start = start_input();
    let payload = find_payload_for_explicit_tx_size(TX_4X8, DCT_DCT);

    let explicit = run_explicit(
        &payload,
        explicit_input_with_tx_size(start, TX_4X8, DCT_DCT),
    );
    let derived = run_mode_to_txfm(
        &payload,
        mode_to_txfm_input_with_angle(start, TX_4X8, UV_D45_PRED, TX_SET_DCTONLY, 0),
    );

    assert_eq!(derived, explicit);
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
    let mut luma = start;
    luma.block.plane = 0;
    luma.eob.plane = 0;
    let mut inter = start;
    inter.eob.is_inter = true;

    let cases = [
        (
            CoeffOrdinaryBranchModeToTxfmInput::NonZero(
                CoeffOrdinaryBranchModeToTxfmNonZeroInput {
                    geometry: tx_size_geometry(luma),
                    coeff_cdf_q_ctx: 0,
                    is_inter: false,
                    base_config: mode_to_txfm_base_config(UV_SMOOTH_PRED, TX_SET_INTRA_1),
                    lossless: false,
                },
            ),
            "luma",
        ),
        (
            CoeffOrdinaryBranchModeToTxfmInput::NonZero(
                CoeffOrdinaryBranchModeToTxfmNonZeroInput {
                    geometry: tx_size_geometry(inter),
                    coeff_cdf_q_ctx: 0,
                    is_inter: true,
                    base_config: mode_to_txfm_base_config(UV_SMOOTH_PRED, TX_SET_INTRA_1),
                    lossless: false,
                },
            ),
            "inter",
        ),
        (
            CoeffOrdinaryBranchModeToTxfmInput::NonZero(
                CoeffOrdinaryBranchModeToTxfmNonZeroInput {
                    geometry: tx_size_geometry(start),
                    coeff_cdf_q_ctx: 0,
                    is_inter: false,
                    base_config: mode_to_txfm_base_config(UV_SMOOTH_PRED, TX_SET_INTRA_1),
                    lossless: true,
                },
            ),
            "lossless",
        ),
    ];

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
