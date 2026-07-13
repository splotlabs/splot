// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use splot_recon::math::{round2_signed, round2_signed_i32};

const REF_SCALE_SHIFT: u32 = 14;
const SUBPEL_BITS: u32 = 4;
const SCALE_SUBPEL_BITS: u32 = 10;
const MI_SIZE: i32 = 4;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PlaneScaling {
    pub(crate) start_x: i32,
    pub(crate) start_y: i32,
    pub(crate) step_x: i32,
    pub(crate) step_y: i32,
    pub(crate) first_x: i32,
    pub(crate) first_y: i32,
    pub(crate) last_x: i32,
    pub(crate) last_y: i32,
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn derive_plane_scaling(
    plane_x: i32,
    plane_y: i32,
    mv_row: i32,
    mv_col: i32,
    sub_x: u32,
    sub_y: u32,
    ref_mi_cols: i32,
    ref_mi_rows: i32,
    _block_w: i32,
    _block_h: i32,
) -> PlaneScaling {
    derive_plane_scaling_inner(
        plane_x,
        plane_y,
        mv_row,
        mv_col,
        sub_x,
        sub_y,
        ref_mi_cols,
        ref_mi_rows,
        false,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn derive_plane_scaling_prescaled(
    plane_x: i32,
    plane_y: i32,
    mv_row: i32,
    mv_col: i32,
    sub_x: u32,
    sub_y: u32,
    ref_mi_cols: i32,
    ref_mi_rows: i32,
) -> PlaneScaling {
    derive_plane_scaling_inner(
        plane_x,
        plane_y,
        mv_row,
        mv_col,
        sub_x,
        sub_y,
        ref_mi_cols,
        ref_mi_rows,
        true,
    )
}

#[allow(clippy::too_many_arguments)]
fn derive_plane_scaling_inner(
    plane_x: i32,
    plane_y: i32,
    mv_row: i32,
    mv_col: i32,
    sub_x: u32,
    sub_y: u32,
    ref_mi_cols: i32,
    ref_mi_rows: i32,
    prescaled: bool,
) -> PlaneScaling {
    let scale = 1i32 << REF_SCALE_SHIFT;
    let half_sample = 1i32 << (SUBPEL_BITS - 1);
    let off = (1i32 << (SCALE_SUBPEL_BITS - SUBPEL_BITS)) / 2;
    let round_shift = REF_SCALE_SHIFT + SUBPEL_BITS - SCALE_SUBPEL_BITS;

    let scaled_start = |plane_pos, mv_component, subsampling| {
        let mv_offset = if prescaled {
            round2_signed_i32(mv_component, subsampling)
        } else {
            (2 * mv_component) >> subsampling
        };
        let orig = (plane_pos << SUBPEL_BITS) + mv_offset + half_sample;
        let base = i64::from(orig) * i64::from(scale) - i64::from(half_sample << REF_SCALE_SHIFT);
        round2_signed(base, round_shift) + i64::from(off)
    };
    let last_bound = |ref_mi: i32, subsampling: u32| {
        let last = ((ref_mi * MI_SIZE) >> subsampling) - 1;
        last.max(0)
    };

    let step = round2_signed_i32(scale, REF_SCALE_SHIFT - SCALE_SUBPEL_BITS);

    PlaneScaling {
        start_x: scaling_value(scaled_start(plane_x, mv_col, sub_x)),
        start_y: scaling_value(scaled_start(plane_y, mv_row, sub_y)),
        step_x: step,
        step_y: step,
        first_x: 0,
        first_y: 0,
        last_x: last_bound(ref_mi_cols, sub_x),
        last_y: last_bound(ref_mi_rows, sub_y),
    }
}

fn scaling_value(value: i64) -> i32 {
    value.clamp(i32::MIN as i64, i32::MAX as i64) as i32
}

#[cfg(test)]
#[path = "mv_scaling_tests.rs"]
mod tests;
