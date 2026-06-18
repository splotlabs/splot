// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::unwrap_used, clippy::panic)]

use splot_core::span::ByteOffset;
use splot_core::symbol::{CdfUpdateMode, SymbolBitPosition, SymbolDecoder, SymbolDecoderConfig};

use super::super::cdf::{FrameCdfSubset, TileCdfSubset};
use super::super::coeff_state::{CoeffContextUpdate, TileCoeffContextState};
use super::base_level_pass::CoeffBaseDerivedLevelPassConfig;
use super::branch::{CoeffBlockEobBranch, NonZeroCoeffBlockStart, NonZeroCoeffBlockStartInput};
use super::max_level::CoeffTransformClass;
use super::ordinary_pass::geometry::{
    CoeffOrdinaryBranchCoeffsGeometryInput, CoeffOrdinaryBranchCoeffsGeometryNonZeroInput,
    CoeffOrdinaryBranchGeometryInput, CoeffOrdinaryBranchGeometryNonZeroInput,
    CoeffOrdinaryCoeffsGeometryConfig, CoeffOrdinaryGeometryStateContextConfig,
    apply_coeff_ordinary_branch_from_coeffs_geometry, apply_coeff_ordinary_branch_from_geometry,
};
use super::ordinary_pass::{
    CoeffOrdinaryBranch, CoeffOrdinaryBranchInput, CoeffOrdinaryBranchPlaneTxTypeBaseConfig,
    CoeffOrdinaryStateContextConfig, CoeffOrdinaryStateContextPassInput,
    NonZeroCoeffOrdinaryDerivedBasePass, apply_coeff_ordinary_branch,
    apply_nonzero_coeff_ordinary_pass_with_state_context,
};
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

fn symbol_decoder(payload: &[u8]) -> SymbolDecoder<'_> {
    SymbolDecoder::with_base_and_config(
        payload,
        ByteOffset::new(0),
        SymbolDecoderConfig::new().with_cdf_update_mode(CdfUpdateMode::Enabled),
    )
    .unwrap()
}

fn seeded_context_state() -> TileCoeffContextState {
    let mut state = TileCoeffContextState::new(6, 6).unwrap();
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

fn payload_from(first: u8, second: u8, suffix: [u8; 3]) -> [u8; 12] {
    [
        first, second, suffix[0], suffix[1], suffix[2], 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x80,
    ]
}

fn base_config_for_plane(plane: usize) -> CoeffBaseDerivedLevelPassConfig {
    CoeffBaseDerivedLevelPassConfig {
        coeff_cdf_q_ctx: 0,
        tx_size_ctx: 0,
        tx_width_log2: 3,
        tx_width: 8,
        tx_height: 8,
        plane,
        tx_class: CoeffTransformClass::TwoD,
        parity_hiding: false,
        use_tcq: false,
    }
}

fn plane_tx_type_base_config_for_plane(plane: usize) -> CoeffOrdinaryBranchPlaneTxTypeBaseConfig {
    CoeffOrdinaryBranchPlaneTxTypeBaseConfig {
        coeff_cdf_q_ctx: 0,
        tx_size_ctx: 0,
        tx_width_log2: 3,
        tx_width: 8,
        tx_height: 8,
        plane,
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

fn nonzero_start_input_for_plane_and_geometry(
    plane: usize,
    x4: usize,
    y4: usize,
    w4: usize,
    h4: usize,
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
            tx_width_log2: 3,
            tx_height_log2: 3,
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
