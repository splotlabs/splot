// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::unwrap_used)]
use super::super::coeff_state::{CoeffContextUpdate, TileCoeffContextState};
use super::*;

fn update(plane: usize, x4: usize, y4: usize, w4: usize, h4: usize) -> CoeffContextUpdate {
    CoeffContextUpdate {
        plane,
        x4,
        y4,
        w4,
        h4,
        cul_level: 4,
        dc_category: 2,
    }
}

#[test]
fn luma_all_zero_context_reads_zero_state_for_first_block() {
    let state = TileCoeffContextState::new(16, 16).unwrap();
    let ctx = luma_all_zero_context(
        &state,
        LumaAllZeroContextInput {
            x4: 0,
            y4: 0,
            w4: 16,
            h4: 16,
            tx_fills_block: true,
            fsc_active: false,
        },
    )
    .unwrap();

    assert_eq!(ctx, 0);
}

#[test]
fn luma_all_zero_context_reduces_state_lines_when_not_filling() {
    let mut state = TileCoeffContextState::new(8, 8).unwrap();
    state.update_after_coeffs(update(0, 2, 3, 2, 2)).unwrap();
    let ctx = luma_all_zero_context(
        &state,
        LumaAllZeroContextInput {
            x4: 1,
            y4: 2,
            w4: 4,
            h4: 4,
            tx_fills_block: false,
            fsc_active: false,
        },
    )
    .unwrap();

    assert_eq!(ctx, 5);
}

#[test]
fn luma_all_zero_context_fsc_overrides_state() {
    let mut state = TileCoeffContextState::new(4, 4).unwrap();
    state.update_after_coeffs(update(0, 0, 0, 4, 4)).unwrap();
    let ctx = luma_all_zero_context(
        &state,
        LumaAllZeroContextInput {
            x4: 0,
            y4: 0,
            w4: 4,
            h4: 4,
            tx_fills_block: true,
            fsc_active: true,
        },
    )
    .unwrap();

    assert_eq!(ctx, 9);
}

#[test]
fn v_all_zero_context_combines_level_dc_state_and_geometry() {
    let mut state = TileCoeffContextState::new(8, 8).unwrap();
    state.update_after_coeffs(update(2, 2, 5, 2, 1)).unwrap();
    let ctx = v_all_zero_context(
        &state,
        VAllZeroContextInput {
            x4: 1,
            y4: 4,
            w4: 4,
            h4: 3,
            chroma_block_larger_than_tx: true,
            eob_u_nonzero: true,
        },
    )
    .unwrap();

    assert_eq!(ctx, 11);
}

#[test]
fn all_zero_context_reductions_bound_out_of_range_and_pathological_counts() {
    let mut state = TileCoeffContextState::new(2, 2).unwrap();
    state.update_after_coeffs(update(2, 0, 0, 1, 1)).unwrap();

    let luma = luma_all_zero_context(
        &state,
        LumaAllZeroContextInput {
            x4: usize::MAX,
            y4: usize::MAX,
            w4: usize::MAX,
            h4: usize::MAX,
            tx_fills_block: false,
            fsc_active: false,
        },
    )
    .unwrap();
    let v = v_all_zero_context(
        &state,
        VAllZeroContextInput {
            x4: usize::MAX,
            y4: usize::MAX,
            w4: usize::MAX,
            h4: usize::MAX,
            chroma_block_larger_than_tx: false,
            eob_u_nonzero: false,
        },
    )
    .unwrap();

    assert_eq!(luma, 1);
    assert_eq!(v, 0);
}

#[test]
fn all_zero_coeff_block_applies_zero_state_and_context_writes() {
    let mut state = TileCoeffContextState::new(6, 6).unwrap();
    state.update_after_coeffs(update(0, 1, 2, 3, 2)).unwrap();

    let applied = apply_all_zero_coeff_block(
        &mut state,
        AllZeroCoeffBlockInput {
            plane: 0,
            x4: 1,
            y4: 2,
            w4: 3,
            h4: 2,
        },
    )
    .unwrap();

    assert_eq!(applied.eob(), 0);
    assert_eq!(applied.cul_level(), 0);
    assert_eq!(applied.dc_category(), 0);
    assert_eq!(applied.block().width(), 12);
    assert_eq!(applied.block().height(), 8);
    assert!(applied.block().level().iter().all(|level| *level == 0));
    assert!(applied.block().quant_sign().iter().all(|sign| *sign == 0));
    assert!(applied.block().quant().iter().all(|quant| *quant == 0));
    assert_eq!(state.above_level(0).unwrap(), &[0, 0, 0, 0, 0, 0]);
    assert_eq!(state.above_dc(0).unwrap(), &[0, 0, 0, 0, 0, 0]);
    assert_eq!(state.left_level(0).unwrap(), &[0, 0, 0, 0, 0, 0]);
    assert_eq!(state.left_dc(0).unwrap(), &[0, 0, 0, 0, 0, 0]);
}

#[test]
fn all_zero_coeff_block_rejects_bad_ranges_without_mutation() {
    let mut state = TileCoeffContextState::new(2, 2).unwrap();
    state.update_after_coeffs(update(0, 0, 0, 1, 1)).unwrap();
    let before = state.clone();

    let err = apply_all_zero_coeff_block(
        &mut state,
        AllZeroCoeffBlockInput {
            plane: 0,
            x4: 2,
            y4: 0,
            w4: 1,
            h4: 1,
        },
    )
    .unwrap_err();

    assert!(matches!(
        err,
        CoeffLoopContextError::State(TileCoeffStateError::ContextRangeOutOfBounds {
            context: "above",
            start: 2,
            end: 3,
            len: 2
        })
    ));
    assert_eq!(state, before);
}

#[test]
fn all_zero_coeff_block_rejects_zero_transform_extent_before_mutation() {
    let mut state = TileCoeffContextState::new(2, 2).unwrap();
    state.update_after_coeffs(update(0, 0, 0, 1, 1)).unwrap();
    let before = state.clone();

    let err = apply_all_zero_coeff_block(
        &mut state,
        AllZeroCoeffBlockInput {
            plane: 0,
            x4: 0,
            y4: 0,
            w4: 0,
            h4: 1,
        },
    )
    .unwrap_err();

    assert!(matches!(
        err,
        CoeffLoopContextError::State(TileCoeffStateError::InvalidAdjustedTransformExtent {
            axis: "width",
            value: 0
        })
    ));
    assert_eq!(state, before);
}

#[test]
fn all_zero_coeff_block_saturates_adjusted_extent_to_spec_cap() {
    let mut state = TileCoeffContextState::new(16, 16).unwrap();

    let applied = apply_all_zero_coeff_block(
        &mut state,
        AllZeroCoeffBlockInput {
            plane: 2,
            x4: 0,
            y4: 0,
            w4: 16,
            h4: 16,
        },
    )
    .unwrap();

    assert_eq!(applied.block().width(), 32);
    assert_eq!(applied.block().height(), 32);
    assert_eq!(applied.block().quant().len(), 1024);
}

#[test]
fn nonzero_coeff_eob_maps_small_points_without_refinements() {
    let eob_one = nonzero_coeff_eob(NonZeroCoeffEobInput {
        eob_pt: 1,
        eob_extra: false,
        eob_extra_bits: 0,
    })
    .unwrap();
    let eob_two = nonzero_coeff_eob(NonZeroCoeffEobInput {
        eob_pt: 2,
        eob_extra: false,
        eob_extra_bits: 0,
    })
    .unwrap();

    assert_eq!(eob_one.eob_pt(), 1);
    assert_eq!(eob_one.eob(), 1);
    assert_eq!(eob_two.eob_pt(), 2);
    assert_eq!(eob_two.eob(), 2);
}

#[test]
fn nonzero_coeff_eob_applies_eob_extra_and_refinement_bits() {
    assert_nonzero_coeff_eob(
        NonZeroCoeffEobInput {
            eob_pt: 6,
            eob_extra: true,
            eob_extra_bits: 0b110,
        },
        31,
    );
}

#[test]
fn nonzero_coeff_eob_reaches_max_av2_eob() {
    assert_nonzero_coeff_eob(
        NonZeroCoeffEobInput {
            eob_pt: 11,
            eob_extra: true,
            eob_extra_bits: 0xFF,
        },
        1024,
    );
}

fn assert_nonzero_coeff_eob(input: NonZeroCoeffEobInput, expected: usize) {
    let eob = nonzero_coeff_eob(input).unwrap();

    assert_eq!(eob.eob(), expected);
}

#[test]
fn nonzero_coeff_eob_rejects_invalid_eob_points() {
    let zero = nonzero_coeff_eob(NonZeroCoeffEobInput {
        eob_pt: 0,
        eob_extra: false,
        eob_extra_bits: 0,
    })
    .unwrap_err();
    let oversized = nonzero_coeff_eob(NonZeroCoeffEobInput {
        eob_pt: 12,
        eob_extra: false,
        eob_extra_bits: 0,
    })
    .unwrap_err();

    assert!(matches!(
        zero,
        CoeffLoopContextError::InvalidEobPoint { eob_pt: 0 }
    ));
    assert!(matches!(
        oversized,
        CoeffLoopContextError::InvalidEobPoint { eob_pt: 12 }
    ));
}

#[test]
fn nonzero_coeff_eob_rejects_refinements_for_small_points() {
    let err = nonzero_coeff_eob(NonZeroCoeffEobInput {
        eob_pt: 1,
        eob_extra: true,
        eob_extra_bits: 0,
    })
    .unwrap_err();

    assert!(matches!(
        err,
        CoeffLoopContextError::UnexpectedEobRefinement {
            eob_pt: 1,
            eob_extra: true,
            eob_extra_bits: 0
        }
    ));
}

#[test]
fn nonzero_coeff_eob_rejects_out_of_range_refinement_bits() {
    let err = nonzero_coeff_eob(NonZeroCoeffEobInput {
        eob_pt: 4,
        eob_extra: false,
        eob_extra_bits: 0b10,
    })
    .unwrap_err();

    assert!(matches!(
        err,
        CoeffLoopContextError::EobExtraBitsOutOfRange {
            eob_pt: 4,
            eob_extra_bits: 2,
            max_eob_extra_bits: 1
        }
    ));
}
