// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::unwrap_used, clippy::panic)]

use splot_core::symbol::{SymbolBitPosition, SymbolDecoder};

use super::super::cdf::{FrameCdfSubset, TileCdfSubset};
use super::super::coeff_state::{CoeffContextUpdate, TileCoeffContextState, TileCoeffStateError};
use super::base_level_pass::{CoeffBaseDerivedLevelPassConfig, CoeffBaseDerivedLevelPassError};
use super::branch::{CoeffBlockEobBranch, NonZeroCoeffBlockStart, NonZeroCoeffBlockStartInput};
use super::max_level::CoeffTransformClass;
use super::ordinary_pass::geometry::{
    CoeffOrdinaryBranchGeometryInput, CoeffOrdinaryBranchGeometryNonZeroInput,
    CoeffOrdinaryGeometryStateContextConfig, apply_coeff_ordinary_branch_from_geometry,
};
use super::ordinary_pass::{
    CoeffOrdinaryBranch, CoeffOrdinaryBranchError, CoeffOrdinaryBranchInput,
    CoeffOrdinaryBranchNonZeroInput, CoeffOrdinaryBranchPlaneTxTypeBaseConfig,
    CoeffOrdinaryBranchPlaneTxTypeInput, CoeffOrdinaryBranchPlaneTxTypeNonZeroInput,
    CoeffOrdinaryBranchPlaneTypeInput, CoeffOrdinaryBranchPlaneTypeNonZeroInput,
    CoeffOrdinaryPassError, CoeffOrdinaryPlaneTypeStateContextConfig,
    CoeffOrdinaryStateContextConfig, CoeffOrdinaryStateContextPassInput,
    NonZeroCoeffOrdinaryDerivedBasePass, apply_coeff_ordinary_branch,
    apply_coeff_ordinary_branch_from_plane_tx_type, apply_coeff_ordinary_branch_from_plane_type,
    apply_nonzero_coeff_ordinary_pass_with_state_context,
};
use super::scan_walk::{NonZeroCoeffScanWalk, walk_nonzero_coeff_scan};
use super::sign_symbol::{CoeffSignCdfSyntax, CoeffSignReadSource};
use super::test_support::symbol_decoder;
use super::*;

const DC_ONLY_SCAN: [u16; 1] = [0];
const PAYLOAD_SUFFIXES: [[u8; 3]; 4] = [
    [0x00, 0x00, 0x80],
    [0xff, 0x00, 0x80],
    [0x55, 0xaa, 0x80],
    [0xff, 0xff, 0x80],
];

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

fn setup_start_and_walk<'a>(
    payload: &'a [u8],
    scan: &[u16],
) -> Option<(
    TileCdfSubset,
    SymbolDecoder<'a>,
    NonZeroCoeffBlockStart,
    NonZeroCoeffScanWalk,
)> {
    setup_start_and_walk_with_input(payload, scan, nonzero_start_input())
}

fn setup_start_and_walk_with_input<'a>(
    payload: &'a [u8],
    scan: &[u16],
    start_input: NonZeroCoeffBlockStartInput,
) -> Option<(
    TileCdfSubset,
    SymbolDecoder<'a>,
    NonZeroCoeffBlockStart,
    NonZeroCoeffScanWalk,
)> {
    let (tile, symbols, start) = setup_start_with_input(payload, start_input)?;
    if start.eob_read().eob().eob() != scan.len() {
        return None;
    }
    let walk = walk_nonzero_coeff_scan(&start, scan).ok()?;
    Some((tile, symbols, start, walk))
}

fn luma_base_config(parity_hiding: bool, use_tcq: bool) -> CoeffBaseDerivedLevelPassConfig {
    base_config_for_plane(0, parity_hiding, use_tcq)
}

fn base_config_for_plane(
    plane: usize,
    parity_hiding: bool,
    use_tcq: bool,
) -> CoeffBaseDerivedLevelPassConfig {
    CoeffBaseDerivedLevelPassConfig {
        coeff_cdf_q_ctx: 0,
        tx_size_ctx: 0,
        tx_width_log2: 3,
        tx_width: 8,
        tx_height: 8,
        plane,
        tx_class: CoeffTransformClass::TwoD,
        parity_hiding,
        use_tcq,
    }
}

fn luma_plane_tx_type_base_config(
    plane_tx_type: usize,
    parity_hiding: bool,
    use_tcq: bool,
) -> CoeffOrdinaryBranchPlaneTxTypeBaseConfig {
    plane_tx_type_base_config_for_plane(0, plane_tx_type, parity_hiding, use_tcq)
}

fn plane_tx_type_base_config_for_plane(
    plane: usize,
    plane_tx_type: usize,
    parity_hiding: bool,
    use_tcq: bool,
) -> CoeffOrdinaryBranchPlaneTxTypeBaseConfig {
    CoeffOrdinaryBranchPlaneTxTypeBaseConfig {
        coeff_cdf_q_ctx: 0,
        tx_size_ctx: 0,
        tx_width_log2: 3,
        tx_width: 8,
        tx_height: 8,
        plane,
        plane_tx_type,
        parity_hiding,
        use_tcq,
    }
}

fn state_context_config() -> CoeffOrdinaryStateContextConfig {
    state_context_config_for_plane_type(0)
}

fn state_context_config_for_plane_type(plane_type: usize) -> CoeffOrdinaryStateContextConfig {
    CoeffOrdinaryStateContextConfig {
        coeff_cdf_q_ctx: 0,
        plane_type,
        x4: 0,
        y4: 0,
        w4: 2,
        h4: 2,
    }
}

fn plane_type_state_context_config() -> CoeffOrdinaryPlaneTypeStateContextConfig {
    plane_type_state_context_config_for_geometry(0, 0, 2, 2)
}

fn plane_type_state_context_config_for_geometry(
    x4: usize,
    y4: usize,
    w4: usize,
    h4: usize,
) -> CoeffOrdinaryPlaneTypeStateContextConfig {
    CoeffOrdinaryPlaneTypeStateContextConfig {
        coeff_cdf_q_ctx: 0,
        x4,
        y4,
        w4,
        h4,
    }
}

fn geometry_state_context_config() -> CoeffOrdinaryGeometryStateContextConfig {
    CoeffOrdinaryGeometryStateContextConfig { coeff_cdf_q_ctx: 0 }
}

fn invalid_update_state_context_config() -> CoeffOrdinaryStateContextConfig {
    CoeffOrdinaryStateContextConfig {
        x4: 5,
        ..state_context_config()
    }
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

fn state_context_pass_for_payload(
    payload: &[u8],
    base_config: CoeffBaseDerivedLevelPassConfig,
    state_context: CoeffOrdinaryStateContextConfig,
) -> Option<(TileCoeffContextState, NonZeroCoeffOrdinaryDerivedBasePass)> {
    state_context_pass_for_payload_with_start(
        payload,
        nonzero_start_input(),
        base_config,
        state_context,
    )
}

fn state_context_pass_for_payload_with_start(
    payload: &[u8],
    start_input: NonZeroCoeffBlockStartInput,
    base_config: CoeffBaseDerivedLevelPassConfig,
    state_context: CoeffOrdinaryStateContextConfig,
) -> Option<(TileCoeffContextState, NonZeroCoeffOrdinaryDerivedBasePass)> {
    let (mut tile, mut symbols, start, _walk) =
        setup_start_and_walk_with_input(payload, &DC_ONLY_SCAN, start_input)?;
    let mut context_state = seeded_context_state();
    let pass = apply_nonzero_coeff_ordinary_pass_with_state_context(
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
    .ok()?;
    Some((context_state, pass))
}

fn branch_nonzero_input(
    start: NonZeroCoeffBlockStartInput,
    base_config: CoeffBaseDerivedLevelPassConfig,
    state_context: CoeffOrdinaryStateContextConfig,
) -> CoeffOrdinaryBranchInput<'static> {
    CoeffOrdinaryBranchInput::NonZero(CoeffOrdinaryBranchNonZeroInput {
        start,
        scan: &DC_ONLY_SCAN,
        base_config,
        state_context,
        lossless: false,
    })
}

fn branch_plane_tx_type_nonzero_input(
    start: NonZeroCoeffBlockStartInput,
    base_config: CoeffOrdinaryBranchPlaneTxTypeBaseConfig,
    state_context: CoeffOrdinaryStateContextConfig,
) -> CoeffOrdinaryBranchPlaneTxTypeInput<'static> {
    CoeffOrdinaryBranchPlaneTxTypeInput::NonZero(CoeffOrdinaryBranchPlaneTxTypeNonZeroInput {
        start,
        scan: &DC_ONLY_SCAN,
        base_config,
        state_context,
        lossless: false,
    })
}

fn branch_plane_type_nonzero_input(
    start: NonZeroCoeffBlockStartInput,
    base_config: CoeffOrdinaryBranchPlaneTxTypeBaseConfig,
    state_context: CoeffOrdinaryPlaneTypeStateContextConfig,
) -> CoeffOrdinaryBranchPlaneTypeInput<'static> {
    CoeffOrdinaryBranchPlaneTypeInput::NonZero(CoeffOrdinaryBranchPlaneTypeNonZeroInput {
        start,
        scan: &DC_ONLY_SCAN,
        base_config,
        state_context,
        lossless: false,
    })
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

fn nonzero_start_input() -> NonZeroCoeffBlockStartInput {
    nonzero_start_input_for_plane(0)
}

fn nonzero_start_input_for_plane(plane: usize) -> NonZeroCoeffBlockStartInput {
    nonzero_start_input_for_plane_and_geometry(plane, 0, 0, 2, 2)
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

fn run_plane_tx_type_branch(
    payload: &[u8],
    input: CoeffOrdinaryBranchPlaneTxTypeInput<'_>,
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
    let branch = apply_coeff_ordinary_branch_from_plane_tx_type(
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

fn run_plane_type_branch(
    payload: &[u8],
    input: CoeffOrdinaryBranchPlaneTypeInput<'_>,
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
    let branch = apply_coeff_ordinary_branch_from_plane_type(
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

fn find_state_context_payload(
    base_config: CoeffBaseDerivedLevelPassConfig,
    state_context: CoeffOrdinaryStateContextConfig,
    predicate: impl Fn(&NonZeroCoeffOrdinaryDerivedBasePass) -> bool,
) -> [u8; 12] {
    find_state_context_payload_with_start(
        nonzero_start_input(),
        base_config,
        state_context,
        predicate,
    )
}

fn find_state_context_payload_with_start(
    start_input: NonZeroCoeffBlockStartInput,
    base_config: CoeffBaseDerivedLevelPassConfig,
    state_context: CoeffOrdinaryStateContextConfig,
    predicate: impl Fn(&NonZeroCoeffOrdinaryDerivedBasePass) -> bool,
) -> [u8; 12] {
    for first in u8::MIN..=u8::MAX {
        for second in u8::MIN..=u8::MAX {
            for suffix in PAYLOAD_SUFFIXES {
                let payload = payload_from(first, second, suffix);
                let Some((_state, pass)) = state_context_pass_for_payload_with_start(
                    &payload,
                    start_input,
                    base_config,
                    state_context,
                ) else {
                    continue;
                };
                if predicate(&pass) {
                    return payload;
                }
            }
        }
    }
    panic!("no state-context ordinary coefficient payload found");
}

#[test]
fn coefficient_ordinary_branch_all_zero_preserves_symbol_state() {
    let frame = FrameCdfSubset::from_defaults();
    let mut tile = frame.tile_copy();
    let mut symbols = symbol_decoder(&[0x80]);
    let mut context_state = seeded_context_state();
    let consumed_before = symbols.consumed_bits();
    let symbols_before = symbols.symbol_count();

    let branch = apply_coeff_ordinary_branch(
        &mut context_state,
        &mut tile,
        &mut symbols,
        CoeffOrdinaryBranchInput::AllZero(AllZeroCoeffBlockInput {
            plane: 0,
            x4: 0,
            y4: 0,
            w4: 2,
            h4: 2,
        }),
    )
    .unwrap();

    let CoeffOrdinaryBranch::AllZero(block) = branch else {
        panic!("expected all-zero branch");
    };
    assert_eq!(block.eob(), 0);
    assert_eq!(block.cul_level(), 0);
    assert_eq!(block.dc_category(), 0);
    assert_eq!(symbols.consumed_bits(), consumed_before);
    assert_eq!(symbols.symbol_count(), symbols_before);
    assert_eq!(&context_state.above_level(0).unwrap()[0..2], &[0, 0]);
    assert_eq!(&context_state.left_level(0).unwrap()[0..2], &[0, 0]);
    assert_eq!(&context_state.above_dc(0).unwrap()[0..2], &[0, 0]);
    assert_eq!(&context_state.left_dc(0).unwrap()[0..2], &[0, 0]);
}

#[test]
fn coefficient_ordinary_branch_nonzero_runs_state_context_pass() {
    let base_config = luma_base_config(false, false);
    let state_context = state_context_config();
    let payload = find_state_context_payload(base_config, state_context, |pass| {
        dc_sign_ctx_from(pass) == Some(1)
    });
    let frame = FrameCdfSubset::from_defaults();
    let mut tile = frame.tile_copy();
    let mut symbols = symbol_decoder(&payload);
    let mut context_state = seeded_context_state();

    let branch = apply_coeff_ordinary_branch(
        &mut context_state,
        &mut tile,
        &mut symbols,
        branch_nonzero_input(nonzero_start_input(), base_config, state_context),
    )
    .unwrap();

    let CoeffOrdinaryBranch::NonZero(pass) = branch else {
        panic!("expected nonzero branch");
    };
    let quant_state = pass.quant_pass().quant_state();

    assert_eq!(dc_sign_ctx_from(&pass), Some(1));
    assert_eq!(
        &context_state.above_level(0).unwrap()[0..2],
        &[quant_state.cul_level(); 2]
    );
    assert_eq!(
        &context_state.left_level(0).unwrap()[0..2],
        &[quant_state.cul_level(); 2]
    );
    assert_eq!(
        &context_state.above_dc(0).unwrap()[0..2],
        &[quant_state.dc_category(); 2]
    );
    assert_eq!(
        &context_state.left_dc(0).unwrap()[0..2],
        &[quant_state.dc_category(); 2]
    );
}

#[test]
fn coefficient_ordinary_branch_invalid_nonzero_start_preserves_mutable_state() {
    let frame = FrameCdfSubset::from_defaults();
    let mut tile = frame.tile_copy();
    let tile_before = tile.clone();
    let mut symbols = symbol_decoder(&[0x80]);
    let consumed_before = symbols.consumed_bits();
    let symbols_before = symbols.symbol_count();
    let mut context_state = seeded_context_state();
    let context_before = context_state.clone();
    let mut start = nonzero_start_input();
    start.eob.tx_width_log2 = 1;

    let err = apply_coeff_ordinary_branch(
        &mut context_state,
        &mut tile,
        &mut symbols,
        branch_nonzero_input(
            start,
            luma_base_config(false, false),
            state_context_config(),
        ),
    )
    .unwrap_err();

    assert!(matches!(
        err,
        CoeffOrdinaryBranchError::Branch(CoeffLoopContextError::InvalidEobTransformLog2 {
            axis: "width",
            value: 1,
            minimum: 2,
        })
    ));
    assert_eq!(context_state, context_before);
    assert_eq!(tile, tile_before);
    assert_eq!(symbols.consumed_bits(), consumed_before);
    assert_eq!(symbols.symbol_count(), symbols_before);
}

#[test]
fn coefficient_ordinary_branch_preserves_context_on_ordinary_pass_failure() {
    let payload = find_state_context_payload(
        luma_base_config(false, false),
        state_context_config(),
        |_| true,
    );
    let frame = FrameCdfSubset::from_defaults();
    let mut tile = frame.tile_copy();
    let mut symbols = symbol_decoder(&payload);
    let mut context_state = seeded_context_state();
    let context_before = context_state.clone();

    let err = apply_coeff_ordinary_branch(
        &mut context_state,
        &mut tile,
        &mut symbols,
        branch_nonzero_input(
            nonzero_start_input(),
            luma_base_config(true, true),
            state_context_config(),
        ),
    )
    .unwrap_err();

    assert!(matches!(
        err,
        CoeffOrdinaryBranchError::Ordinary(CoeffOrdinaryPassError::BaseDerived(
            CoeffBaseDerivedLevelPassError::InconsistentParityAndTcq
        ))
    ));
    assert_eq!(context_state, context_before);
}

#[test]
fn coefficient_ordinary_branch_plane_tx_type_nonzero_matches_direct_tx_class() {
    let cases = [
        (0, CoeffTransformClass::TwoD),
        (10, CoeffTransformClass::Vertical),
        (11, CoeffTransformClass::Horizontal),
        (usize::MAX, CoeffTransformClass::TwoD),
    ];

    for (plane_tx_type, tx_class) in cases {
        let mut direct_base_config = luma_base_config(false, false);
        direct_base_config.tx_class = tx_class;
        let plane_tx_type_base_config = luma_plane_tx_type_base_config(plane_tx_type, false, false);
        let state_context = state_context_config();
        let payload = find_state_context_payload(direct_base_config, state_context, |_| true);

        let direct = run_direct_branch(
            &payload,
            branch_nonzero_input(nonzero_start_input(), direct_base_config, state_context),
        );
        let derived = run_plane_tx_type_branch(
            &payload,
            branch_plane_tx_type_nonzero_input(
                nonzero_start_input(),
                plane_tx_type_base_config,
                state_context,
            ),
        );

        assert_eq!(derived, direct, "PlaneTxType {plane_tx_type}");
    }
}

#[test]
fn coefficient_ordinary_branch_plane_tx_type_all_zero_preserves_direct_branch() {
    let direct = run_direct_branch(
        &[0x80],
        CoeffOrdinaryBranchInput::AllZero(AllZeroCoeffBlockInput {
            plane: 0,
            x4: 0,
            y4: 0,
            w4: 2,
            h4: 2,
        }),
    );
    let derived = run_plane_tx_type_branch(
        &[0x80],
        CoeffOrdinaryBranchPlaneTxTypeInput::AllZero(AllZeroCoeffBlockInput {
            plane: 0,
            x4: 0,
            y4: 0,
            w4: 2,
            h4: 2,
        }),
    );

    assert_eq!(derived, direct);
}

#[test]
fn coefficient_ordinary_branch_plane_type_nonzero_matches_explicit_plane_type() {
    for plane in [0, 1, 2] {
        let start = nonzero_start_input_for_plane(plane);
        let direct_base_config = base_config_for_plane(plane, false, false);
        let direct_state_context =
            state_context_config_for_plane_type(if plane > 0 { 1 } else { 0 });
        let derived_base_config = plane_tx_type_base_config_for_plane(plane, 0, false, false);
        let derived_state_context = plane_type_state_context_config();
        let payload = find_state_context_payload_with_start(
            start,
            direct_base_config,
            direct_state_context,
            |_| true,
        );

        let direct = run_direct_branch(
            &payload,
            branch_nonzero_input(start, direct_base_config, direct_state_context),
        );
        let derived = run_plane_type_branch(
            &payload,
            branch_plane_type_nonzero_input(start, derived_base_config, derived_state_context),
        );

        assert_eq!(derived, direct, "plane {plane}");
    }
}

#[test]
fn coefficient_ordinary_branch_plane_type_all_zero_preserves_direct_branch() {
    let direct = run_direct_branch(
        &[0x80],
        CoeffOrdinaryBranchInput::AllZero(AllZeroCoeffBlockInput {
            plane: 2,
            x4: 0,
            y4: 0,
            w4: 2,
            h4: 2,
        }),
    );
    let derived = run_plane_type_branch(
        &[0x80],
        CoeffOrdinaryBranchPlaneTypeInput::AllZero(AllZeroCoeffBlockInput {
            plane: 2,
            x4: 0,
            y4: 0,
            w4: 2,
            h4: 2,
        }),
    );

    assert_eq!(derived, direct);
}

#[test]
fn coefficient_ordinary_branch_geometry_nonzero_matches_explicit_geometry() {
    let start = nonzero_start_input_for_plane_and_geometry(1, 1, 1, 2, 2);
    let direct_base_config = base_config_for_plane(1, false, false);
    let direct_state_context = CoeffOrdinaryStateContextConfig {
        coeff_cdf_q_ctx: 0,
        plane_type: 1,
        x4: start.block.x4,
        y4: start.block.y4,
        w4: start.block.w4,
        h4: start.block.h4,
    };
    let explicit_state_context = plane_type_state_context_config_for_geometry(
        start.block.x4,
        start.block.y4,
        start.block.w4,
        start.block.h4,
    );
    let derived_base_config = plane_tx_type_base_config_for_plane(1, 0, false, false);
    let payload = find_state_context_payload_with_start(
        start,
        direct_base_config,
        direct_state_context,
        |_| true,
    );

    let explicit = run_plane_type_branch(
        &payload,
        branch_plane_type_nonzero_input(start, derived_base_config, explicit_state_context),
    );
    let derived = run_geometry_branch(
        &payload,
        branch_geometry_nonzero_input(start, derived_base_config, geometry_state_context_config()),
    );

    assert_eq!(derived, explicit);
}

#[test]
fn coefficient_ordinary_branch_geometry_all_zero_preserves_direct_branch() {
    let block = AllZeroCoeffBlockInput {
        plane: 1,
        x4: 1,
        y4: 1,
        w4: 2,
        h4: 2,
    };
    let direct = run_direct_branch(&[0x80], CoeffOrdinaryBranchInput::AllZero(block));
    let derived = run_geometry_branch(&[0x80], CoeffOrdinaryBranchGeometryInput::AllZero(block));

    assert_eq!(derived, direct);
}

fn dc_sign_ctx_from(pass: &NonZeroCoeffOrdinaryDerivedBasePass) -> Option<usize> {
    pass.derived_sign_inputs()
        .iter()
        .find_map(|input| match input.source {
            CoeffSignReadSource::Cdf {
                syntax: CoeffSignCdfSyntax::DcSign,
                selector,
            } if input.entry.scan_index() == 0 => Some(selector.ctx),
            _ => None,
        })
}

#[test]
fn coefficient_ordinary_pass_with_state_context_reads_dc_before_commit() {
    let base_config = luma_base_config(false, false);
    let state_context = state_context_config();
    let payload = find_state_context_payload(base_config, state_context, |pass| {
        dc_sign_ctx_from(pass) == Some(1)
    });
    let (context_state, pass) =
        state_context_pass_for_payload(&payload, base_config, state_context).unwrap();
    let quant_state = pass.quant_pass().quant_state();

    assert_eq!(dc_sign_ctx_from(&pass), Some(1));
    assert_eq!(
        &context_state.above_level(0).unwrap()[0..2],
        &[quant_state.cul_level(); 2]
    );
    assert_eq!(
        &context_state.left_level(0).unwrap()[0..2],
        &[quant_state.cul_level(); 2]
    );
    assert_eq!(
        &context_state.above_dc(0).unwrap()[0..2],
        &[quant_state.dc_category(); 2]
    );
    assert_eq!(
        &context_state.left_dc(0).unwrap()[0..2],
        &[quant_state.dc_category(); 2]
    );
}

#[test]
fn coefficient_ordinary_pass_with_state_context_preserves_context_on_pass_failure() {
    let payload = find_state_context_payload(
        luma_base_config(false, false),
        state_context_config(),
        |_| true,
    );
    let (mut tile, mut symbols, start, _walk) =
        setup_start_and_walk(&payload, &DC_ONLY_SCAN).unwrap();
    let mut context_state = seeded_context_state();
    let context_before = context_state.clone();

    let err = apply_nonzero_coeff_ordinary_pass_with_state_context(
        &mut context_state,
        &mut tile,
        &mut symbols,
        CoeffOrdinaryStateContextPassInput {
            start,
            scan: &DC_ONLY_SCAN,
            base_config: luma_base_config(true, true),
            state_context: state_context_config(),
            lossless: false,
        },
    )
    .unwrap_err();

    assert!(matches!(
        err,
        CoeffOrdinaryPassError::BaseDerived(
            CoeffBaseDerivedLevelPassError::InconsistentParityAndTcq
        )
    ));
    assert_eq!(context_state, context_before);
}

#[test]
fn coefficient_ordinary_pass_with_state_context_preserves_context_on_update_failure() {
    let base_config = luma_base_config(false, false);
    let payload = find_state_context_payload(base_config, state_context_config(), |_| true);
    let (mut tile, mut symbols, start, _walk) =
        setup_start_and_walk(&payload, &DC_ONLY_SCAN).unwrap();
    let mut context_state = seeded_context_state();
    let context_before = context_state.clone();

    let err = apply_nonzero_coeff_ordinary_pass_with_state_context(
        &mut context_state,
        &mut tile,
        &mut symbols,
        CoeffOrdinaryStateContextPassInput {
            start,
            scan: &DC_ONLY_SCAN,
            base_config,
            state_context: invalid_update_state_context_config(),
            lossless: false,
        },
    )
    .unwrap_err();

    assert!(matches!(
        err,
        CoeffOrdinaryPassError::ContextUpdate(TileCoeffStateError::ContextRangeOutOfBounds {
            context: "above",
            start: 5,
            end: 7,
            len: 6,
        })
    ));
    assert_eq!(context_state, context_before);
}
