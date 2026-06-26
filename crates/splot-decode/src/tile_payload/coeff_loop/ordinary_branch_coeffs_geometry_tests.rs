// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::unwrap_used, clippy::panic)]

use splot_core::symbol::{SymbolBitPosition, SymbolDecoder};
use splot_core::tables::conversion::{ADJUSTED_TX_SIZE, TX_SIZE_SQR, TX_SIZE_SQR_UP};

use super::super::cdf::{FrameCdfSubset, TileCdfSubset};
use super::super::coeff_state::TileCoeffContextState;
use super::base_level_pass::{CoeffBaseDerivedLevelPassConfig, CoeffBaseDerivedLevelPassError};
use super::branch::{CoeffBlockEobBranch, NonZeroCoeffBlockStart, NonZeroCoeffBlockStartInput};
use super::max_level::CoeffTransformClass;
use super::ordinary_pass::geometry::{
    CoeffOrdinaryBranchCoeffsGeometryInput, CoeffOrdinaryBranchCoeffsGeometryNonZeroInput,
    CoeffOrdinaryBranchGeometryInput, CoeffOrdinaryBranchGeometryNonZeroInput,
    CoeffOrdinaryBranchTxSizeDimensionsBaseConfig, CoeffOrdinaryBranchTxSizeDimensionsInput,
    CoeffOrdinaryBranchTxSizeDimensionsNonZeroInput, CoeffOrdinaryCoeffsGeometryConfig,
    CoeffOrdinaryGeometryStateContextConfig, CoeffOrdinaryTestDimensionTables,
    CoeffOrdinaryTxSizeGeometryConfig, apply_coeff_ordinary_branch_from_coeffs_geometry,
    apply_coeff_ordinary_branch_from_geometry, apply_coeff_ordinary_branch_from_tx_size_dimensions,
    apply_coeff_ordinary_branch_from_tx_size_dimensions_with_test_dimension_tables,
    apply_coeff_ordinary_branch_from_tx_size_dimensions_with_test_tables, tx_size_scan_for_test,
};
use super::ordinary_pass::{
    CoeffOrdinaryBranch, CoeffOrdinaryBranchError, CoeffOrdinaryBranchInput,
    CoeffOrdinaryBranchPlaneTxTypeBaseConfig, CoeffOrdinaryPassError,
    CoeffOrdinaryStateContextConfig, CoeffOrdinaryStateContextPassInput,
    NonZeroCoeffOrdinaryDerivedBasePass, apply_coeff_ordinary_branch,
    apply_nonzero_coeff_ordinary_pass_with_state_context,
};
use super::test_support::{seeded_context_state, symbol_decoder};
use super::{
    AllZeroCoeffBlockInput, CoeffBlockEobBranchInput, NonZeroCoeffEobContextInput,
    read_coeff_block_eob_branch,
};

const DC_ONLY_SCAN: [u16; 1] = [0];
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

fn assert_scan_permutation(scan: &[u16]) {
    let mut seen = vec![false; scan.len()];
    for &pos in scan {
        let pos = usize::from(pos);
        assert!(pos < scan.len(), "scan position {pos} outside scan length");
        assert!(!seen[pos], "scan position {pos} repeated");
        seen[pos] = true;
    }
    assert!(seen.into_iter().all(|seen| seen));
}

fn base_config_for_plane(plane: usize) -> CoeffBaseDerivedLevelPassConfig {
    base_config_for_plane_and_dimensions(plane, 3, 8, 8)
}

fn base_config_for_plane_and_dimensions(
    plane: usize,
    tx_width_log2: u32,
    tx_width: usize,
    tx_height: usize,
) -> CoeffBaseDerivedLevelPassConfig {
    base_config_for_plane_context_and_dimensions(plane, 0, tx_width_log2, tx_width, tx_height)
}

fn base_config_for_plane_context_and_dimensions(
    plane: usize,
    tx_size_ctx: usize,
    tx_width_log2: u32,
    tx_width: usize,
    tx_height: usize,
) -> CoeffBaseDerivedLevelPassConfig {
    CoeffBaseDerivedLevelPassConfig {
        coeff_cdf_q_ctx: 0,
        tx_size_ctx,
        tx_width_log2,
        tx_width,
        tx_height,
        plane,
        tx_class: CoeffTransformClass::TwoD,
        parity_hiding: false,
        use_tcq: false,
    }
}

fn plane_tx_type_base_config_for_plane(plane: usize) -> CoeffOrdinaryBranchPlaneTxTypeBaseConfig {
    plane_tx_type_base_config_for_plane_and_dimensions(plane, 3, 8, 8)
}

fn plane_tx_type_base_config_for_plane_and_dimensions(
    plane: usize,
    tx_width_log2: u32,
    tx_width: usize,
    tx_height: usize,
) -> CoeffOrdinaryBranchPlaneTxTypeBaseConfig {
    plane_tx_type_base_config_for_plane_context_and_dimensions(
        plane,
        0,
        tx_width_log2,
        tx_width,
        tx_height,
    )
}

fn plane_tx_type_base_config_for_plane_context_and_dimensions(
    plane: usize,
    tx_size_ctx: usize,
    tx_width_log2: u32,
    tx_width: usize,
    tx_height: usize,
) -> CoeffOrdinaryBranchPlaneTxTypeBaseConfig {
    CoeffOrdinaryBranchPlaneTxTypeBaseConfig {
        coeff_cdf_q_ctx: 0,
        tx_size_ctx,
        tx_width_log2,
        tx_width,
        tx_height,
        plane,
        plane_tx_type: 0,
        parity_hiding: false,
        use_tcq: false,
    }
}

fn tx_size_base_config() -> CoeffOrdinaryBranchTxSizeDimensionsBaseConfig {
    CoeffOrdinaryBranchTxSizeDimensionsBaseConfig {
        plane_tx_type: 0,
        parity_hiding: false,
        use_tcq: false,
    }
}

fn geometry_state_context_config() -> CoeffOrdinaryGeometryStateContextConfig {
    CoeffOrdinaryGeometryStateContextConfig { coeff_cdf_q_ctx: 0 }
}

fn coeffs_geometry_config_for_block(
    block: AllZeroCoeffBlockInput,
) -> CoeffOrdinaryCoeffsGeometryConfig {
    CoeffOrdinaryCoeffsGeometryConfig {
        plane: block.plane,
        start_x: block.x4 << 2,
        start_y: block.y4 << 2,
        tx_width: block.w4 << 2,
        tx_height: block.h4 << 2,
    }
}

fn tx_size_geometry_config_for_block(
    block: AllZeroCoeffBlockInput,
    tx_size: usize,
) -> CoeffOrdinaryTxSizeGeometryConfig {
    CoeffOrdinaryTxSizeGeometryConfig {
        plane: block.plane,
        start_x: block.x4 << 2,
        start_y: block.y4 << 2,
        tx_size,
    }
}

fn nonzero_start_input_for_plane_and_geometry(
    plane: usize,
    x4: usize,
    y4: usize,
    w4: usize,
    h4: usize,
) -> NonZeroCoeffBlockStartInput {
    nonzero_start_input_for_plane_geometry_and_log2(plane, x4, y4, w4, h4, 3, 3)
}

fn nonzero_start_input_for_plane_geometry_and_log2(
    plane: usize,
    x4: usize,
    y4: usize,
    w4: usize,
    h4: usize,
    tx_width_log2: usize,
    tx_height_log2: usize,
) -> NonZeroCoeffBlockStartInput {
    NonZeroCoeffBlockStartInput {
        block: AllZeroCoeffBlockInput {
            plane,
            x4,
            y4,
            w4,
            h4,
        },
        eob: NonZeroCoeffEobContextInput {
            plane,
            is_inter: false,
            tx_width_log2,
            tx_height_log2,
            coeff_cdf_q_ctx: 0,
        },
    }
}

fn branch_nonzero(branch: CoeffBlockEobBranch) -> Option<NonZeroCoeffBlockStart> {
    match branch {
        CoeffBlockEobBranch::AllZero(_) => None,
        CoeffBlockEobBranch::NonZero(start) => Some(start),
    }
}

fn setup_start_with_input<'a>(
    payload: &'a [u8],
    start: NonZeroCoeffBlockStartInput,
) -> Option<(TileCdfSubset, SymbolDecoder<'a>, NonZeroCoeffBlockStart)> {
    let frame = FrameCdfSubset::from_defaults();
    let mut tile = frame.tile_copy();
    let mut symbols = symbol_decoder(payload);
    let mut state = TileCoeffContextState::new(4, 4).ok()?;
    let branch = read_coeff_block_eob_branch(
        &mut state,
        &mut tile,
        &mut symbols,
        CoeffBlockEobBranchInput::NonZero(start),
    )
    .ok()?;
    Some((tile, symbols, branch_nonzero(branch)?))
}

fn state_context_pass_for_payload_with_start(
    payload: &[u8],
    start_input: NonZeroCoeffBlockStartInput,
    base_config: CoeffBaseDerivedLevelPassConfig,
    state_context: CoeffOrdinaryStateContextConfig,
) -> Option<NonZeroCoeffOrdinaryDerivedBasePass> {
    let (mut tile, mut symbols, start) = setup_start_with_input(payload, start_input)?;
    let mut context_state = seeded_context_state();
    apply_nonzero_coeff_ordinary_pass_with_state_context(
        &mut context_state,
        &mut tile,
        &mut symbols,
        CoeffOrdinaryStateContextPassInput {
            start,
            scan: &DC_ONLY_SCAN,
            base_config,
            state_context,
            lossless: false,
        },
    )
    .ok()
}

fn find_state_context_payload_with_start(
    start_input: NonZeroCoeffBlockStartInput,
    base_config: CoeffBaseDerivedLevelPassConfig,
    state_context: CoeffOrdinaryStateContextConfig,
) -> [u8; 12] {
    for first in u8::MIN..=u8::MAX {
        for second in u8::MIN..=u8::MAX {
            for suffix in PAYLOAD_SUFFIXES {
                let payload = payload_from(first, second, suffix);
                if state_context_pass_for_payload_with_start(
                    &payload,
                    start_input,
                    base_config,
                    state_context,
                )
                .is_some()
                {
                    return payload;
                }
            }
        }
    }
    panic!("no state-context ordinary coefficient payload found");
}

fn branch_geometry_nonzero_input(
    start: NonZeroCoeffBlockStartInput,
    base_config: CoeffOrdinaryBranchPlaneTxTypeBaseConfig,
    state_context: CoeffOrdinaryGeometryStateContextConfig,
) -> CoeffOrdinaryBranchGeometryInput<'static> {
    CoeffOrdinaryBranchGeometryInput::NonZero(CoeffOrdinaryBranchGeometryNonZeroInput {
        start,
        scan: &DC_ONLY_SCAN,
        base_config,
        state_context,
        lossless: false,
    })
}

fn branch_coeffs_geometry_nonzero_input(
    start: NonZeroCoeffBlockStartInput,
    base_config: CoeffOrdinaryBranchPlaneTxTypeBaseConfig,
    state_context: CoeffOrdinaryGeometryStateContextConfig,
) -> CoeffOrdinaryBranchCoeffsGeometryInput<'static> {
    CoeffOrdinaryBranchCoeffsGeometryInput::NonZero(CoeffOrdinaryBranchCoeffsGeometryNonZeroInput {
        geometry: coeffs_geometry_config_for_block(start.block),
        eob: start.eob,
        scan: &DC_ONLY_SCAN,
        base_config,
        state_context,
        lossless: false,
    })
}

fn branch_tx_size_dimensions_nonzero_input(
    start: NonZeroCoeffBlockStartInput,
    tx_size: usize,
    base_config: CoeffOrdinaryBranchTxSizeDimensionsBaseConfig,
) -> CoeffOrdinaryBranchTxSizeDimensionsInput {
    CoeffOrdinaryBranchTxSizeDimensionsInput::NonZero(
        CoeffOrdinaryBranchTxSizeDimensionsNonZeroInput {
            geometry: tx_size_geometry_config_for_block(start.block, tx_size),
            coeff_cdf_q_ctx: 0,
            is_inter: start.eob.is_inter,
            base_config,
            lossless: false,
        },
    )
}

fn run_direct_branch(
    payload: &[u8],
    input: CoeffOrdinaryBranchInput<'_>,
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
        apply_coeff_ordinary_branch(&mut context_state, &mut tile, &mut symbols, input).unwrap();
    (
        branch,
        context_state,
        tile,
        symbols.consumed_bits(),
        symbols.symbol_count(),
    )
}

fn run_geometry_branch(
    payload: &[u8],
    input: CoeffOrdinaryBranchGeometryInput<'_>,
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
    let branch = apply_coeff_ordinary_branch_from_geometry(
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

fn run_coeffs_geometry_branch(
    payload: &[u8],
    input: CoeffOrdinaryBranchCoeffsGeometryInput<'_>,
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
    let branch = apply_coeff_ordinary_branch_from_coeffs_geometry(
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

fn run_tx_size_dimensions_branch(
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

#[test]
fn coefficient_ordinary_branch_coeffs_geometry_nonzero_matches_explicit_geometry() {
    let start = nonzero_start_input_for_plane_and_geometry(1, 1, 1, 2, 2);
    let direct_state_context = CoeffOrdinaryStateContextConfig {
        coeff_cdf_q_ctx: 0,
        plane_type: 1,
        x4: start.block.x4,
        y4: start.block.y4,
        w4: start.block.w4,
        h4: start.block.h4,
    };
    let payload = find_state_context_payload_with_start(
        start,
        base_config_for_plane(1),
        direct_state_context,
    );
    let derived_base_config = plane_tx_type_base_config_for_plane(1);

    let explicit = run_geometry_branch(
        &payload,
        branch_geometry_nonzero_input(start, derived_base_config, geometry_state_context_config()),
    );
    let derived = run_coeffs_geometry_branch(
        &payload,
        branch_coeffs_geometry_nonzero_input(
            start,
            derived_base_config,
            geometry_state_context_config(),
        ),
    );

    assert_eq!(derived, explicit);
}

#[test]
fn coefficient_ordinary_branch_tx_size_scan_order_derives_two_dimensional() {
    let scan = tx_size_scan_for_test(8, 4, 0).unwrap();

    assert_eq!(&scan[..10], &[0, 8, 1, 16, 9, 2, 24, 17, 10, 3]);
    assert_eq!(scan.len(), 32);
    assert_scan_permutation(&scan);
}

#[test]
fn coefficient_ordinary_branch_tx_size_scan_order_derives_horizontal() {
    let scan = tx_size_scan_for_test(4, 4, 11).unwrap();

    assert_eq!(
        scan.as_slice(),
        &[0, 4, 8, 12, 1, 5, 9, 13, 2, 6, 10, 14, 3, 7, 11, 15]
    );
}

#[test]
fn coefficient_ordinary_branch_tx_size_scan_order_derives_vertical() {
    let scan = tx_size_scan_for_test(4, 4, 10).unwrap();

    assert_eq!(
        scan.as_slice(),
        &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15]
    );
}

#[test]
fn coefficient_ordinary_branch_tx_size_dimensions_nonzero_matches_explicit_dimensions() {
    let start = nonzero_start_input_for_plane_and_geometry(1, 1, 1, 2, 2);
    let direct_state_context = CoeffOrdinaryStateContextConfig {
        coeff_cdf_q_ctx: 0,
        plane_type: 1,
        x4: start.block.x4,
        y4: start.block.y4,
        w4: start.block.w4,
        h4: start.block.h4,
    };
    let payload = find_state_context_payload_with_start(
        start,
        base_config_for_plane(1),
        direct_state_context,
    );
    let explicit_base_config = plane_tx_type_base_config_for_plane(1);

    let explicit = run_coeffs_geometry_branch(
        &payload,
        branch_coeffs_geometry_nonzero_input(
            start,
            explicit_base_config,
            geometry_state_context_config(),
        ),
    );
    let derived = run_tx_size_dimensions_branch(
        &payload,
        branch_tx_size_dimensions_nonzero_input(start, 1, tx_size_base_config()),
    );

    assert_eq!(derived, explicit);
}

#[test]
fn coefficient_ordinary_branch_tx_size_dimensions_uses_adjusted_base_dimensions() {
    // TX_64X32 has raw geometry 64x32 for § 5.20.7.27 block/EOB facts, but
    // Adjusted_Tx_Size[TX_64X32] is TX_32X32 for § 8.3.2 base contexts and
    // Tx_Size_Sqr/Up derive txSzCtx 4.
    let tx_64x32 = 12;
    let tx_size_ctx = 4;
    let start = nonzero_start_input_for_plane_geometry_and_log2(0, 0, 0, 16, 8, 6, 5);
    let raw_state_context = CoeffOrdinaryStateContextConfig {
        coeff_cdf_q_ctx: 0,
        plane_type: 0,
        x4: start.block.x4,
        y4: start.block.y4,
        w4: start.block.w4,
        h4: start.block.h4,
    };
    let adjusted_base_config =
        plane_tx_type_base_config_for_plane_context_and_dimensions(0, tx_size_ctx, 5, 32, 32);
    let payload = find_state_context_payload_with_start(
        start,
        base_config_for_plane_context_and_dimensions(0, tx_size_ctx, 5, 32, 32),
        raw_state_context,
    );

    let explicit = run_coeffs_geometry_branch(
        &payload,
        branch_coeffs_geometry_nonzero_input(
            start,
            adjusted_base_config,
            geometry_state_context_config(),
        ),
    );
    let derived = run_tx_size_dimensions_branch(
        &payload,
        branch_tx_size_dimensions_nonzero_input(start, tx_64x32, tx_size_base_config()),
    );

    assert_eq!(derived, explicit);

    let raw_base_config = plane_tx_type_base_config_for_plane_and_dimensions(0, 6, 64, 32);
    let frame = FrameCdfSubset::from_defaults();
    let mut tile = frame.tile_copy();
    let mut symbols = symbol_decoder(&payload);
    let mut context_state = seeded_context_state();
    let err = apply_coeff_ordinary_branch_from_coeffs_geometry(
        &mut context_state,
        &mut tile,
        &mut symbols,
        branch_coeffs_geometry_nonzero_input(
            start,
            raw_base_config,
            geometry_state_context_config(),
        ),
    )
    .unwrap_err();

    assert!(matches!(
        err,
        CoeffOrdinaryBranchError::Ordinary(CoeffOrdinaryPassError::BaseDerived(
            CoeffBaseDerivedLevelPassError::BlockGeometryMismatch {
                block_width: 32,
                block_height: 32,
                config_width: 64,
                config_height: 32,
            }
        ))
    ));
}

#[test]
fn coefficient_ordinary_branch_tx_size_dimensions_uses_derived_tx_size_context() {
    // TX_64X32 keeps raw 64x32 geometry while deriving txSzCtx from the square
    // transform-size tables, so context and dimensions differ in one path.
    let tx_64x32 = 12;
    let derived_tx_size_ctx = 4;
    let start = nonzero_start_input_for_plane_geometry_and_log2(0, 0, 0, 16, 8, 6, 5);
    let raw_state_context = CoeffOrdinaryStateContextConfig {
        coeff_cdf_q_ctx: 0,
        plane_type: 0,
        x4: start.block.x4,
        y4: start.block.y4,
        w4: start.block.w4,
        h4: start.block.h4,
    };
    let payload = find_state_context_payload_with_start(
        start,
        base_config_for_plane_context_and_dimensions(0, derived_tx_size_ctx, 5, 32, 32),
        raw_state_context,
    );

    let explicit = run_coeffs_geometry_branch(
        &payload,
        branch_coeffs_geometry_nonzero_input(
            start,
            plane_tx_type_base_config_for_plane_context_and_dimensions(
                0,
                derived_tx_size_ctx,
                5,
                32,
                32,
            ),
            geometry_state_context_config(),
        ),
    );
    let derived = run_tx_size_dimensions_branch(
        &payload,
        branch_tx_size_dimensions_nonzero_input(start, tx_64x32, tx_size_base_config()),
    );

    assert_eq!(derived, explicit);

    let stale_context_base_config =
        plane_tx_type_base_config_for_plane_context_and_dimensions(0, 3, 5, 32, 32);
    let stale_context = run_coeffs_geometry_branch(
        &payload,
        branch_coeffs_geometry_nonzero_input(
            start,
            stale_context_base_config,
            geometry_state_context_config(),
        ),
    );

    assert_ne!(stale_context, derived);
}

#[test]
fn coefficient_ordinary_branch_coeffs_geometry_all_zero_preserves_direct_branch() {
    let block = AllZeroCoeffBlockInput {
        plane: 1,
        x4: 1,
        y4: 1,
        w4: 2,
        h4: 2,
    };
    let direct = run_direct_branch(&[0x80], CoeffOrdinaryBranchInput::AllZero(block));
    let derived = run_coeffs_geometry_branch(
        &[0x80],
        CoeffOrdinaryBranchCoeffsGeometryInput::AllZero(coeffs_geometry_config_for_block(block)),
    );

    assert_eq!(derived, direct);
}

#[test]
fn coefficient_ordinary_branch_tx_size_dimensions_all_zero_preserves_direct_branch() {
    let block = AllZeroCoeffBlockInput {
        plane: 1,
        x4: 1,
        y4: 1,
        w4: 2,
        h4: 2,
    };
    let direct = run_direct_branch(&[0x80], CoeffOrdinaryBranchInput::AllZero(block));
    let derived = run_tx_size_dimensions_branch(
        &[0x80],
        CoeffOrdinaryBranchTxSizeDimensionsInput::AllZero(tx_size_geometry_config_for_block(
            block, 1,
        )),
    );

    assert_eq!(derived, direct);
}

#[test]
fn coefficient_ordinary_branch_tx_size_dimensions_invalid_tx_size_preserves_mutable_state() {
    let frame = FrameCdfSubset::from_defaults();
    let mut tile = frame.tile_copy();
    let tile_before = tile.clone();
    let mut symbols = symbol_decoder(&[0x80]);
    let consumed_before = symbols.consumed_bits();
    let symbols_before = symbols.symbol_count();
    let mut context_state = seeded_context_state();
    let context_before = context_state.clone();
    let start = nonzero_start_input_for_plane_and_geometry(1, 1, 1, 2, 2);

    let err = apply_coeff_ordinary_branch_from_tx_size_dimensions(
        &mut context_state,
        &mut tile,
        &mut symbols,
        branch_tx_size_dimensions_nonzero_input(start, usize::MAX, tx_size_base_config()),
    )
    .unwrap_err();

    assert!(matches!(
        err,
        CoeffOrdinaryBranchError::InvalidTransformSize {
            tx_size: usize::MAX
        }
    ));
    assert_eq!(context_state, context_before);
    assert_eq!(tile, tile_before);
    assert_eq!(symbols.consumed_bits(), consumed_before);
    assert_eq!(symbols.symbol_count(), symbols_before);
}

#[test]
fn coefficient_ordinary_branch_adjusted_tx_size_table_value_preserves_mutable_state() {
    let frame = FrameCdfSubset::from_defaults();
    let mut tile = frame.tile_copy();
    let tile_before = tile.clone();
    let mut symbols = symbol_decoder(&[0x80]);
    let consumed_before = symbols.consumed_bits();
    let symbols_before = symbols.symbol_count();
    let mut context_state = seeded_context_state();
    let context_before = context_state.clone();
    let start = nonzero_start_input_for_plane_and_geometry(1, 1, 1, 2, 2);
    let invalid_adjusted_tx_size_table = [-1];

    let err = apply_coeff_ordinary_branch_from_tx_size_dimensions_with_test_tables(
        &mut context_state,
        &mut tile,
        &mut symbols,
        branch_tx_size_dimensions_nonzero_input(start, 0, tx_size_base_config()),
        &invalid_adjusted_tx_size_table,
        &TX_SIZE_SQR,
        &TX_SIZE_SQR_UP,
    )
    .unwrap_err();

    assert!(matches!(
        err,
        CoeffOrdinaryBranchError::InvalidTransformSizeTableValue {
            table: "Adjusted_Tx_Size",
            tx_size: 0,
            value: -1,
        }
    ));
    assert_eq!(context_state, context_before);
    assert_eq!(tile, tile_before);
    assert_eq!(symbols.consumed_bits(), consumed_before);
    assert_eq!(symbols.symbol_count(), symbols_before);
}

#[test]
fn coefficient_ordinary_branch_tx_size_context_table_value_preserves_mutable_state() {
    fn assert_invalid_square_table_preserves_mutable_state<F>(
        tx_size_sqr_table: &[i32],
        tx_size_sqr_up_table: &[i32],
        assert_expected_error: F,
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
        let start = nonzero_start_input_for_plane_and_geometry(1, 1, 1, 2, 2);

        let err = apply_coeff_ordinary_branch_from_tx_size_dimensions_with_test_tables(
            &mut context_state,
            &mut tile,
            &mut symbols,
            branch_tx_size_dimensions_nonzero_input(start, 0, tx_size_base_config()),
            &ADJUSTED_TX_SIZE,
            tx_size_sqr_table,
            tx_size_sqr_up_table,
        )
        .unwrap_err();

        assert_expected_error(err);
        assert_eq!(context_state, context_before);
        assert_eq!(tile, tile_before);
        assert_eq!(symbols.consumed_bits(), consumed_before);
        assert_eq!(symbols.symbol_count(), symbols_before);
    }

    assert_invalid_square_table_preserves_mutable_state(&[-1], &TX_SIZE_SQR_UP, |err| {
        assert!(matches!(
            err,
            CoeffOrdinaryBranchError::InvalidTransformSizeTableValue {
                table: "Tx_Size_Sqr",
                tx_size: 0,
                value: -1,
            }
        ));
    });
    assert_invalid_square_table_preserves_mutable_state(&TX_SIZE_SQR, &[-1], |err| {
        assert!(matches!(
            err,
            CoeffOrdinaryBranchError::InvalidTransformSizeTableValue {
                table: "Tx_Size_Sqr_Up",
                tx_size: 0,
                value: -1,
            }
        ));
    });
    assert_invalid_square_table_preserves_mutable_state(&[25], &TX_SIZE_SQR_UP, |err| {
        assert!(matches!(
            err,
            CoeffOrdinaryBranchError::InvalidTransformSize { tx_size: 25 }
        ));
    });
}

#[test]
fn coefficient_ordinary_branch_tx_size_scan_shape_preserves_mutable_state() {
    let frame = FrameCdfSubset::from_defaults();
    let mut tile = frame.tile_copy();
    let tile_before = tile.clone();
    let mut symbols = symbol_decoder(&[0x80]);
    let consumed_before = symbols.consumed_bits();
    let symbols_before = symbols.symbol_count();
    let mut context_state = seeded_context_state();
    let context_before = context_state.clone();
    let start = nonzero_start_input_for_plane_and_geometry(1, 1, 1, 2, 2);

    let err = apply_coeff_ordinary_branch_from_tx_size_dimensions_with_test_dimension_tables(
        &mut context_state,
        &mut tile,
        &mut symbols,
        branch_tx_size_dimensions_nonzero_input(start, 0, tx_size_base_config()),
        CoeffOrdinaryTestDimensionTables {
            tx_width: &[2],
            tx_height: &[4],
            tx_width_log2: &[1],
            tx_height_log2: &[2],
        },
    )
    .unwrap_err();

    assert!(matches!(
        err,
        CoeffOrdinaryBranchError::InvalidScanShape {
            width: 2,
            height: 4
        }
    ));
    assert_eq!(context_state, context_before);
    assert_eq!(tile, tile_before);
    assert_eq!(symbols.consumed_bits(), consumed_before);
    assert_eq!(symbols.symbol_count(), symbols_before);
}
