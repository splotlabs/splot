// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::panic, clippy::unwrap_used)]

use super::super::cdf::FrameCdfSubset;
use super::super::{TileCoeffFrameFacts, TileCoeffFrameFactsInput};
use super::ordinary_pass::geometry::{
    CoeffOrdinaryBranchLosslessBaseConfig, CoeffOrdinaryTxSizeGeometryConfig,
};
use super::test_support::{BranchRun, run_optional_branch, seeded_context_state, symbol_decoder};
use super::use_fsc_branch::{
    CoeffUseFscBaseQFacts, CoeffUseFscBaseQFactsInput, CoeffUseFscBaseQFactsNonZeroInput,
    CoeffUseFscBranch, CoeffUseFscBranchError, CoeffUseFscFrameBlockFacts,
    CoeffUseFscFrameFactsInput, CoeffUseFscFrameFactsNonZeroInput, CoeffUseFscFrameOrdinaryFacts,
    apply_coeff_use_fsc_branch_from_base_q_facts, apply_coeff_use_fsc_branch_from_frame_facts,
};
use splot_core::segment::MAX_SEGMENTS;

const DCT_DCT: usize = 0;
const IDTX: usize = 9;
const V_DCT: usize = 10;
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

type SelectorRun = BranchRun<CoeffUseFscBranch>;

fn geometry() -> CoeffOrdinaryTxSizeGeometryConfig {
    CoeffOrdinaryTxSizeGeometryConfig {
        plane: 0,
        start_x: 0,
        start_y: 0,
        tx_size: TX_8X8,
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

fn ordinary_facts() -> CoeffUseFscFrameOrdinaryFacts {
    CoeffUseFscFrameOrdinaryFacts {
        uv_mode: UV_SMOOTH_PRED,
        angle_delta_uv: 0,
        luma_tx_type: DCT_DCT,
        chroma_inter_tx_type: DCT_DCT,
    }
}

#[allow(clippy::too_many_arguments, clippy::fn_params_excessive_bools)]
fn frame_facts(
    enable_fsc: bool,
    enable_chroma_dctonly: bool,
    reduced_tx_set: usize,
    allow_tcq: bool,
    allow_parity_hiding: bool,
    base_q_idx: u32,
    lossless_segment: Option<usize>,
) -> TileCoeffFrameFacts {
    let mut lossless_array = [false; MAX_SEGMENTS];
    if let Some(segment_id) = lossless_segment {
        lossless_array[segment_id] = true;
    }
    TileCoeffFrameFacts::new(TileCoeffFrameFactsInput {
        enable_fsc,
        enable_idtx_intra: enable_fsc,
        enable_intra_ist: false,
        enable_inter_ist: false,
        enable_chroma_dctonly,
        enable_cctx: false,
        reduced_tx_set,
        lossless_array,
        allow_tcq,
        allow_parity_hiding,
        base_q_idx,
    })
}

fn block_facts() -> CoeffUseFscFrameBlockFacts {
    CoeffUseFscFrameBlockFacts {
        geometry: geometry(),
        plane_tx_type: IDTX,
        fsc_mode: true,
        is_inter: false,
        segment_id: 0,
    }
}

fn frame_nonzero_input(enable_fsc: bool, base_q_idx: u32) -> CoeffUseFscFrameFactsNonZeroInput {
    CoeffUseFscFrameFactsNonZeroInput {
        frame: frame_facts(enable_fsc, false, 0, false, false, base_q_idx, Some(0)),
        block: block_facts(),
        ordinary: ordinary_facts(),
    }
}

fn expected_base_q_input_with(
    enable_fsc: bool,
    base_q_idx: u32,
    block: CoeffUseFscFrameBlockFacts,
    ordinary_base_config: CoeffOrdinaryBranchLosslessBaseConfig,
    lossless: bool,
) -> CoeffUseFscBaseQFactsNonZeroInput {
    CoeffUseFscBaseQFactsNonZeroInput {
        facts: CoeffUseFscBaseQFacts {
            geometry: block.geometry,
            enable_fsc,
            plane_tx_type: block.plane_tx_type,
            fsc_mode: block.fsc_mode,
            is_inter: block.is_inter,
            base_q_idx,
        },
        ordinary_base_config,
        lossless,
    }
}

fn expected_base_q_input(enable_fsc: bool, base_q_idx: u32) -> CoeffUseFscBaseQFactsNonZeroInput {
    expected_base_q_input_with(
        enable_fsc,
        base_q_idx,
        block_facts(),
        ordinary_base_config(),
        true,
    )
}

fn run_base_q(payload: &[u8], input: CoeffUseFscBaseQFactsInput) -> Option<SelectorRun> {
    run_optional_branch(payload, |context, tile, symbols| {
        apply_coeff_use_fsc_branch_from_base_q_facts(context, tile, symbols, input).ok()
    })
}

fn run_frame_facts(payload: &[u8], input: CoeffUseFscFrameFactsInput) -> Option<SelectorRun> {
    run_optional_branch(payload, |context, tile, symbols| {
        apply_coeff_use_fsc_branch_from_frame_facts(context, tile, symbols, input).ok()
    })
}

fn ordinary_payload_from(first: u8, second: u8, suffix: [u8; 3]) -> [u8; 12] {
    [
        first, second, suffix[0], suffix[1], suffix[2], 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x80,
    ]
}

fn find_ordinary_payload_for(input: impl Fn() -> CoeffUseFscBaseQFactsNonZeroInput) -> [u8; 12] {
    for first in u8::MIN..=u8::MAX {
        for second in u8::MIN..=u8::MAX {
            for suffix in ORDINARY_PAYLOAD_SUFFIXES {
                let payload = ordinary_payload_from(first, second, suffix);
                if run_base_q(&payload, CoeffUseFscBaseQFactsInput::NonZero(input())).is_some() {
                    return payload;
                }
            }
        }
    }
    panic!("no ordinary coefficient frame-facts payload found");
}

fn find_ordinary_payload() -> [u8; 12] {
    find_ordinary_payload_for(|| expected_base_q_input(false, 91))
}

fn find_fsc_payload() -> [u8; 8] {
    for first in u8::MIN..=u8::MAX {
        for second in u8::MIN..=u8::MAX {
            for suffix in FSC_PAYLOAD_SUFFIXES {
                let payload = [
                    first, second, suffix[0], suffix[1], suffix[2], suffix[3], suffix[4], suffix[5],
                ];
                if run_base_q(
                    &payload,
                    CoeffUseFscBaseQFactsInput::NonZero(expected_base_q_input(true, 91)),
                )
                .is_some()
                {
                    return payload;
                }
            }
        }
    }
    panic!("no FSC coefficient frame-facts payload found");
}

fn assert_runs_eq(derived: &SelectorRun, expected: &SelectorRun) {
    assert_eq!(derived.0, expected.0);
    assert_eq!(derived.1, expected.1);
    assert_eq!(derived.2, expected.2);
    assert_eq!(derived.3, expected.3);
    assert_eq!(derived.4, expected.4);
}

#[test]
fn coefficient_frame_facts_nonzero_input_derives_lower_packet() {
    let lossless_array = [false; MAX_SEGMENTS];
    let input = CoeffUseFscFrameFactsNonZeroInput {
        frame: TileCoeffFrameFacts::new(TileCoeffFrameFactsInput {
            enable_fsc: true,
            enable_idtx_intra: true,
            enable_intra_ist: false,
            enable_inter_ist: false,
            enable_chroma_dctonly: true,
            enable_cctx: false,
            reduced_tx_set: 3,
            lossless_array,
            allow_tcq: true,
            allow_parity_hiding: true,
            base_q_idx: 141,
        }),
        block: CoeffUseFscFrameBlockFacts {
            geometry: geometry(),
            plane_tx_type: DCT_DCT,
            fsc_mode: false,
            is_inter: true,
            segment_id: 4,
        },
        ordinary: CoeffUseFscFrameOrdinaryFacts {
            uv_mode: 5,
            angle_delta_uv: -2,
            luma_tx_type: 6,
            chroma_inter_tx_type: 7,
        },
    };

    let derived = input.base_q_input().unwrap();

    assert_eq!(derived.facts.geometry, geometry());
    assert!(derived.facts.enable_fsc);
    assert_eq!(derived.facts.plane_tx_type, DCT_DCT);
    assert!(!derived.facts.fsc_mode);
    assert!(derived.facts.is_inter);
    assert_eq!(derived.facts.base_q_idx, 141);
    assert_eq!(derived.ordinary_base_config.reduced_tx_set, 3);
    assert!(derived.ordinary_base_config.enable_chroma_dctonly);
    assert_eq!(derived.ordinary_base_config.uv_mode, 5);
    assert_eq!(derived.ordinary_base_config.angle_delta_uv, -2);
    assert_eq!(derived.ordinary_base_config.luma_tx_type, 6);
    assert_eq!(derived.ordinary_base_config.chroma_inter_tx_type, 7);
    assert!(derived.ordinary_base_config.parity_hiding);
    assert!(derived.ordinary_base_config.use_tcq);
    assert!(!derived.lossless);
}

#[test]
fn coefficient_frame_facts_lossless_suppresses_parity_hiding_and_tcq() {
    let input = CoeffUseFscFrameFactsNonZeroInput {
        frame: frame_facts(false, false, 0, true, true, 91, Some(0)),
        block: CoeffUseFscFrameBlockFacts {
            geometry: geometry(),
            plane_tx_type: DCT_DCT,
            fsc_mode: false,
            is_inter: false,
            segment_id: 0,
        },
        ordinary: ordinary_facts(),
    };

    let derived = input.base_q_input().unwrap();

    assert!(!derived.ordinary_base_config.parity_hiding);
    assert!(!derived.ordinary_base_config.use_tcq);
    assert!(derived.lossless);
}

#[test]
fn coefficient_frame_facts_chroma_suppresses_parity_hiding_and_tcq() {
    let input = CoeffUseFscFrameFactsNonZeroInput {
        frame: frame_facts(false, false, 0, true, true, 91, None),
        block: CoeffUseFscFrameBlockFacts {
            geometry: CoeffOrdinaryTxSizeGeometryConfig {
                plane: 1,
                ..geometry()
            },
            plane_tx_type: DCT_DCT,
            fsc_mode: false,
            is_inter: false,
            segment_id: 0,
        },
        ordinary: ordinary_facts(),
    };

    let derived = input.base_q_input().unwrap();

    assert!(!derived.ordinary_base_config.parity_hiding);
    assert!(!derived.ordinary_base_config.use_tcq);
    assert!(!derived.lossless);
}

#[test]
fn coefficient_frame_facts_idtx_fsc_suppresses_parity_hiding_and_tcq() {
    let input = CoeffUseFscFrameFactsNonZeroInput {
        frame: frame_facts(true, false, 0, true, true, 91, None),
        block: CoeffUseFscFrameBlockFacts {
            geometry: geometry(),
            plane_tx_type: IDTX,
            fsc_mode: true,
            is_inter: false,
            segment_id: 0,
        },
        ordinary: ordinary_facts(),
    };

    let derived = input.base_q_input().unwrap();

    assert!(!derived.ordinary_base_config.parity_hiding);
    assert!(!derived.ordinary_base_config.use_tcq);
    assert!(!derived.lossless);
}

#[test]
fn coefficient_frame_facts_non_2d_suppresses_tcq_only() {
    let input = CoeffUseFscFrameFactsNonZeroInput {
        frame: frame_facts(false, false, 0, true, true, 91, None),
        block: CoeffUseFscFrameBlockFacts {
            geometry: geometry(),
            plane_tx_type: V_DCT,
            fsc_mode: false,
            is_inter: false,
            segment_id: 0,
        },
        ordinary: ordinary_facts(),
    };

    let derived = input.base_q_input().unwrap();

    assert!(derived.ordinary_base_config.parity_hiding);
    assert!(!derived.ordinary_base_config.use_tcq);
    assert!(!derived.lossless);
}

#[test]
fn coefficient_frame_facts_all_zero_matches_base_q_path() {
    let expected = run_base_q(&[0x80], CoeffUseFscBaseQFactsInput::AllZero(geometry())).unwrap();
    let derived =
        run_frame_facts(&[0x80], CoeffUseFscFrameFactsInput::AllZero(geometry())).unwrap();

    assert_runs_eq(&derived, &expected);
}

#[test]
fn coefficient_frame_facts_false_matches_base_q_ordinary_path() {
    let payload = find_ordinary_payload();
    let expected = run_base_q(
        &payload,
        CoeffUseFscBaseQFactsInput::NonZero(expected_base_q_input(false, 91)),
    )
    .unwrap();
    let derived = run_frame_facts(
        &payload,
        CoeffUseFscFrameFactsInput::NonZero(frame_nonzero_input(false, 91)),
    )
    .unwrap();

    assert_runs_eq(&derived, &expected);
}

#[test]
fn coefficient_frame_facts_parity_hiding_matches_explicit_base_q_path() {
    let block = CoeffUseFscFrameBlockFacts {
        geometry: geometry(),
        plane_tx_type: DCT_DCT,
        fsc_mode: false,
        is_inter: false,
        segment_id: 0,
    };
    let mut ordinary = ordinary_base_config();
    ordinary.parity_hiding = true;
    let payload =
        find_ordinary_payload_for(|| expected_base_q_input_with(false, 91, block, ordinary, false));
    let expected = run_base_q(
        &payload,
        CoeffUseFscBaseQFactsInput::NonZero(expected_base_q_input_with(
            false, 91, block, ordinary, false,
        )),
    )
    .unwrap();
    let derived = run_frame_facts(
        &payload,
        CoeffUseFscFrameFactsInput::NonZero(CoeffUseFscFrameFactsNonZeroInput {
            frame: frame_facts(false, false, 0, false, true, 91, None),
            block,
            ordinary: ordinary_facts(),
        }),
    )
    .unwrap();

    assert_runs_eq(&derived, &expected);
}

#[test]
fn coefficient_frame_facts_tcq_matches_explicit_base_q_path() {
    let block = CoeffUseFscFrameBlockFacts {
        geometry: geometry(),
        plane_tx_type: DCT_DCT,
        fsc_mode: false,
        is_inter: false,
        segment_id: 0,
    };
    let mut ordinary = ordinary_base_config();
    ordinary.use_tcq = true;
    let payload =
        find_ordinary_payload_for(|| expected_base_q_input_with(false, 91, block, ordinary, false));
    let expected = run_base_q(
        &payload,
        CoeffUseFscBaseQFactsInput::NonZero(expected_base_q_input_with(
            false, 91, block, ordinary, false,
        )),
    )
    .unwrap();
    let derived = run_frame_facts(
        &payload,
        CoeffUseFscFrameFactsInput::NonZero(CoeffUseFscFrameFactsNonZeroInput {
            frame: frame_facts(false, false, 0, true, false, 91, None),
            block,
            ordinary: ordinary_facts(),
        }),
    )
    .unwrap();

    assert_runs_eq(&derived, &expected);
}

#[test]
fn coefficient_frame_facts_true_matches_base_q_fsc_path() {
    let payload = find_fsc_payload();
    let expected = run_base_q(
        &payload,
        CoeffUseFscBaseQFactsInput::NonZero(expected_base_q_input(true, 91)),
    )
    .unwrap();
    let derived = run_frame_facts(
        &payload,
        CoeffUseFscFrameFactsInput::NonZero(frame_nonzero_input(true, 91)),
    )
    .unwrap();

    assert_runs_eq(&derived, &expected);
}

#[test]
fn coefficient_frame_facts_invalid_segment_id_is_fail_atomic() {
    let frame = FrameCdfSubset::from_defaults();
    let mut tile = frame.tile_copy();
    let mut symbols = symbol_decoder(&[0x80]);
    let mut context = seeded_context_state();
    let tile_before = tile.clone();
    let context_before = context.clone();
    let consumed_before = symbols.consumed_bits();
    let symbol_count_before = symbols.symbol_count();

    let mut input = frame_nonzero_input(false, 91);
    input.block.segment_id = MAX_SEGMENTS;
    let err = apply_coeff_use_fsc_branch_from_frame_facts(
        &mut context,
        &mut tile,
        &mut symbols,
        CoeffUseFscFrameFactsInput::NonZero(input),
    )
    .unwrap_err();

    assert!(matches!(
        err,
        CoeffUseFscBranchError::InvalidSegmentId {
            segment_id: MAX_SEGMENTS
        }
    ));
    assert_eq!(context, context_before);
    assert_eq!(tile, tile_before);
    assert_eq!(symbols.consumed_bits(), consumed_before);
    assert_eq!(symbols.symbol_count(), symbol_count_before);
}
