// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Shared §7.13.3.18 `BILINEAR` IntrABC sub-pel parameter mapping, used by the
//! inter motion-compensation path and its unit test.

use crate::prediction::inter::mv_scaling::PlaneScaling;
use splot_recon::{BitDepth, InterpolationFilter, SubpelPredictParams};

pub(crate) fn intrabc_bilinear_params(
    scaling: PlaneScaling,
    w: usize,
    h: usize,
    bit_depth: BitDepth,
) -> SubpelPredictParams {
    SubpelPredictParams {
        interp: InterpolationFilter::Bilinear,
        w,
        h,
        start_x: scaling.start_x,
        start_y: scaling.start_y,
        step_x: scaling.step_x,
        step_y: scaling.step_y,
        first_x: scaling.first_x,
        first_y: scaling.first_y,
        last_x: scaling.last_x,
        last_y: scaling.last_y,
        bit_depth,
    }
}
