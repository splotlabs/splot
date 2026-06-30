// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use splot_recon::math::{clip3, round2_signed};

const REF_SCALE_SHIFT: u32 = 14;
const SUBPEL_BITS: u32 = 4;
const SCALE_SUBPEL_BITS: u32 = 10;
const MI_SIZE: i64 = 4;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::runtime_minimal) struct PlaneScaling {
    pub(in crate::runtime_minimal) start_x: i64,
    pub(in crate::runtime_minimal) start_y: i64,
    pub(in crate::runtime_minimal) step_x: i64,
    pub(in crate::runtime_minimal) step_y: i64,
    pub(in crate::runtime_minimal) first_x: i64,
    pub(in crate::runtime_minimal) first_y: i64,
    pub(in crate::runtime_minimal) last_x: i64,
    pub(in crate::runtime_minimal) last_y: i64,
}

#[allow(clippy::too_many_arguments)]
pub(in crate::runtime_minimal) fn derive_plane_scaling(
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
    let scale: i64 = 1 << REF_SCALE_SHIFT;
    let half_sample: i64 = 1 << (SUBPEL_BITS - 1);
    let off: i64 = (1 << (SCALE_SUBPEL_BITS - SUBPEL_BITS)) / 2;
    let round_shift = REF_SCALE_SHIFT + SUBPEL_BITS - SCALE_SUBPEL_BITS;

    let scaled_start = |plane_pos, mv_component, subsampling| {
        let orig = (plane_pos << SUBPEL_BITS) + ((2 * mv_component) >> subsampling) + half_sample;
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
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn zero_mv_luma_is_full_pel_origin() {
        let s = derive_plane_scaling(0, 0, 0, 0, 0, 0, 16, 16, 64, 64);
        assert_eq!(s.step_x, 1024);
        assert_eq!(s.step_y, 1024);
        assert_eq!(s.last_x, 63);
        assert_eq!(s.last_y, 63);
        assert_eq!((s.start_x >> 6) & 15, 0);
        assert_eq!((s.start_y >> 6) & 15, 0);
    }

    #[test]
    fn fractional_mv_produces_subpel_phase() {
        let s = derive_plane_scaling(0, 0, 0, 4, 0, 0, 16, 16, 64, 64);
        assert_ne!((s.start_x >> 6) & 15, 0, "horizontal sub-pel phase set");
        assert_eq!((s.start_y >> 6) & 15, 0, "vertical phase zero");
    }

    #[test]
    fn chroma_420_halves_dimensions() {
        let s = derive_plane_scaling(0, 0, 0, 0, 1, 1, 16, 16, 32, 32);
        assert_eq!(s.last_x, 31);
        assert_eq!(s.last_y, 31);
    }
}
