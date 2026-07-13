// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use splot_recon::math::{round2_signed, round2_signed_i32};

const REF_SCALE_SHIFT: u32 = 14;
const SUBPEL_BITS: u32 = 4;
const SCALE_SUBPEL_BITS: u32 = 10;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PlaneScaling {
    pub(crate) start_x: i32,
    pub(crate) start_y: i32,
    pub(crate) step_x: i32,
    pub(crate) step_y: i32,
    pub(crate) scale_x: i32,
    pub(crate) scale_y: i32,
    pub(crate) first_x: i32,
    pub(crate) first_y: i32,
    pub(crate) last_x: i32,
    pub(crate) last_y: i32,
}

impl PlaneScaling {
    pub(crate) const fn is_scaled(self) -> bool {
        self.scale_x != 1 << REF_SCALE_SHIFT || self.scale_y != 1 << REF_SCALE_SHIFT
    }

    pub(crate) fn with_prescaled_mv(
        self,
        plane_x: i32,
        plane_y: i32,
        mv_row: i32,
        mv_col: i32,
        sub_x: u32,
        sub_y: u32,
    ) -> Self {
        self.with_mv_precision(plane_x, plane_y, mv_row, mv_col, sub_x, sub_y, true)
    }

    pub(crate) fn with_mv(
        self,
        plane_x: i32,
        plane_y: i32,
        mv_row: i32,
        mv_col: i32,
        sub_x: u32,
        sub_y: u32,
    ) -> Self {
        self.with_mv_precision(plane_x, plane_y, mv_row, mv_col, sub_x, sub_y, false)
    }

    #[allow(clippy::too_many_arguments)]
    fn with_mv_precision(
        self,
        plane_x: i32,
        plane_y: i32,
        mv_row: i32,
        mv_col: i32,
        sub_x: u32,
        sub_y: u32,
        prescaled: bool,
    ) -> Self {
        derive_plane_scaling_from_scale(
            plane_x,
            plane_y,
            mv_row,
            mv_col,
            sub_x,
            sub_y,
            self.scale_x,
            self.scale_y,
            self.last_x,
            self.last_y,
            prescaled,
        )
    }
}

pub(crate) fn reference_is_scaled(
    reference_width: i32,
    reference_height: i32,
    frame_width: i32,
    frame_height: i32,
) -> bool {
    scale_dimension(reference_width, frame_width) != 1 << REF_SCALE_SHIFT
        || scale_dimension(reference_height, frame_height) != 1 << REF_SCALE_SHIFT
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn derive_plane_scaling(
    plane_x: i32,
    plane_y: i32,
    mv_row: i32,
    mv_col: i32,
    sub_x: u32,
    sub_y: u32,
    reference_width: i32,
    reference_height: i32,
    frame_width: i32,
    frame_height: i32,
) -> PlaneScaling {
    let scale_x = scale_dimension(reference_width, frame_width);
    let scale_y = scale_dimension(reference_height, frame_height);
    let plane_bound = |reference_dimension: i32, subsampling: u32| {
        let sample_scale = 1 << subsampling;
        ((reference_dimension + sample_scale - 1) / sample_scale - 1).max(0)
    };
    derive_plane_scaling_from_scale(
        plane_x,
        plane_y,
        mv_row,
        mv_col,
        sub_x,
        sub_y,
        scale_x,
        scale_y,
        plane_bound(reference_width, sub_x),
        plane_bound(reference_height, sub_y),
        false,
    )
}

#[allow(clippy::too_many_arguments)]
fn derive_plane_scaling_from_scale(
    plane_x: i32,
    plane_y: i32,
    mv_row: i32,
    mv_col: i32,
    sub_x: u32,
    sub_y: u32,
    scale_x: i32,
    scale_y: i32,
    last_x: i32,
    last_y: i32,
    prescaled: bool,
) -> PlaneScaling {
    let half_sample = 1i32 << (SUBPEL_BITS - 1);
    let off = (1i32 << (SCALE_SUBPEL_BITS - SUBPEL_BITS)) / 2;
    let round_shift = REF_SCALE_SHIFT + SUBPEL_BITS - SCALE_SUBPEL_BITS;

    let scaled_start = |plane_pos, mv_component, subsampling, scale| {
        let mv_offset = if prescaled {
            round2_signed_i32(mv_component, subsampling)
        } else {
            (2 * mv_component) >> subsampling
        };
        let orig = (plane_pos << SUBPEL_BITS) + mv_offset + half_sample;
        let base = i64::from(orig) * i64::from(scale) - i64::from(half_sample << REF_SCALE_SHIFT);
        scaling_value(round2_signed(base, round_shift) + i64::from(off))
    };
    PlaneScaling {
        start_x: scaled_start(plane_x, mv_col, sub_x, scale_x),
        start_y: scaled_start(plane_y, mv_row, sub_y, scale_y),
        step_x: round2_signed_i32(scale_x, REF_SCALE_SHIFT - SCALE_SUBPEL_BITS),
        step_y: round2_signed_i32(scale_y, REF_SCALE_SHIFT - SCALE_SUBPEL_BITS),
        scale_x,
        scale_y,
        first_x: 0,
        first_y: 0,
        last_x,
        last_y,
    }
}

fn scale_dimension(reference: i32, current: i32) -> i32 {
    let numerator = (i64::from(reference) << REF_SCALE_SHIFT) + i64::from(current) / 2;
    scaling_value(numerator / i64::from(current))
}

fn scaling_value(value: i64) -> i32 {
    value.clamp(i32::MIN as i64, i32::MAX as i64) as i32
}

#[cfg(test)]
#[path = "mv_scaling_tests.rs"]
mod tests;
