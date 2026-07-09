// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use splot_recon::math::{clip3, round2_signed};

const REF_SCALE_SHIFT: u32 = 14;
const SUBPEL_BITS: u32 = 4;
const SCALE_SUBPEL_BITS: u32 = 10;
const MI_SIZE: i64 = 4;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PlaneScaling {
    pub(crate) start_x: i64,
    pub(crate) start_y: i64,
    pub(crate) step_x: i64,
    pub(crate) step_y: i64,
    pub(crate) first_x: i64,
    pub(crate) first_y: i64,
    pub(crate) last_x: i64,
    pub(crate) last_y: i64,
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn derive_plane_scaling(
    plane_x: i64,
    plane_y: i64,
    mv_row: i64,
    mv_col: i64,
    sub_x: u32,
    sub_y: u32,
    ref_mi_cols: i64,
    ref_mi_rows: i64,
    _block_w: i64,
    _block_h: i64,
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
    plane_x: i64,
    plane_y: i64,
    mv_row: i64,
    mv_col: i64,
    sub_x: u32,
    sub_y: u32,
    ref_mi_cols: i64,
    ref_mi_rows: i64,
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
    plane_x: i64,
    plane_y: i64,
    mv_row: i64,
    mv_col: i64,
    sub_x: u32,
    sub_y: u32,
    ref_mi_cols: i64,
    ref_mi_rows: i64,
    prescaled: bool,
) -> PlaneScaling {
    let scale: i64 = 1 << REF_SCALE_SHIFT;
    let half_sample: i64 = 1 << (SUBPEL_BITS - 1);
    let off: i64 = (1 << (SCALE_SUBPEL_BITS - SUBPEL_BITS)) / 2;
    let round_shift = REF_SCALE_SHIFT + SUBPEL_BITS - SCALE_SUBPEL_BITS;

    let scaled_start = |plane_pos, mv_component, subsampling| {
        let mv_offset = if prescaled {
            round2_signed(mv_component, subsampling)
        } else {
            (2 * mv_component) >> subsampling
        };
        let orig = (plane_pos << SUBPEL_BITS) + mv_offset + half_sample;
        let base = orig * scale - (half_sample << REF_SCALE_SHIFT);
        round2_signed(base, round_shift) + off
    };
    let last_bound = |ref_mi, subsampling| {
        let last = ((ref_mi * MI_SIZE) >> subsampling) - 1;
        clip3(0, last, last)
    };

    let step = round2_signed(scale, REF_SCALE_SHIFT - SCALE_SUBPEL_BITS);

    PlaneScaling {
        start_x: scaled_start(plane_x, mv_col, sub_x),
        start_y: scaled_start(plane_y, mv_row, sub_y),
        step_x: step,
        step_y: step,
        first_x: 0,
        first_y: 0,
        last_x: last_bound(ref_mi_cols, sub_x),
        last_y: last_bound(ref_mi_rows, sub_y),
    }
}

#[cfg(test)]
#[path = "mv_scaling_tests.rs"]
mod tests;
