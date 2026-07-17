// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::unwrap_used)]

use super::*;

#[test]
fn zero_mv_luma_is_full_pel_origin() {
    let s = derive_plane_scaling(0, 0, 0, 0, 0, 0, 64, 64, 64, 64);
    assert_eq!(s.step_x, 1024);
    assert_eq!(s.step_y, 1024);
    assert_eq!(s.last_x, 63);
    assert_eq!(s.last_y, 63);
    assert_eq!((s.start_x >> 6) & 15, 0);
    assert_eq!((s.start_y >> 6) & 15, 0);
}

#[test]
fn fractional_mv_produces_subpel_phase() {
    let s = derive_plane_scaling(0, 0, 0, 4, 0, 0, 64, 64, 64, 64);
    assert_ne!((s.start_x >> 6) & 15, 0, "horizontal sub-pel phase set");
    assert_eq!((s.start_y >> 6) & 15, 0, "vertical phase zero");
}

#[test]
fn chroma_420_halves_dimensions() {
    let s = derive_plane_scaling(0, 0, 0, 0, 1, 1, 64, 64, 64, 64);
    assert_eq!(s.last_x, 31);
    assert_eq!(s.last_y, 31);
}

#[test]
fn scaled_reference_uses_spec_ratio_and_independent_axis_steps() {
    let s = derive_plane_scaling(0, 0, 0, 0, 0, 0, 64, 48, 51, 60);
    assert_eq!(s.scale_x, 20_560);
    assert_eq!(s.scale_y, 13_107);
    assert_eq!(s.step_x, 1_285);
    assert_eq!(s.step_y, 819);
    assert_eq!(s.start_x, 163);
    assert_eq!(s.last_x, 63);
    assert_eq!(s.last_y, 47);
    assert!(s.is_scaled());
}

#[test]
fn reference_scale_detection_uses_rounded_spec_scale() {
    assert!(!reference_is_scaled(64, 64, 64, 64));
    assert!(reference_is_scaled(64, 64, 51, 51));
    assert!(reference_is_scaled(51, 51, 64, 64));
}

#[test]
fn prescaled_sixteenth_pel_mv_matches_ordinary_eighth_pel_mv() {
    let ordinary = derive_plane_scaling(3, 5, -7, 11, 1, 1, 64, 64, 64, 64);
    let prescaled = ordinary.with_prescaled_mv(3, 5, -14, 22, 1, 1);
    assert_eq!(ordinary, prescaled);
}

#[test]
fn prescaled_chroma_mv_rounds_odd_components() {
    let ordinary = derive_plane_scaling(3, 5, -7, 11, 1, 1, 64, 64, 64, 64);
    let prescaled = ordinary.with_prescaled_mv(3, 5, -13, 21, 1, 1);
    assert_eq!(ordinary, prescaled);
}

#[test]
fn maximum_av2_geometry_and_mv_fit_scaling_state() {
    let scaling = derive_plane_scaling(
        65_535, 0, -65_535, 65_535, 0, 0, 65_536, 65_536, 65_536, 65_536,
    );
    let expected_start = |plane_pos: i64, mv: i64| {
        let half_sample = 1i64 << (SUBPEL_BITS - 1);
        let orig = (plane_pos << SUBPEL_BITS) + 2 * mv + half_sample;
        let base = orig * (1 << REF_SCALE_SHIFT) - (half_sample << REF_SCALE_SHIFT);
        round2_signed(base, REF_SCALE_SHIFT + SUBPEL_BITS - SCALE_SUBPEL_BITS)
            + (1 << (SCALE_SUBPEL_BITS - SUBPEL_BITS)) / 2
    };

    assert_eq!(i64::from(scaling.start_x), expected_start(65_535, 65_535));
    assert_eq!(i64::from(scaling.start_y), expected_start(0, -65_535));
    assert_eq!(scaling.last_x, 65_535);
    assert_eq!(scaling.last_y, 65_535);
}

#[test]
fn unscaled_fast_path_matches_general_equation() {
    let unit_scale = 1 << REF_SCALE_SHIFT;
    for prescaled in [false, true] {
        for subsampling in [0u32, 1] {
            for plane_pos in [0, 1, 127, 2047, 65_535] {
                for mv in [-65_535, -257, -1, 0, 1, 257, 65_535] {
                    let actual = derive_plane_scaling_from_scale(
                        plane_pos,
                        plane_pos,
                        mv,
                        mv,
                        subsampling,
                        subsampling,
                        unit_scale,
                        unit_scale,
                        65_535,
                        65_535,
                        prescaled,
                    );
                    let mv_offset = if prescaled {
                        round2_signed_i32(mv, subsampling)
                    } else {
                        (2 * mv) >> subsampling
                    };
                    let half_sample = 1i32 << (SUBPEL_BITS - 1);
                    let orig = (plane_pos << SUBPEL_BITS) + mv_offset + half_sample;
                    let base = i64::from(orig) * i64::from(unit_scale)
                        - i64::from(half_sample << REF_SCALE_SHIFT);
                    let expected = scaling_value(
                        round2_signed(base, REF_SCALE_SHIFT + SUBPEL_BITS - SCALE_SUBPEL_BITS) + 32,
                    );
                    assert_eq!(
                        (actual.start_x, actual.start_y),
                        (expected, expected),
                        "prescaled={prescaled} subsampling={subsampling} \
                         plane_pos={plane_pos} mv={mv}"
                    );
                }
            }
        }
    }
}
