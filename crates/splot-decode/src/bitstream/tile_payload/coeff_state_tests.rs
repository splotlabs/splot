// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::unwrap_used)]

use super::*;
use splot_parallel::{ThreadCount, WorkerPool};

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

fn reset(
    plane: usize,
    c: usize,
    r: usize,
    w4: usize,
    h4: usize,
    sub_x: u32,
    sub_y: u32,
) -> CoeffContextReset {
    CoeffContextReset {
        plane,
        c,
        r,
        w4,
        h4,
        sub_x,
        sub_y,
    }
}

#[test]
fn transform_block_state_is_zero_initialized_and_row_major() {
    let mut state = TransformCoeffBlockState::new(4, 3).unwrap();

    assert_eq!(state.width(), 4);
    assert_eq!(state.height(), 3);
    assert_eq!(state.level(), &[0; 12]);
    assert!(state.quant_sign.is_empty());
    assert_eq!(state.quant_sign(), &[0; 12]);
    assert_eq!(state.quant(), &[0; 12]);

    state.set_level(2, 1, 7).unwrap();
    state.set_quant_sign(2, 1, -1).unwrap();
    state.set_quant(9, -12).unwrap();

    assert!(!state.quant_sign.is_empty());
    assert_eq!(state.level_at(2, 1).unwrap(), 7);
    assert_eq!(state.quant_sign_at(2, 1).unwrap(), -1);
    assert_eq!(state.quant_at(9).unwrap(), -12);
    assert_eq!(state.level()[9], 7);
    assert_eq!(state.quant_sign()[9], -1);
    assert_eq!(state.quant()[9], -12);
}

#[test]
fn transform_block_fsc_state_allocates_zeroed_quant_sign() {
    let mut state = TransformCoeffBlockState::new(4, 3).unwrap();
    state.ensure_quant_sign().unwrap();

    assert_eq!(state.quant_sign, [0; 12]);
    assert_eq!(state.quant_sign(), &[0; 12]);
}

#[test]
fn maximum_transform_buffers_are_reused_and_zeroed() {
    clear_transform_coeff_buffers();
    let mut state = TransformCoeffBlockState::new(32, 32).unwrap();
    state.ensure_quant_sign().unwrap();
    let pointers = (state.level.as_ptr(), state.quant_sign.as_ptr());
    state.level.fill(7);
    state.quant_sign.fill(-1);
    state.quant.fill(9);
    let quant = state.into_quant();
    recycle_coeff_quant(quant);

    let mut reused = TransformCoeffBlockState::new(32, 32).unwrap();
    reused.ensure_quant_sign().unwrap();
    assert_eq!(reused.level.as_ptr(), pointers.0);
    assert_eq!(reused.quant_sign.as_ptr(), pointers.1);
    assert!(reused.level.iter().all(|value| *value == 0));
    assert!(reused.quant_sign.iter().all(|value| *value == 0));
    assert!(reused.quant.iter().all(|value| *value == 0));
}

#[test]
fn transform_state_errors_return_buffers_for_reuse() {
    clear_transform_coeff_buffers();
    let mut state = TransformCoeffBlockState::new(4, 4).unwrap();
    let level = state.level.as_ptr();
    assert!(state.set_quant(16, 1).is_err());
    drop(state);

    assert!(TransformCoeffBlockState::new(0, 4).is_err());
    let reused = TransformCoeffBlockState::new(4, 4).unwrap();
    assert_eq!(reused.level.as_ptr(), level);
}

#[test]
fn transform_block_scratch_remains_thread_local() {
    clear_transform_coeff_buffers();
    drop(TransformCoeffBlockState::new(4, 4).unwrap());
    assert_eq!(transform_coeff_buffer_counts(), (1, 0));

    let pool = WorkerPool::new(ThreadCount::Fixed(1.try_into().unwrap())).unwrap();
    pool.install(|| {
        assert_eq!(transform_coeff_buffer_counts(), (0, 0));
        drop(TransformCoeffBlockState::new(4, 4).unwrap());
        assert_eq!(transform_coeff_buffer_counts(), (1, 0));
    });

    assert_eq!(transform_coeff_buffer_counts(), (1, 0));
}

#[test]
fn quant_storage_recycles_from_worker_to_parser() {
    let buffers = SharedQuantBuffers::new(Vec::new());
    let quant = vec![7; 16];
    let pointer = quant.as_ptr();
    let pool = WorkerPool::new(ThreadCount::Fixed(1.try_into().unwrap())).unwrap();
    pool.install(|| recycle_coeff_quant_into(&buffers, quant));

    let reused = take_zeroed_quant_buffer_from(&buffers, 16).unwrap();
    assert_eq!(reused.as_ptr(), pointer);
    assert!(reused.iter().all(|value| *value == 0));
}

#[test]
fn transform_block_recycler_tolerates_reentrant_recycle() {
    clear_transform_coeff_buffers();
    with_reusable_scratch(&TRANSFORM_COEFF_BUFFERS, |outer| {
        recycle_buffer(&mut outer.levels, vec![0]);
        with_reusable_scratch(&TRANSFORM_COEFF_BUFFERS, |inner| {
            recycle_buffer(&mut inner.levels, vec![0]);
        });
        assert_eq!(outer.levels.len(), 1);
    });
    assert_eq!(transform_coeff_buffer_counts(), (1, 0));
}

#[test]
fn transform_block_recycler_has_a_retention_limit() {
    let buffers = SharedQuantBuffers::new(Vec::new());
    for _ in 0..=MAX_RETAINED_SHARED_QUANT_BUFFERS {
        recycle_coeff_quant_into(&buffers, vec![0]);
    }
    assert_eq!(
        lock_transform_coeff_quant_buffers(&buffers).len(),
        MAX_RETAINED_SHARED_QUANT_BUFFERS
    );

    let oversized = SharedQuantBuffers::new(Vec::new());
    recycle_coeff_quant_into(&oversized, Vec::new());
    recycle_coeff_quant_into(
        &oversized,
        Vec::with_capacity(MAX_RETAINED_COEFF_BUFFER_CAPACITY + 1),
    );
    assert!(lock_transform_coeff_quant_buffers(&oversized).is_empty());
}

#[test]
fn quant_recycler_retains_and_selects_the_largest_useful_buffers() {
    let buffers = SharedQuantBuffers::new(
        (0..MAX_RETAINED_SHARED_QUANT_BUFFERS)
            .map(|_| Vec::with_capacity(1))
            .collect(),
    );
    recycle_coeff_quant_into(&buffers, Vec::with_capacity(32));
    let retained = lock_transform_coeff_quant_buffers(&buffers);
    assert_eq!(retained.len(), MAX_RETAINED_SHARED_QUANT_BUFFERS);
    assert!(retained.iter().any(|buffer| buffer.capacity() >= 32));
    drop(retained);

    let reused = take_zeroed_quant_buffer_from(&buffers, 16).unwrap();
    assert!(reused.capacity() >= 32);
    assert_eq!(reused, [0; 16]);
}

#[test]
fn transform_block_state_rejects_invalid_extents_and_coordinates() {
    assert!(matches!(
        TransformCoeffBlockState::new(0, 4).unwrap_err(),
        TileCoeffStateError::InvalidAdjustedTransformExtent {
            axis: "width",
            value: 0
        }
    ));
    assert!(matches!(
        TransformCoeffBlockState::new(33, 4).unwrap_err(),
        TileCoeffStateError::InvalidAdjustedTransformExtent {
            axis: "width",
            value: 33
        }
    ));

    let mut state = TransformCoeffBlockState::new(4, 4).unwrap();
    assert!(matches!(
        state.level_at(4, 0).unwrap_err(),
        TileCoeffStateError::TransformCoordinateOutOfBounds {
            row: 4,
            col: 0,
            height: 4,
            width: 4
        }
    ));
    assert!(matches!(
        state.quant_at(16).unwrap_err(),
        TileCoeffStateError::QuantPositionOutOfBounds { pos: 16, len: 16 }
    ));
    assert!(matches!(
        state.set_quant_sign(4, 0, 1).unwrap_err(),
        TileCoeffStateError::TransformCoordinateOutOfBounds {
            row: 4,
            col: 0,
            height: 4,
            width: 4
        }
    ));
    assert!(state.quant_sign.is_empty());
}

#[test]
fn allocation_accounting_covers_transform_and_context_lines() {
    let block = TransformCoeffBlockState::allocation(32, 32).unwrap();
    assert_eq!(block.coeff_count(), 1024);

    let context = TileCoeffContextState::allocation(6, 8).unwrap();
    assert_eq!(context.above_len(), 8);
    assert_eq!(context.left_len(), 6);
    assert_eq!(context.total_entries(), 3 * (6 + 8) * 2);
}

#[test]
fn tile_context_state_initializes_three_zero_planes() {
    let state = TileCoeffContextState::new(2, 3).unwrap();

    assert_eq!(state.mi_rows(), 2);
    assert_eq!(state.mi_cols(), 3);
    for plane in 0..3 {
        assert_eq!(state.above_level(plane).unwrap(), &[0, 0, 0]);
        assert_eq!(state.left_level(plane).unwrap(), &[0, 0]);
        assert_eq!(state.above_dc(plane).unwrap(), &[0, 0, 0]);
        assert_eq!(state.left_dc(plane).unwrap(), &[0, 0]);
    }
}

#[test]
fn update_after_coeffs_writes_above_and_left_ranges_only() {
    let mut state = TileCoeffContextState::new(5, 6).unwrap();

    state.update_after_coeffs(update(0, 2, 1, 3, 2)).unwrap();

    assert_eq!(state.above_level(0).unwrap(), &[0, 0, 4, 4, 4, 0]);
    assert_eq!(state.above_dc(0).unwrap(), &[0, 0, 2, 2, 2, 0]);
    assert_eq!(state.left_level(0).unwrap(), &[0, 4, 4, 0, 0]);
    assert_eq!(state.left_dc(0).unwrap(), &[0, 2, 2, 0, 0]);
    assert_eq!(state.above_level(1).unwrap(), &[0, 0, 0, 0, 0, 0]);
    assert_eq!(state.left_dc(2).unwrap(), &[0, 0, 0, 0, 0]);
}

#[test]
fn chroma_context_updates_clip_to_subsampled_plane_edges() {
    let mut state = TileCoeffContextState::new_with_chroma_sampling(
        72,
        88,
        crate::tile::block_context::ChromaSampling::Yuv420,
    )
    .unwrap();

    state.update_after_coeffs(update(1, 32, 0, 16, 8)).unwrap();

    let above = state.above_level(1).unwrap();
    assert!(above[32..44].iter().all(|&value| value == 4));
    assert!(above[44..48].iter().all(|&value| value == 0));
    let left = state.left_level(1).unwrap();
    assert!(left[0..8].iter().all(|&value| value == 4));
    assert!(left[36..40].iter().all(|&value| value == 0));
}

#[test]
fn update_after_coeffs_rejects_bad_facts_without_mutation() {
    let mut state = TileCoeffContextState::new(2, 2).unwrap();
    let before = state.clone();

    assert!(matches!(
        state
            .update_after_coeffs(update(3, 0, 0, 1, 1))
            .unwrap_err(),
        TileCoeffStateError::InvalidPlane { plane: 3 }
    ));
    assert_eq!(state, before);

    assert!(matches!(
        state
            .update_after_coeffs(CoeffContextUpdate {
                dc_category: 3,
                ..update(0, 0, 0, 1, 1)
            })
            .unwrap_err(),
        TileCoeffStateError::InvalidDcCategory { dc_category: 3 }
    ));
    assert_eq!(state, before);

    assert!(matches!(
        state
            .update_after_coeffs(update(0, 2, 0, 1, 1))
            .unwrap_err(),
        TileCoeffStateError::ContextRangeOutOfBounds {
            context: "above",
            start: 2,
            end: 3,
            len: 2
        }
    ));
    assert_eq!(state, before);

    assert!(matches!(
        state
            .update_after_coeffs(update(0, 0, 2, 1, 1))
            .unwrap_err(),
        TileCoeffStateError::ContextRangeOutOfBounds {
            context: "left",
            start: 2,
            end: 3,
            len: 2
        }
    ));
    assert_eq!(state, before);
}

#[test]
fn update_after_coeffs_clamps_bottom_edge_overhang_to_on_tile_rows() {
    let mi_rows = 270;
    let mi_cols = 480;
    let mut state = TileCoeffContextState::new(mi_rows, mi_cols).unwrap();
    let y4 = mi_rows - 14;

    state
        .update_after_coeffs(CoeffContextUpdate {
            plane: 0,
            x4: 0,
            y4,
            w4: 16,
            h4: 16,
            cul_level: 3,
            dc_category: 2,
        })
        .unwrap();

    let left_level = state.left_level(0).unwrap();
    assert_eq!(left_level.len(), mi_rows);
    for (row, &value) in left_level.iter().enumerate() {
        let expected = if (y4..mi_rows).contains(&row) { 3 } else { 0 };
        assert_eq!(value, expected, "left_level[{row}]");
    }
    let left_dc = state.left_dc(0).unwrap();
    for (row, &value) in left_dc.iter().enumerate() {
        let expected = if (y4..mi_rows).contains(&row) { 2 } else { 0 };
        assert_eq!(value, expected, "left_dc[{row}]");
    }
    assert!(state.above_level(0).unwrap()[..16].iter().all(|&v| v == 3));
    assert_eq!(state.above_level(0).unwrap()[16], 0);
}

#[test]
fn update_after_coeffs_clamps_right_edge_overhang_to_on_tile_cols() {
    let mi_rows = 270;
    let mi_cols = 480;
    let mut state = TileCoeffContextState::new(mi_rows, mi_cols).unwrap();
    let x4 = mi_cols - 14;

    state
        .update_after_coeffs(CoeffContextUpdate {
            plane: 0,
            x4,
            y4: 0,
            w4: 16,
            h4: 16,
            cul_level: 4,
            dc_category: 1,
        })
        .unwrap();

    let above_level = state.above_level(0).unwrap();
    assert_eq!(above_level.len(), mi_cols);
    for (col, &value) in above_level.iter().enumerate() {
        let expected = if (x4..mi_cols).contains(&col) { 4 } else { 0 };
        assert_eq!(value, expected, "above_level[{col}]");
    }
    assert!(state.left_level(0).unwrap()[..16].iter().all(|&v| v == 4));
    assert_eq!(state.left_level(0).unwrap()[16], 0);
}

#[test]
fn reset_block_context_plane_zeros_subsampled_ranges() {
    let mut state = TileCoeffContextState::new(6, 8).unwrap();
    state
        .update_after_coeffs(CoeffContextUpdate {
            plane: 1,
            x4: 0,
            y4: 0,
            w4: 8,
            h4: 6,
            cul_level: 4,
            dc_category: 2,
        })
        .unwrap();

    state
        .reset_block_context_plane(reset(1, 2, 2, 4, 4, 1, 1))
        .unwrap();

    assert_eq!(state.above_level(1).unwrap(), &[4, 0, 0, 4, 4, 4, 4, 4]);
    assert_eq!(state.above_dc(1).unwrap(), &[2, 0, 0, 2, 2, 2, 2, 2]);
    assert_eq!(state.left_level(1).unwrap(), &[4, 0, 0, 4, 4, 4]);
    assert_eq!(state.left_dc(1).unwrap(), &[2, 0, 0, 2, 2, 2]);
}

#[test]
fn reset_block_context_plane_handles_empty_shifted_range() {
    let mut state = TileCoeffContextState::new(2, 2).unwrap();

    state
        .reset_block_context_plane(reset(2, 0, 0, 1, 1, 1, 1))
        .unwrap();

    assert_eq!(state.above_level(2).unwrap(), &[0, 0]);
    assert_eq!(state.left_dc(2).unwrap(), &[0, 0]);
}

#[test]
fn reset_block_context_plane_rejects_overflow_and_bad_subsampling() {
    let mut state = TileCoeffContextState::new(2, 2).unwrap();

    assert!(matches!(
        state
            .reset_block_context_plane(reset(0, usize::MAX, 0, 1, 1, 0, 0))
            .unwrap_err(),
        TileCoeffStateError::CoordinateOverflow {
            coordinate: "column",
            base: usize::MAX,
            offset: 1
        }
    ));
    assert!(matches!(
        state
            .reset_block_context_plane(reset(0, 0, 0, 1, 1, 2, 0))
            .unwrap_err(),
        TileCoeffStateError::InvalidSubsampling {
            axis: "x",
            value: 2
        }
    ));
}

#[test]
fn reset_block_context_plane_clamps_bottom_and_right_edge_overhang() {
    let mi_rows = 270;
    let mi_cols = 16;
    let mut state = TileCoeffContextState::new(mi_rows, mi_cols).unwrap();
    state.above_level[0].fill(7);
    state.above_dc[0].fill(3);
    state.left_level[0].fill(7);
    state.left_dc[0].fill(3);

    state
        .reset_block_context_plane(reset(0, 0, 256, 16, 16, 0, 0))
        .unwrap();

    assert!(state.above_level(0).unwrap().iter().all(|&v| v == 0));
    assert!(state.above_dc(0).unwrap().iter().all(|&v| v == 0));
    let left_level = state.left_level(0).unwrap();
    for (row, &value) in left_level.iter().enumerate() {
        let expected = if (256..mi_rows).contains(&row) { 0 } else { 7 };
        assert_eq!(value, expected, "left_level[{row}]");
    }
    let left_dc = state.left_dc(0).unwrap();
    for (row, &value) in left_dc.iter().enumerate() {
        let expected = if (256..mi_rows).contains(&row) { 0 } else { 3 };
        assert_eq!(value, expected, "left_dc[{row}]");
    }
}

#[test]
fn reset_block_context_plane_clamps_right_edge_only() {
    let mi_rows = 8;
    let mi_cols = 8;
    let mut state = TileCoeffContextState::new(mi_rows, mi_cols).unwrap();
    state.above_level[0].fill(5);
    state.above_dc[0].fill(1);

    state
        .reset_block_context_plane(reset(0, 6, 0, 4, 1, 0, 0))
        .unwrap();

    assert_eq!(state.above_level(0).unwrap(), &[5, 5, 5, 5, 5, 5, 0, 0]);
    assert_eq!(state.above_dc(0).unwrap(), &[1, 1, 1, 1, 1, 1, 0, 0]);
}

#[test]
fn reset_block_context_plane_rejects_out_of_frame_origin() {
    let mut state = TileCoeffContextState::new(4, 4).unwrap();
    assert!(matches!(
        state
            .reset_block_context_plane(reset(0, 4, 0, 1, 1, 0, 0))
            .unwrap_err(),
        TileCoeffStateError::ContextRangeOutOfBounds {
            context: "above reset",
            start: 4,
            len: 4,
            ..
        }
    ));
    assert!(matches!(
        state
            .reset_block_context_plane(reset(0, 0, 4, 1, 1, 0, 0))
            .unwrap_err(),
        TileCoeffStateError::ContextRangeOutOfBounds {
            context: "left reset",
            start: 4,
            len: 4,
            ..
        }
    ));
}

#[test]
fn rejects_empty_tile_context_dimensions_and_invalid_plane_views() {
    assert!(matches!(
        TileCoeffContextState::new(0, 1).unwrap_err(),
        TileCoeffStateError::EmptyTileDimensions {
            mi_rows: 0,
            mi_cols: 1
        }
    ));

    let state = TileCoeffContextState::new(1, 1).unwrap();
    assert!(matches!(
        state.above_dc(3).unwrap_err(),
        TileCoeffStateError::InvalidPlane { plane: 3 }
    ));
}
