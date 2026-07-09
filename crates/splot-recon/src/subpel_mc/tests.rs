// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Unit tests for the AV2 § 7.13.3.18 sub-pel motion-compensation kernel,
//! anchored to spec-derived worked examples plus an independent in-test re-trace
//! of the § 7.13.3.18 pseudocode.

use super::*;
use crate::error::ReconError;

/// Builds a reference view from a fresh `width * height` row-major buffer.
fn build_ref(samples: Vec<u16>, width: usize, height: usize) -> Vec<u16> {
    assert_eq!(samples.len(), width * height);
    samples
}

/// Default single-reference 8-bit params for a `w x h` block sampled from
/// reference origin `(rx, ry)` in whole samples (full-pel) with the given filter.
fn full_pel_params(
    interp: InterpolationFilter,
    w: usize,
    h: usize,
    rx: i64,
    ry: i64,
    ref_w: i64,
    ref_h: i64,
) -> SubpelPredictParams {
    SubpelPredictParams {
        interp,
        w,
        h,
        start_x: rx << SCALE_SUBPEL_BITS,
        start_y: ry << SCALE_SUBPEL_BITS,
        step_x: 1 << SCALE_SUBPEL_BITS,
        step_y: 1 << SCALE_SUBPEL_BITS,
        first_x: 0,
        first_y: 0,
        last_x: ref_w - 1,
        last_y: ref_h - 1,
        bit_depth: BitDepth::Eight,
    }
}

/// An independent in-test re-trace of the AV2 § 7.13.3.18 single-reference
/// (non-compound) two-pass convolution, used as the property-test oracle. The
/// explicit `for t in 0..8` tap loops mirror the spec pseudocode index variable.
#[allow(
    clippy::too_many_arguments,
    clippy::needless_range_loop,
    clippy::many_single_char_names
)]
fn reference_subpel(
    samples: &[u16],
    ref_w: usize,
    ref_h: usize,
    params: &SubpelPredictParams,
) -> Vec<u16> {
    let w = params.w;
    let h = params.h;
    let x = params.start_x;
    let y = params.start_y;
    let x_step = params.step_x;
    let y_step = params.step_y;
    let first_x = params.first_x;
    let first_y = params.first_y;
    let last_x = params.last_x;
    let last_y = params.last_y;
    let max_sample = i64::from(params.bit_depth.max_sample());

    let intermediate_height = ((((h as i64 - 1) * y_step + (1 << 10) - 1) >> 10) + 8) as usize;

    let clip3 = |lo: i64, hi: i64, v: i64| -> i64 {
        if v < lo {
            lo
        } else if v > hi {
            hi
        } else {
            v
        }
    };
    let round2 = |v: i64, n: u32| -> i64 { if n == 0 { v } else { (v + (1 << (n - 1))) >> n } };
    let fetch = |row: i64, col: i64| -> i64 {
        let row = clip3(0, ref_h as i64 - 1, row) as usize;
        let col = clip3(0, ref_w as i64 - 1, col) as usize;
        i64::from(samples[row * ref_w + col])
    };

    let mut h_filter = match params.interp {
        InterpolationFilter::EightTap => 0,
        InterpolationFilter::EightTapSmooth => 1,
        InterpolationFilter::EightTapSharp => 2,
        InterpolationFilter::Bilinear => 3,
    };
    if w <= 4 {
        h_filter = match params.interp {
            InterpolationFilter::EightTap | InterpolationFilter::EightTapSharp => 4,
            InterpolationFilter::EightTapSmooth => 5,
            InterpolationFilter::Bilinear => 3,
        };
    }
    let mut intermediate = vec![0i64; intermediate_height * w];
    for r in 0..intermediate_height {
        for c in 0..w {
            let p = x + x_step * c as i64;
            let mut s = 0i64;
            for t in 0..8 {
                let phase = ((p >> 6) & 15) as usize;
                let tap = i64::from(SUBPEL_FILTERS[h_filter][phase][t]);
                let rr = clip3(first_y, last_y, (y >> 10) + r as i64 - 3);
                let cc = clip3(first_x, last_x, (p >> 10) + t as i64 - 3);
                s += tap * fetch(rr, cc);
            }
            intermediate[r * w + c] = round2(s, 3);
        }
    }

    let mut v_filter = match params.interp {
        InterpolationFilter::EightTap => 0,
        InterpolationFilter::EightTapSmooth => 1,
        InterpolationFilter::EightTapSharp => 2,
        InterpolationFilter::Bilinear => 3,
    };
    if h <= 4 {
        v_filter = match params.interp {
            InterpolationFilter::EightTap | InterpolationFilter::EightTapSharp => 4,
            InterpolationFilter::EightTapSmooth => 5,
            InterpolationFilter::Bilinear => 3,
        };
    }
    let mut out = vec![0u16; w * h];
    for r in 0..h {
        for c in 0..w {
            let p = (y & 1023) + y_step * r as i64;
            let mut s = 0i64;
            for t in 0..8 {
                let phase = ((p >> 6) & 15) as usize;
                let tap = i64::from(SUBPEL_FILTERS[v_filter][phase][t]);
                let row = ((p >> 10) + t as i64) as usize;
                s += tap * intermediate[row * w + c];
            }
            let pred = round2(s, 11);
            out[r * w + c] = clip3(0, max_sample, pred) as u16;
        }
    }
    out
}

#[test]
fn subpel_filters_table_shape_and_sums() {
    assert_eq!(SUBPEL_FILTERS.len(), 6);
    for (fi, filter) in SUBPEL_FILTERS.iter().enumerate() {
        assert_eq!(filter.len(), 16, "filter {fi} phase count");
        for (pi, phase) in filter.iter().enumerate() {
            let sum: i32 = phase.iter().sum();
            assert_eq!(sum, 128, "filter {fi} phase {pi} sum");
            for &tap in phase {
                assert_eq!(tap % 2, 0, "filter {fi} phase {pi} tap parity");
            }
        }
        assert_eq!(filter[0], [0, 0, 0, 128, 0, 0, 0, 0], "filter {fi} phase 0");
    }
}

#[test]
fn subpel_filters_first_table_rows_verbatim() {
    assert_eq!(SUBPEL_FILTERS[0][8], [0, 2, -14, 76, 76, -14, 2, 0]); // EIGHTTAP half-pel
    assert_eq!(SUBPEL_FILTERS[1][8], [0, -2, 14, 52, 52, 14, -2, 0]); // SMOOTH half-pel
    assert_eq!(SUBPEL_FILTERS[2][8], [-4, 12, -24, 80, 80, -24, 12, -4]); // SHARP half-pel
    assert_eq!(SUBPEL_FILTERS[3][8], [0, 0, 0, 64, 64, 0, 0, 0]); // bilinear half-pel
    assert_eq!(SUBPEL_FILTERS[4][8], [0, 0, -12, 76, 76, -12, 0, 0]); // 4-tap EIGHTTAP half
    assert_eq!(SUBPEL_FILTERS[5][8], [0, 0, 12, 52, 52, 12, 0, 0]); // 4-tap SMOOTH half
}

#[test]
fn full_pel_zero_fraction_is_exact_copy() {
    let ref_w = 16usize;
    let ref_h = 16usize;
    let samples: Vec<u16> = (0..(ref_w * ref_h) as u16).collect();
    let samples = build_ref(samples, ref_w, ref_h);
    let view = ReferencePlaneView::new(&samples, ref_w, ref_h).unwrap();

    let w = 8usize;
    let h = 8usize;
    let params = full_pel_params(
        InterpolationFilter::EightTap,
        w,
        h,
        2,
        3,
        ref_w as i64,
        ref_h as i64,
    );
    let out = subpel_predict_block(&view, &params).unwrap();

    for r in 0..h {
        for c in 0..w {
            let expected = samples[(3 + r) * ref_w + (2 + c)];
            assert_eq!(out[r * w + c], expected, "({r},{c})");
        }
    }
}

#[test]
fn full_pel_flat_reference_returns_flat() {
    let ref_w = 12usize;
    let ref_h = 12usize;
    let samples = build_ref(vec![100u16; ref_w * ref_h], ref_w, ref_h);
    let view = ReferencePlaneView::new(&samples, ref_w, ref_h).unwrap();

    let params = SubpelPredictParams {
        interp: InterpolationFilter::EightTapSharp,
        w: 4,
        h: 4,
        start_x: (3 << SCALE_SUBPEL_BITS) + (8 << 6), // phase 8 (half-pel) at sample 3
        start_y: (3 << SCALE_SUBPEL_BITS) + (8 << 6),
        step_x: 1 << SCALE_SUBPEL_BITS,
        step_y: 1 << SCALE_SUBPEL_BITS,
        first_x: 0,
        first_y: 0,
        last_x: ref_w as i64 - 1,
        last_y: ref_h as i64 - 1,
        bit_depth: BitDepth::Eight,
    };
    let out = subpel_predict_block(&view, &params).unwrap();
    assert!(
        out.iter().all(|&s| s == 100),
        "flat in -> flat out: {out:?}"
    );
}

#[test]
fn half_pel_horizontal_worked_example() {
    let ref_w = 16usize;
    let ref_h = 16usize;
    let mut samples = vec![0u16; ref_w * ref_h];
    for r in 0..ref_h {
        for c in 0..ref_w {
            samples[r * ref_w + c] = (10 * (c as u16 + 1)).min(255);
        }
    }
    let samples = build_ref(samples, ref_w, ref_h);
    let view = ReferencePlaneView::new(&samples, ref_w, ref_h).unwrap();

    let params = SubpelPredictParams {
        interp: InterpolationFilter::EightTapSharp,
        w: 8,
        h: 8,
        start_x: (4 << SCALE_SUBPEL_BITS) + (8 << 6), // sample 4, phase 8 (half-pel)
        start_y: 5 << SCALE_SUBPEL_BITS,              // full-pel row 5
        step_x: 1 << SCALE_SUBPEL_BITS,
        step_y: 1 << SCALE_SUBPEL_BITS,
        first_x: 0,
        first_y: 0,
        last_x: ref_w as i64 - 1,
        last_y: ref_h as i64 - 1,
        bit_depth: BitDepth::Eight,
    };
    let out = subpel_predict_block(&view, &params).unwrap();

    let taps = SUBPEL_FILTERS[2][8];
    let mut s = 0i64;
    for (t, &tap) in taps.iter().enumerate() {
        let col = 4 + t as i64 - 3; // 1..8
        s += i64::from(tap) * i64::from(samples[5 * ref_w + col as usize]);
    }
    let intermediate = (s + (1 << 2)) >> 3; // Round2(s, 3)
    let expected = ((128 * intermediate + (1 << 10)) >> 11).clamp(0, 255) as u16;
    assert_eq!(out[0], expected);

    let want = reference_subpel(&samples, ref_w, ref_h, &params);
    assert_eq!(out, want);
}

#[test]
fn reference_border_extension_clips() {
    let ref_w = 8usize;
    let ref_h = 8usize;
    let samples: Vec<u16> = (0..(ref_w * ref_h) as u16).map(|v| v + 1).collect();
    let samples = build_ref(samples, ref_w, ref_h);
    let view = ReferencePlaneView::new(&samples, ref_w, ref_h).unwrap();

    let params = SubpelPredictParams {
        interp: InterpolationFilter::EightTap,
        w: 8,
        h: 8,
        start_x: (8 << 6), // sample 0, phase 8 -> taps read cols -3..4 (clipped)
        start_y: (8 << 6), // sample 0, phase 8 -> rows read -3..4 (clipped)
        step_x: 1 << SCALE_SUBPEL_BITS,
        step_y: 1 << SCALE_SUBPEL_BITS,
        first_x: 0,
        first_y: 0,
        last_x: ref_w as i64 - 1,
        last_y: ref_h as i64 - 1,
        bit_depth: BitDepth::Eight,
    };
    let out = subpel_predict_block(&view, &params).unwrap();
    let want = reference_subpel(&samples, ref_w, ref_h, &params);
    assert_eq!(out, want);
}

#[test]
fn matches_independent_reference_over_many_cases() {
    let ref_w = 24usize;
    let ref_h = 24usize;
    let mut state: u64 = 0x1234_5678_9abc_def0;
    let mut next = || {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (state >> 33) as u32
    };

    let filters = [
        InterpolationFilter::EightTap,
        InterpolationFilter::EightTapSmooth,
        InterpolationFilter::EightTapSharp,
        InterpolationFilter::Bilinear,
    ];
    let dims = [2usize, 4, 8, 16];

    for case in 0..2000 {
        let samples: Vec<u16> = (0..(ref_w * ref_h))
            .map(|_| (next() % 256) as u16)
            .collect();
        let samples = build_ref(samples, ref_w, ref_h);
        let view = ReferencePlaneView::new(&samples, ref_w, ref_h).unwrap();

        let w = dims[(next() % dims.len() as u32) as usize];
        let h = dims[(next() % dims.len() as u32) as usize];
        let interp = filters[(next() % filters.len() as u32) as usize];

        let max_base_x = (ref_w - w - 4) as i64;
        let max_base_y = (ref_h - h - 4) as i64;
        let base_x = 4 + (next() as i64 % (max_base_x - 3));
        let base_y = 4 + (next() as i64 % (max_base_y - 3));
        let phase_x = (next() % 16) as i64;
        let phase_y = (next() % 16) as i64;

        let params = SubpelPredictParams {
            interp,
            w,
            h,
            start_x: (base_x << SCALE_SUBPEL_BITS) + (phase_x << 6),
            start_y: (base_y << SCALE_SUBPEL_BITS) + (phase_y << 6),
            step_x: 1 << SCALE_SUBPEL_BITS,
            step_y: 1 << SCALE_SUBPEL_BITS,
            first_x: 0,
            first_y: 0,
            last_x: ref_w as i64 - 1,
            last_y: ref_h as i64 - 1,
            bit_depth: BitDepth::Eight,
        };
        let out = subpel_predict_block(&view, &params).unwrap();
        let want = reference_subpel(&samples, ref_w, ref_h, &params);
        assert_eq!(
            out, want,
            "case {case} w={w} h={h} px={phase_x} py={phase_y}"
        );
    }
}

#[test]
fn edge_positions_match_independent_reference() {
    let ref_w = 24usize;
    let ref_h = 24usize;
    let mut state: u64 = 0x0fed_cba9_8765_4321;
    let mut next = || {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (state >> 33) as u32
    };

    let filters = [
        InterpolationFilter::EightTap,
        InterpolationFilter::EightTapSmooth,
        InterpolationFilter::EightTapSharp,
        InterpolationFilter::Bilinear,
    ];
    let dims = [2usize, 4, 8, 16];

    for case in 0..2000 {
        let samples: Vec<u16> = (0..(ref_w * ref_h))
            .map(|_| (next() % 256) as u16)
            .collect();
        let samples = build_ref(samples, ref_w, ref_h);
        let view = ReferencePlaneView::new(&samples, ref_w, ref_h).unwrap();

        let w = dims[(next() % dims.len() as u32) as usize];
        let h = dims[(next() % dims.len() as u32) as usize];
        let interp = filters[(next() % filters.len() as u32) as usize];

        let base_x = (next() as i64 % (ref_w as i64 + 12)) - 6;
        let base_y = (next() as i64 % (ref_h as i64 + 12)) - 6;
        let phase_x = (next() % 16) as i64;
        let phase_y = (next() % 16) as i64;

        let params = SubpelPredictParams {
            interp,
            w,
            h,
            start_x: (base_x << SCALE_SUBPEL_BITS) + (phase_x << 6),
            start_y: (base_y << SCALE_SUBPEL_BITS) + (phase_y << 6),
            step_x: 1 << SCALE_SUBPEL_BITS,
            step_y: 1 << SCALE_SUBPEL_BITS,
            first_x: 0,
            first_y: 0,
            last_x: ref_w as i64 - 1,
            last_y: ref_h as i64 - 1,
            bit_depth: BitDepth::Eight,
        };
        let out = subpel_predict_block(&view, &params).unwrap();
        let want = reference_subpel(&samples, ref_w, ref_h, &params);
        assert_eq!(
            out, want,
            "case {case} w={w} h={h} bx={base_x} by={base_y} px={phase_x} py={phase_y}"
        );
    }
}

#[test]
fn zero_phase_copy_matches_independent_reference() {
    let ref_w = 24usize;
    let ref_h = 24usize;
    let mut state: u64 = 0x1357_9bdf_2468_ace0;
    let mut next = || {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (state >> 33) as u32
    };
    let samples: Vec<u16> = (0..(ref_w * ref_h))
        .map(|_| (next() % 1024) as u16)
        .collect();
    let samples = build_ref(samples, ref_w, ref_h);
    let view = ReferencePlaneView::new(&samples, ref_w, ref_h).unwrap();

    for (base_x, base_y, w, h) in [
        (-5i64, -5i64, 8usize, 8usize),
        (0, 0, 4, 16),
        (9, 7, 16, 8),
        (20, 21, 8, 8),
    ] {
        let params = SubpelPredictParams {
            interp: InterpolationFilter::EightTap,
            w,
            h,
            start_x: base_x << SCALE_SUBPEL_BITS,
            start_y: base_y << SCALE_SUBPEL_BITS,
            step_x: 1 << SCALE_SUBPEL_BITS,
            step_y: 1 << SCALE_SUBPEL_BITS,
            first_x: 0,
            first_y: 0,
            last_x: ref_w as i64 - 1,
            last_y: ref_h as i64 - 1,
            bit_depth: BitDepth::Ten,
        };
        let out = subpel_predict_block(&view, &params).unwrap();
        let want = reference_subpel(&samples, ref_w, ref_h, &params);
        assert_eq!(out, want, "bx={base_x} by={base_y} w={w} h={h}");
    }
}

#[test]
fn strided_view_matches_contiguous_view() {
    let ref_w = 16usize;
    let ref_h = 12usize;
    let stride = 23usize;
    let mut state: u64 = 0x5a5a_a5a5_5a5a_a5a5;
    let mut next = || {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (state >> 33) as u32
    };
    let contiguous: Vec<u16> = (0..(ref_w * ref_h))
        .map(|_| (next() % 1024) as u16)
        .collect();
    let mut strided = vec![0x3fffu16 & 1023; stride * (ref_h - 1) + ref_w];
    for row in 0..ref_h {
        // splot-copy-ok: test fixture construction only (builds the strided plane)
        strided[row * stride..row * stride + ref_w]
            .copy_from_slice(&contiguous[row * ref_w..(row + 1) * ref_w]);
    }
    let flat = ReferencePlaneView::new(&contiguous, ref_w, ref_h).unwrap();
    let view = ReferencePlaneView::from_strided(&strided, stride, ref_w, ref_h).unwrap();

    for (base_x, base_y, phase) in [(-4i64, -4i64, 5i64), (3, 2, 0), (10, 6, 9), (14, 10, 15)] {
        let params = SubpelPredictParams {
            interp: InterpolationFilter::EightTapSharp,
            w: 8,
            h: 8,
            start_x: (base_x << SCALE_SUBPEL_BITS) + (phase << 6),
            start_y: (base_y << SCALE_SUBPEL_BITS) + (phase << 6),
            step_x: 1 << SCALE_SUBPEL_BITS,
            step_y: 1 << SCALE_SUBPEL_BITS,
            first_x: 0,
            first_y: 0,
            last_x: ref_w as i64 - 1,
            last_y: ref_h as i64 - 1,
            bit_depth: BitDepth::Ten,
        };
        assert_eq!(
            subpel_predict_block(&view, &params).unwrap(),
            subpel_predict_block(&flat, &params).unwrap(),
            "bx={base_x} by={base_y} phase={phase}"
        );
    }
}

#[test]
fn ten_bit_clip_uses_full_range() {
    let ref_w = 12usize;
    let ref_h = 12usize;
    let samples = build_ref(vec![1000u16; ref_w * ref_h], ref_w, ref_h);
    let view = ReferencePlaneView::new(&samples, ref_w, ref_h).unwrap();
    let params = SubpelPredictParams {
        bit_depth: BitDepth::Ten,
        ..full_pel_params(
            InterpolationFilter::EightTap,
            4,
            4,
            3,
            3,
            ref_w as i64,
            ref_h as i64,
        )
    };
    let out = subpel_predict_block(&view, &params).unwrap();
    assert!(out.iter().all(|&s| s == 1000), "10-bit flat: {out:?}");
}

#[test]
fn compound_intermediate_keeps_unclipped_prescaled_predictor() {
    let ref_w = 8usize;
    let ref_h = 8usize;
    let samples = build_ref(vec![255u16; ref_w * ref_h], ref_w, ref_h);
    let view = ReferencePlaneView::new(&samples, ref_w, ref_h).unwrap();
    let params = full_pel_params(
        InterpolationFilter::EightTap,
        4,
        4,
        2,
        2,
        ref_w as i64,
        ref_h as i64,
    );

    let intermediate = subpel_predict_block_compound_intermediate(&view, &params).unwrap();
    let prescale = 1i32 << (INTER_ROUND1_NON_COMPOUND - INTER_ROUND1_COMPOUND);
    assert!(
        intermediate.iter().all(|&sample| sample == 255 * prescale),
        "compound intermediate: {intermediate:?}"
    );

    let blended =
        blend_compound_average_equal(&intermediate, &intermediate, BitDepth::Eight).unwrap();
    assert!(
        blended.iter().all(|&sample| sample == 255),
        "equal-weight blend clips after averaging: {blended:?}"
    );
}

#[test]
fn compound_equal_average_blend_rounds_and_clips() {
    let left = [20 * 16, 60 * 16, 255 * 16];
    let right = [44 * 16, 120 * 16, 255 * 16];
    let out = blend_compound_average_equal(&left, &right, BitDepth::Eight).unwrap();
    assert_eq!(out, [32, 90, 255]);
}

#[test]
fn compound_weighted_average_blend_applies_cwp_weight() {
    let left = [20 * 16, 60 * 16];
    let right = [44 * 16, 120 * 16];

    assert_eq!(
        blend_compound_average_weighted(&left, &right, BitDepth::Eight, 12).unwrap(),
        [26, 75]
    );
    assert_eq!(
        blend_compound_average_weighted(&left[..1], &right[..1], BitDepth::Eight, -4).unwrap(),
        [50]
    );
}

#[test]
fn compound_blend_rejects_length_mismatch() {
    assert!(matches!(
        blend_compound_average_equal(&[0], &[0, 1], BitDepth::Eight),
        Err(ReconError::CompoundBlendLengthMismatch {
            left_len: 1,
            right_len: 2
        })
    ));
}

#[test]
fn small_block_uses_four_tap_filter() {
    let ref_w = 16usize;
    let ref_h = 16usize;
    let samples: Vec<u16> = (0..(ref_w * ref_h))
        .map(|i| ((i * 7) % 200 + 10) as u16)
        .collect();
    let samples = build_ref(samples, ref_w, ref_h);
    let view = ReferencePlaneView::new(&samples, ref_w, ref_h).unwrap();
    let params = SubpelPredictParams {
        interp: InterpolationFilter::EightTap,
        w: 4,
        h: 4,
        start_x: (5 << SCALE_SUBPEL_BITS) + (5 << 6),
        start_y: (6 << SCALE_SUBPEL_BITS) + (11 << 6),
        step_x: 1 << SCALE_SUBPEL_BITS,
        step_y: 1 << SCALE_SUBPEL_BITS,
        first_x: 0,
        first_y: 0,
        last_x: ref_w as i64 - 1,
        last_y: ref_h as i64 - 1,
        bit_depth: BitDepth::Eight,
    };
    let out = subpel_predict_block(&view, &params).unwrap();
    let want = reference_subpel(&samples, ref_w, ref_h, &params);
    assert_eq!(out, want);

    assert_ne!(SUBPEL_FILTERS[0][5], SUBPEL_FILTERS[4][5]);
}

#[test]
fn rejects_zero_dimension() {
    let samples = build_ref(vec![0u16; 16], 4, 4);
    let view = ReferencePlaneView::new(&samples, 4, 4).unwrap();
    let mut params = full_pel_params(InterpolationFilter::EightTap, 4, 4, 0, 0, 4, 4);
    params.w = 0;
    assert!(matches!(
        subpel_predict_block(&view, &params),
        Err(ReconError::ZeroDimension { .. })
    ));
}

#[test]
fn rejects_oversized_block() {
    let samples = build_ref(vec![0u16; 16], 4, 4);
    let view = ReferencePlaneView::new(&samples, 4, 4).unwrap();
    let mut params = full_pel_params(InterpolationFilter::EightTap, 4, 4, 0, 0, 4, 4);
    params.w = 129;
    assert!(matches!(
        subpel_predict_block(&view, &params),
        Err(ReconError::SubpelBlockDimensionUnsupported { w: 129, h: 4 })
    ));
}

#[test]
fn rejects_negative_step() {
    let samples = build_ref(vec![0u16; 16], 4, 4);
    let view = ReferencePlaneView::new(&samples, 4, 4).unwrap();
    let mut params = full_pel_params(InterpolationFilter::EightTap, 4, 4, 0, 0, 4, 4);
    params.step_y = -1;
    assert!(matches!(
        subpel_predict_block(&view, &params),
        Err(ReconError::SubpelNegativeStep { .. })
    ));
}

#[test]
fn rejects_overflowing_step_without_panic() {
    let samples = build_ref(vec![0u16; 16], 4, 4);
    let view = ReferencePlaneView::new(&samples, 4, 4).unwrap();
    let mut params = full_pel_params(InterpolationFilter::EightTap, 4, 4, 0, 0, 4, 4);
    params.step_y = i64::MAX;
    assert!(matches!(
        subpel_predict_block(&view, &params),
        Err(ReconError::ArithmeticOverflow { .. })
    ));
    let mut params = full_pel_params(InterpolationFilter::EightTap, 4, 4, 0, 0, 4, 4);
    params.step_x = i64::MAX;
    assert!(matches!(
        subpel_predict_block(&view, &params),
        Err(ReconError::ArithmeticOverflow { .. })
    ));
}

#[test]
fn reference_plane_view_rejects_length_mismatch() {
    let samples = vec![0u16; 15];
    assert!(matches!(
        ReferencePlaneView::new(&samples, 4, 4),
        Err(ReconError::SubpelReferencePlaneMismatch {
            expected: 16,
            actual: 15
        })
    ));
}

#[test]
fn reference_plane_view_rejects_zero_dimension() {
    let samples: Vec<u16> = Vec::new();
    assert!(matches!(
        ReferencePlaneView::new(&samples, 0, 4),
        Err(ReconError::ZeroDimension { .. })
    ));
}
