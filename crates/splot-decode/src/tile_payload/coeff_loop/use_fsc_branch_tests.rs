// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::panic, clippy::unwrap_used)]

use splot_core::span::ByteOffset;
use splot_core::symbol::{CdfUpdateMode, SymbolBitPosition, SymbolDecoder, SymbolDecoderConfig};

use super::super::cdf::{FrameCdfSubset, TileCdfSubset};
use super::super::coeff_state::{CoeffContextUpdate, TileCoeffContextState};
use super::fsc_quant_pass::{
    CoeffFscBranch, CoeffFscBranchError, CoeffFscBranchTxSizeInput,
    CoeffFscBranchTxSizeNonZeroInput, apply_coeff_fsc_branch_from_tx_size,
};
use super::ordinary_pass::geometry::{
    CoeffOrdinaryBranchLosslessBaseConfig, CoeffOrdinaryBranchLosslessInput,
    CoeffOrdinaryBranchLosslessNonZeroInput, CoeffOrdinaryTxSizeGeometryConfig,
    apply_coeff_ordinary_branch_from_lossless,
};
use super::ordinary_pass::{CoeffOrdinaryBranch, CoeffOrdinaryBranchError};
use super::use_fsc_branch::{
    CoeffUseFscBranch, CoeffUseFscBranchError, CoeffUseFscBranchInput,
    CoeffUseFscBranchNonZeroInput, CoeffUseFscConditionFacts, CoeffUseFscConditionInput,
    CoeffUseFscConditionNonZeroInput, CoeffUseFscSharedFacts, CoeffUseFscSharedFactsInput,
    CoeffUseFscSharedFactsNonZeroInput, apply_coeff_use_fsc_branch,
    apply_coeff_use_fsc_branch_from_condition, apply_coeff_use_fsc_branch_from_shared_facts,
};
use super::*;

const DCT_DCT: usize = 0;
const IDTX: usize = 9;
const TX_8X8: usize = 1;
const UV_SMOOTH_PRED: usize = 9;
const ORDINARY_PAYLOAD_SUFFIXES: [[u8; 3]; 4] = [
    [0x00, 0x00, 0x80],
    [0xff, 0x00, 0x80],
    [0x55, 0xaa, 0x80],
    [0xff, 0xff, 0x80],
];
const FSC_PAYLOAD_SUFFIXES: [[u8; 6]; 6] = [
    [0x00, 0x00, 0x00, 0x00, 0x00, 0x80],
    [0xff, 0x00, 0x00, 0x00, 0x00, 0x80],
    [0x55, 0xaa, 0x00, 0x00, 0x00, 0x80],
    [0xff, 0xff, 0x00, 0x00, 0x00, 0x80],
    [0x00, 0x00, 0b0011_0100, 0x00, 0x00, 0x80],
    [0xff, 0xff, 0b0011_0100, 0xff, 0x00, 0x80],
];

type OrdinaryRun = (
    CoeffOrdinaryBranch,
    TileCoeffContextState,
    TileCdfSubset,
    SymbolBitPosition,
    u64,
);
type FscRun = (
    CoeffFscBranch,
    TileCoeffContextState,
    TileCdfSubset,
    SymbolBitPosition,
    u64,
);
type SelectorRun = (
    CoeffUseFscBranch,
    TileCoeffContextState,
    TileCdfSubset,
    SymbolBitPosition,
    u64,
);

fn symbol_decoder(payload: &[u8]) -> SymbolDecoder<'_> {
    SymbolDecoder::with_base_and_config(
        payload,
        ByteOffset::new(0),
        SymbolDecoderConfig::new().with_cdf_update_mode(CdfUpdateMode::Enabled),
    )
    .unwrap()
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

fn geometry(tx_size: usize) -> CoeffOrdinaryTxSizeGeometryConfig {
    CoeffOrdinaryTxSizeGeometryConfig {
        plane: 0,
        start_x: 0,
        start_y: 0,
        tx_size,
    }
}

fn geometry_for_plane(plane: usize, tx_size: usize) -> CoeffOrdinaryTxSizeGeometryConfig {
    CoeffOrdinaryTxSizeGeometryConfig {
        plane,
        ..geometry(tx_size)
    }
}

fn ordinary_base_config() -> CoeffOrdinaryBranchLosslessBaseConfig {
    CoeffOrdinaryBranchLosslessBaseConfig {
        reduced_tx_set: 0,
        enable_chroma_dctonly: false,
        uv_mode: UV_SMOOTH_PRED,
        angle_delta_uv: 0,
        luma_tx_type: DCT_DCT,
        chroma_inter_tx_type: DCT_DCT,
        parity_hiding: false,
        use_tcq: false,
    }
}

fn invalid_ordinary_base_config() -> CoeffOrdinaryBranchLosslessBaseConfig {
    CoeffOrdinaryBranchLosslessBaseConfig {
        reduced_tx_set: usize::MAX,
        ..ordinary_base_config()
    }
}

fn ordinary_nonzero_input(tx_size: usize) -> CoeffOrdinaryBranchLosslessNonZeroInput {
    ordinary_nonzero_input_for_geometry(geometry(tx_size))
}

fn ordinary_nonzero_input_for_geometry(
    geometry: CoeffOrdinaryTxSizeGeometryConfig,
) -> CoeffOrdinaryBranchLosslessNonZeroInput {
    CoeffOrdinaryBranchLosslessNonZeroInput {
        geometry,
        coeff_cdf_q_ctx: 0,
        is_inter: false,
        base_config: ordinary_base_config(),
        lossless: true,
    }
}

fn ordinary_inter_input(tx_size: usize) -> CoeffOrdinaryBranchLosslessNonZeroInput {
    CoeffOrdinaryBranchLosslessNonZeroInput {
        is_inter: true,
        ..ordinary_nonzero_input(tx_size)
    }
}

fn fsc_block() -> AllZeroCoeffBlockInput {
    AllZeroCoeffBlockInput {
        plane: 0,
        x4: 0,
        y4: 0,
        w4: 2,
        h4: 2,
    }
}

fn fsc_nonzero_input(
    tx_size: usize,
    block: AllZeroCoeffBlockInput,
) -> CoeffFscBranchTxSizeNonZeroInput {
    fsc_nonzero_input_with_plane_tx_type(tx_size, block, DCT_DCT)
}

fn fsc_nonzero_input_with_plane_tx_type(
    tx_size: usize,
    block: AllZeroCoeffBlockInput,
    plane_tx_type: usize,
) -> CoeffFscBranchTxSizeNonZeroInput {
    CoeffFscBranchTxSizeNonZeroInput {
        block,
        tx_size,
        plane_tx_type,
        is_inter: false,
        coeff_cdf_q_ctx: 0,
    }
}

fn run_direct_ordinary(
    payload: &[u8],
    input: CoeffOrdinaryBranchLosslessInput,
) -> Option<OrdinaryRun> {
    let frame = FrameCdfSubset::from_defaults();
    let mut tile = frame.tile_copy();
    let mut symbols = symbol_decoder(payload);
    let mut context = seeded_context_state();
    let branch =
        apply_coeff_ordinary_branch_from_lossless(&mut context, &mut tile, &mut symbols, input)
            .ok()?;
    Some((
        branch,
        context,
        tile,
        symbols.consumed_bits(),
        symbols.symbol_count(),
    ))
}

fn run_direct_fsc(payload: &[u8], input: CoeffFscBranchTxSizeInput) -> Option<FscRun> {
    let frame = FrameCdfSubset::from_defaults();
    let mut tile = frame.tile_copy();
    let mut symbols = symbol_decoder(payload);
    let mut context = seeded_context_state();
    let branch =
        apply_coeff_fsc_branch_from_tx_size(&mut context, &mut tile, &mut symbols, input).ok()?;
    Some((
        branch,
        context,
        tile,
        symbols.consumed_bits(),
        symbols.symbol_count(),
    ))
}

fn run_selector(payload: &[u8], input: CoeffUseFscBranchInput) -> Option<SelectorRun> {
    let frame = FrameCdfSubset::from_defaults();
    let mut tile = frame.tile_copy();
    let mut symbols = symbol_decoder(payload);
    let mut context = seeded_context_state();
    let branch = apply_coeff_use_fsc_branch(&mut context, &mut tile, &mut symbols, input).ok()?;
    Some((
        branch,
        context,
        tile,
        symbols.consumed_bits(),
        symbols.symbol_count(),
    ))
}

fn run_condition(payload: &[u8], input: CoeffUseFscConditionInput) -> Option<SelectorRun> {
    let frame = FrameCdfSubset::from_defaults();
    let mut tile = frame.tile_copy();
    let mut symbols = symbol_decoder(payload);
    let mut context = seeded_context_state();
    let branch =
        apply_coeff_use_fsc_branch_from_condition(&mut context, &mut tile, &mut symbols, input)
            .ok()?;
    Some((
        branch,
        context,
        tile,
        symbols.consumed_bits(),
        symbols.symbol_count(),
    ))
}

fn run_shared(payload: &[u8], input: CoeffUseFscSharedFactsInput) -> Option<SelectorRun> {
    let frame = FrameCdfSubset::from_defaults();
    let mut tile = frame.tile_copy();
    let mut symbols = symbol_decoder(payload);
    let mut context = seeded_context_state();
    let branch =
        apply_coeff_use_fsc_branch_from_shared_facts(&mut context, &mut tile, &mut symbols, input)
            .ok()?;
    Some((
        branch,
        context,
        tile,
        symbols.consumed_bits(),
        symbols.symbol_count(),
    ))
}

fn condition_facts(
    enable_fsc: bool,
    plane_tx_type: usize,
    plane: usize,
    fsc_mode: bool,
    is_inter: bool,
) -> CoeffUseFscConditionFacts {
    CoeffUseFscConditionFacts {
        enable_fsc,
        plane_tx_type,
        plane,
        fsc_mode,
        is_inter,
    }
}

fn shared_facts(
    geometry: CoeffOrdinaryTxSizeGeometryConfig,
    enable_fsc: bool,
    plane_tx_type: usize,
    fsc_mode: bool,
    is_inter: bool,
) -> CoeffUseFscSharedFacts {
    CoeffUseFscSharedFacts {
        geometry,
        enable_fsc,
        plane_tx_type,
        fsc_mode,
        is_inter,
        coeff_cdf_q_ctx: 0,
    }
}

fn shared_nonzero_input(facts: CoeffUseFscSharedFacts) -> CoeffUseFscSharedFactsNonZeroInput {
    CoeffUseFscSharedFactsNonZeroInput {
        facts,
        ordinary_base_config: ordinary_base_config(),
        lossless: true,
    }
}

fn ordinary_payload_from(first: u8, second: u8, suffix: [u8; 3]) -> [u8; 12] {
    [
        first, second, suffix[0], suffix[1], suffix[2], 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x80,
    ]
}

fn find_ordinary_payload() -> [u8; 12] {
    find_ordinary_payload_for(|| ordinary_nonzero_input(TX_8X8))
}

fn find_ordinary_payload_for(
    input: impl Fn() -> CoeffOrdinaryBranchLosslessNonZeroInput,
) -> [u8; 12] {
    for first in u8::MIN..=u8::MAX {
        for second in u8::MIN..=u8::MAX {
            for suffix in ORDINARY_PAYLOAD_SUFFIXES {
                let payload = ordinary_payload_from(first, second, suffix);
                if run_direct_ordinary(&payload, CoeffOrdinaryBranchLosslessInput::NonZero(input()))
                    .is_some()
                {
                    return payload;
                }
            }
        }
    }
    panic!("no ordinary coefficient useFsc payload found");
}

fn find_fsc_payload() -> [u8; 8] {
    for first in u8::MIN..=u8::MAX {
        for second in u8::MIN..=u8::MAX {
            for suffix in FSC_PAYLOAD_SUFFIXES {
                let payload = [
                    first, second, suffix[0], suffix[1], suffix[2], suffix[3], suffix[4], suffix[5],
                ];
                if run_direct_fsc(
                    &payload,
                    CoeffFscBranchTxSizeInput::NonZero(fsc_nonzero_input(TX_8X8, fsc_block())),
                )
                .is_some()
                {
                    return payload;
                }
            }
        }
    }
    panic!("no FSC coefficient useFsc payload found");
}

#[test]
fn coefficient_use_fsc_branch_all_zero_routes_through_ordinary() {
    let expected = run_direct_ordinary(
        &[0x80],
        CoeffOrdinaryBranchLosslessInput::AllZero(geometry(TX_8X8)),
    )
    .unwrap();
    let derived = run_selector(&[0x80], CoeffUseFscBranchInput::AllZero(geometry(TX_8X8))).unwrap();

    assert_eq!(derived.0, CoeffUseFscBranch::Ordinary(expected.0));
    assert_eq!(derived.1, expected.1);
    assert_eq!(derived.2, expected.2);
    assert_eq!(derived.3, expected.3);
    assert_eq!(derived.4, expected.4);
}

#[test]
fn coefficient_use_fsc_branch_false_delegates_to_ordinary() {
    let payload = find_ordinary_payload();
    let expected = run_direct_ordinary(
        &payload,
        CoeffOrdinaryBranchLosslessInput::NonZero(ordinary_nonzero_input(TX_8X8)),
    )
    .unwrap();
    let derived = run_selector(
        &payload,
        CoeffUseFscBranchInput::NonZero(CoeffUseFscBranchNonZeroInput {
            use_fsc: false,
            ordinary: ordinary_nonzero_input(TX_8X8),
            fsc: fsc_nonzero_input(
                TX_8X8,
                AllZeroCoeffBlockInput {
                    plane: 1,
                    ..fsc_block()
                },
            ),
        }),
    )
    .unwrap();

    assert_eq!(derived.0, CoeffUseFscBranch::Ordinary(expected.0));
    assert_eq!(derived.1, expected.1);
    assert_eq!(derived.2, expected.2);
    assert_eq!(derived.3, expected.3);
    assert_eq!(derived.4, expected.4);
}

#[test]
fn coefficient_use_fsc_branch_true_delegates_to_fsc() {
    let payload = find_fsc_payload();
    let expected = run_direct_fsc(
        &payload,
        CoeffFscBranchTxSizeInput::NonZero(fsc_nonzero_input(TX_8X8, fsc_block())),
    )
    .unwrap();
    let derived = run_selector(
        &payload,
        CoeffUseFscBranchInput::NonZero(CoeffUseFscBranchNonZeroInput {
            use_fsc: true,
            ordinary: ordinary_inter_input(TX_8X8),
            fsc: fsc_nonzero_input(TX_8X8, fsc_block()),
        }),
    )
    .unwrap();

    assert_eq!(derived.0, CoeffUseFscBranch::Fsc(expected.0));
    assert_eq!(derived.1, expected.1);
    assert_eq!(derived.2, expected.2);
    assert_eq!(derived.3, expected.3);
    assert_eq!(derived.4, expected.4);
}

fn assert_selector_error_preserves_state(
    input: CoeffUseFscBranchInput,
    assert_error: impl FnOnce(&CoeffUseFscBranchError),
) {
    let frame = FrameCdfSubset::from_defaults();
    let mut tile = frame.tile_copy();
    let mut symbols = symbol_decoder(&[0x80]);
    let mut context = seeded_context_state();
    let tile_before = tile.clone();
    let context_before = context.clone();
    let consumed_before = symbols.consumed_bits();
    let symbol_count_before = symbols.symbol_count();

    let err = apply_coeff_use_fsc_branch(&mut context, &mut tile, &mut symbols, input).unwrap_err();

    assert_error(&err);
    assert_eq!(context, context_before);
    assert_eq!(tile, tile_before);
    assert_eq!(symbols.consumed_bits(), consumed_before);
    assert_eq!(symbols.symbol_count(), symbol_count_before);
}

#[test]
fn coefficient_use_fsc_branch_false_wraps_ordinary_error() {
    assert_selector_error_preserves_state(
        CoeffUseFscBranchInput::NonZero(CoeffUseFscBranchNonZeroInput {
            use_fsc: false,
            ordinary: ordinary_nonzero_input(25),
            fsc: fsc_nonzero_input(TX_8X8, fsc_block()),
        }),
        |err| {
            assert!(matches!(
                err,
                CoeffUseFscBranchError::Ordinary(CoeffOrdinaryBranchError::InvalidTransformSize {
                    tx_size: 25
                })
            ));
        },
    );
}

#[test]
fn coefficient_use_fsc_branch_true_wraps_fsc_error() {
    assert_selector_error_preserves_state(
        CoeffUseFscBranchInput::NonZero(CoeffUseFscBranchNonZeroInput {
            use_fsc: true,
            ordinary: ordinary_nonzero_input(TX_8X8),
            fsc: fsc_nonzero_input(
                TX_8X8,
                AllZeroCoeffBlockInput {
                    plane: 1,
                    ..fsc_block()
                },
            ),
        }),
        |err| {
            assert!(matches!(
                err,
                CoeffUseFscBranchError::Fsc(CoeffFscBranchError::NonLumaPlane { plane: 1 })
            ));
        },
    );
}

#[test]
fn coefficient_use_fsc_condition_all_zero_matches_selector() {
    let expected =
        run_selector(&[0x80], CoeffUseFscBranchInput::AllZero(geometry(TX_8X8))).unwrap();
    let derived = run_condition(
        &[0x80],
        CoeffUseFscConditionInput::AllZero(geometry(TX_8X8)),
    )
    .unwrap();

    assert_eq!(derived.0, expected.0);
    assert_eq!(derived.1, expected.1);
    assert_eq!(derived.2, expected.2);
    assert_eq!(derived.3, expected.3);
    assert_eq!(derived.4, expected.4);
}

#[test]
fn coefficient_use_fsc_condition_false_delegates_to_ordinary_and_ignores_fsc() {
    let payload = find_ordinary_payload();
    let expected = run_selector(
        &payload,
        CoeffUseFscBranchInput::NonZero(CoeffUseFscBranchNonZeroInput {
            use_fsc: false,
            ordinary: ordinary_nonzero_input(TX_8X8),
            fsc: fsc_nonzero_input(
                TX_8X8,
                AllZeroCoeffBlockInput {
                    plane: 1,
                    ..fsc_block()
                },
            ),
        }),
    )
    .unwrap();
    let false_cases = [
        condition_facts(false, IDTX, 0, true, false),
        condition_facts(true, DCT_DCT, 0, true, false),
        condition_facts(true, IDTX, 1, true, false),
        condition_facts(true, IDTX, 0, false, false),
    ];

    for condition in false_cases {
        let derived = run_condition(
            &payload,
            CoeffUseFscConditionInput::NonZero(CoeffUseFscConditionNonZeroInput {
                condition,
                ordinary: ordinary_nonzero_input(TX_8X8),
                fsc: fsc_nonzero_input(
                    TX_8X8,
                    AllZeroCoeffBlockInput {
                        plane: 1,
                        ..fsc_block()
                    },
                ),
            }),
        )
        .unwrap();

        assert_eq!(derived.0, expected.0, "{condition:?}");
        assert_eq!(derived.1, expected.1, "{condition:?}");
        assert_eq!(derived.2, expected.2, "{condition:?}");
        assert_eq!(derived.3, expected.3, "{condition:?}");
        assert_eq!(derived.4, expected.4, "{condition:?}");
    }
}

#[test]
fn coefficient_use_fsc_condition_true_delegates_to_fsc_and_ignores_ordinary() {
    let payload = find_fsc_payload();
    let expected = run_selector(
        &payload,
        CoeffUseFscBranchInput::NonZero(CoeffUseFscBranchNonZeroInput {
            use_fsc: true,
            ordinary: ordinary_nonzero_input(25),
            fsc: fsc_nonzero_input(TX_8X8, fsc_block()),
        }),
    )
    .unwrap();
    let derived = run_condition(
        &payload,
        CoeffUseFscConditionInput::NonZero(CoeffUseFscConditionNonZeroInput {
            condition: condition_facts(true, IDTX, 0, true, false),
            ordinary: ordinary_nonzero_input(25),
            fsc: fsc_nonzero_input(TX_8X8, fsc_block()),
        }),
    )
    .unwrap();

    assert_eq!(derived.0, expected.0);
    assert_eq!(derived.1, expected.1);
    assert_eq!(derived.2, expected.2);
    assert_eq!(derived.3, expected.3);
    assert_eq!(derived.4, expected.4);
}

#[test]
fn coefficient_use_fsc_condition_inter_true_also_selects_fsc() {
    let payload = find_fsc_payload();
    let expected = run_selector(
        &payload,
        CoeffUseFscBranchInput::NonZero(CoeffUseFscBranchNonZeroInput {
            use_fsc: true,
            ordinary: ordinary_nonzero_input(25),
            fsc: fsc_nonzero_input(TX_8X8, fsc_block()),
        }),
    )
    .unwrap();
    let derived = run_condition(
        &payload,
        CoeffUseFscConditionInput::NonZero(CoeffUseFscConditionNonZeroInput {
            condition: condition_facts(true, IDTX, 0, false, true),
            ordinary: ordinary_nonzero_input(25),
            fsc: fsc_nonzero_input(TX_8X8, fsc_block()),
        }),
    )
    .unwrap();

    assert_eq!(derived.0, expected.0);
    assert_eq!(derived.1, expected.1);
    assert_eq!(derived.2, expected.2);
    assert_eq!(derived.3, expected.3);
    assert_eq!(derived.4, expected.4);
}

#[test]
fn coefficient_use_fsc_shared_facts_all_zero_matches_selector() {
    let expected =
        run_selector(&[0x80], CoeffUseFscBranchInput::AllZero(geometry(TX_8X8))).unwrap();
    let derived = run_shared(
        &[0x80],
        CoeffUseFscSharedFactsInput::AllZero(geometry(TX_8X8)),
    )
    .unwrap();

    assert_eq!(derived.0, expected.0);
    assert_eq!(derived.1, expected.1);
    assert_eq!(derived.2, expected.2);
    assert_eq!(derived.3, expected.3);
    assert_eq!(derived.4, expected.4);
}

#[test]
fn coefficient_use_fsc_shared_facts_false_matches_ordinary_branch() {
    let payload = find_ordinary_payload();
    let expected = run_direct_ordinary(
        &payload,
        CoeffOrdinaryBranchLosslessInput::NonZero(ordinary_nonzero_input(TX_8X8)),
    )
    .unwrap();
    let derived = run_shared(
        &payload,
        CoeffUseFscSharedFactsInput::NonZero(shared_nonzero_input(shared_facts(
            geometry(TX_8X8),
            false,
            IDTX,
            true,
            false,
        ))),
    )
    .unwrap();

    assert_eq!(derived.0, CoeffUseFscBranch::Ordinary(expected.0));
    assert_eq!(derived.1, expected.1);
    assert_eq!(derived.2, expected.2);
    assert_eq!(derived.3, expected.3);
    assert_eq!(derived.4, expected.4);
}

#[test]
fn coefficient_use_fsc_shared_facts_true_matches_fsc_branch() {
    let payload = find_fsc_payload();
    let expected = run_direct_fsc(
        &payload,
        CoeffFscBranchTxSizeInput::NonZero(fsc_nonzero_input_with_plane_tx_type(
            TX_8X8,
            fsc_block(),
            IDTX,
        )),
    )
    .unwrap();
    let derived = run_shared(
        &payload,
        CoeffUseFscSharedFactsInput::NonZero(shared_nonzero_input(shared_facts(
            geometry(TX_8X8),
            true,
            IDTX,
            true,
            false,
        ))),
    )
    .unwrap();

    assert_eq!(derived.0, CoeffUseFscBranch::Fsc(expected.0));
    assert_eq!(derived.1, expected.1);
    assert_eq!(derived.2, expected.2);
    assert_eq!(derived.3, expected.3);
    assert_eq!(derived.4, expected.4);
}

#[test]
fn coefficient_use_fsc_shared_facts_true_ignores_invalid_ordinary_facts() {
    let payload = find_fsc_payload();
    let expected = run_direct_fsc(
        &payload,
        CoeffFscBranchTxSizeInput::NonZero(fsc_nonzero_input_with_plane_tx_type(
            TX_8X8,
            fsc_block(),
            IDTX,
        )),
    )
    .unwrap();
    let derived = run_shared(
        &payload,
        CoeffUseFscSharedFactsInput::NonZero(CoeffUseFscSharedFactsNonZeroInput {
            facts: shared_facts(geometry(TX_8X8), true, IDTX, true, false),
            ordinary_base_config: invalid_ordinary_base_config(),
            lossless: true,
        }),
    )
    .unwrap();

    assert_eq!(derived.0, CoeffUseFscBranch::Fsc(expected.0));
    assert_eq!(derived.1, expected.1);
    assert_eq!(derived.2, expected.2);
    assert_eq!(derived.3, expected.3);
    assert_eq!(derived.4, expected.4);
}

#[test]
fn coefficient_use_fsc_shared_facts_false_does_not_validate_fsc_non_luma() {
    let chroma_geometry = geometry_for_plane(1, TX_8X8);
    let payload =
        find_ordinary_payload_for(|| ordinary_nonzero_input_for_geometry(chroma_geometry));
    let expected = run_direct_ordinary(
        &payload,
        CoeffOrdinaryBranchLosslessInput::NonZero(ordinary_nonzero_input_for_geometry(
            chroma_geometry,
        )),
    )
    .unwrap();
    let derived = run_shared(
        &payload,
        CoeffUseFscSharedFactsInput::NonZero(shared_nonzero_input(shared_facts(
            chroma_geometry,
            true,
            IDTX,
            true,
            false,
        ))),
    )
    .unwrap();

    assert_eq!(derived.0, CoeffUseFscBranch::Ordinary(expected.0));
    assert_eq!(derived.1, expected.1);
    assert_eq!(derived.2, expected.2);
    assert_eq!(derived.3, expected.3);
    assert_eq!(derived.4, expected.4);
}
