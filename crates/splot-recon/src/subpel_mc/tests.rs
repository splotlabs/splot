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
    rx: i32,
    ry: i32,
    ref_w: i32,
    ref_h: i32,
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

/// An independent in-test re-trace of the AV2 § 7.13.3.18 two-pass
/// convolution, used as the property-test oracle. The
/// explicit `for t in 0..8` tap loops mirror the spec pseudocode index variable.
#[allow(
    clippy::too_many_arguments,
    clippy::needless_range_loop,
    clippy::many_single_char_names
)]
fn reference_subpel_values(
    samples: &[u16],
    ref_w: usize,
    ref_h: usize,
    params: &SubpelPredictParams,
    inter_round1: u32,
) -> Vec<i64> {
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
    let intermediate_height =
        ((((h as i64 - 1) * i64::from(y_step) + (1 << 10) - 1) >> 10) + 8) as usize;

    let clip3 = |lo: i32, hi: i32, v: i32| -> i32 {
        if v < lo {
            lo
        } else if v > hi {
            hi
        } else {
            v
        }
    };
    let round2 = |v: i64, n: u32| -> i64 { if n == 0 { v } else { (v + (1 << (n - 1))) >> n } };
    let fetch = |row: i32, col: i32| -> i64 {
        let row = clip3(0, ref_h as i32 - 1, row) as usize;
        let col = clip3(0, ref_w as i32 - 1, col) as usize;
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
            let p = x + x_step * c as i32;
            let mut s = 0i64;
            for t in 0..8 {
                let phase = ((p >> 6) & 15) as usize;
                let tap = i64::from(SUBPEL_FILTERS[h_filter][phase][t]);
                let rr = clip3(first_y, last_y, (y >> 10) + r as i32 - 3);
                let cc = clip3(first_x, last_x, (p >> 10) + t as i32 - 3);
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
    let mut out = vec![0i64; w * h];
    for r in 0..h {
        for c in 0..w {
            let p = (y & 1023) + y_step * r as i32;
            let mut s = 0i64;
            for t in 0..8 {
                let phase = ((p >> 6) & 15) as usize;
                let tap = i64::from(SUBPEL_FILTERS[v_filter][phase][t]);
                let row = ((p >> 10) + t as i32) as usize;
                s += tap * intermediate[row * w + c];
            }
            out[r * w + c] = round2(s, inter_round1);
        }
    }
    out
}

fn reference_subpel(
    samples: &[u16],
    ref_w: usize,
    ref_h: usize,
    params: &SubpelPredictParams,
) -> Vec<u16> {
    let max_sample = i64::from(params.bit_depth.max_sample());
    reference_subpel_values(samples, ref_w, ref_h, params, INTER_ROUND1_NON_COMPOUND)
        .into_iter()
        .map(|sample| sample.clamp(0, max_sample) as u16)
        .collect()
}

fn reference_subpel_compound(
    samples: &[u16],
    ref_w: usize,
    ref_h: usize,
    params: &SubpelPredictParams,
) -> Vec<i32> {
    reference_subpel_values(samples, ref_w, ref_h, params, INTER_ROUND1_COMPOUND)
        .into_iter()
        .map(|sample| sample as i32)
        .collect()
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
        ref_w as i32,
        ref_h as i32,
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
fn full_pel_into_clamps_vector_chunks_and_tail() {
    let ref_w = 16usize;
    let ref_h = 8usize;
    let samples = build_ref(vec![1200u16; ref_w * ref_h], ref_w, ref_h);
    let view = ReferencePlaneView::new(&samples, ref_w, ref_h).unwrap();
    let params = SubpelPredictParams {
        bit_depth: BitDepth::Ten,
        ..full_pel_params(
            InterpolationFilter::EightTap,
            10,
            3,
            2,
            1,
            ref_w as i32,
            ref_h as i32,
        )
    };
    let mut output = [u16::MAX; 32];

    subpel_predict_block_into(&view, &params, &mut output).unwrap();

    assert_eq!(&output[..30], &[1023; 30]);
    assert_eq!(&output[30..], &[u16::MAX; 2]);
}

#[test]
fn bilinear_horizontal_overlap_matches_fresh_tip_predictor() {
    let ref_w = 48usize;
    let ref_h = 32usize;
    let samples = build_ref(
        (0..ref_w * ref_h)
            .map(|index| ((index * 37 + index / ref_w * 19) % 1024) as u16)
            .collect(),
        ref_w,
        ref_h,
    );
    let view = ReferencePlaneView::new(&samples, ref_w, ref_h).unwrap();
    for (h_phase, v_phase) in [(0, 0), (5, 0), (0, 7), (5, 7)] {
        let previous = SubpelPredictParams {
            interp: InterpolationFilter::Bilinear,
            w: 16,
            h: 16,
            start_x: (4 << SCALE_SUBPEL_BITS) + (h_phase << 6),
            start_y: (5 << SCALE_SUBPEL_BITS) + (v_phase << 6),
            step_x: 1 << SCALE_SUBPEL_BITS,
            step_y: 1 << SCALE_SUBPEL_BITS,
            first_x: 5,
            first_y: 6,
            last_x: 19,
            last_y: 20,
            bit_depth: BitDepth::Ten,
        };
        let current = SubpelPredictParams {
            start_x: previous.start_x + (8 << SCALE_SUBPEL_BITS),
            first_x: previous.first_x + 8,
            last_x: previous.last_x + 8,
            ..previous
        };
        let mut reused = vec![0; previous.w * previous.h];
        subpel_predict_block_into(&view, &previous, &mut reused).unwrap();
        assert!(
            subpel_predict_16x16_bilinear_horizontal_overlap_into(&view, &current, &mut reused)
                .unwrap()
        );

        let mut expected = vec![0; current.w * current.h];
        subpel_predict_block_into(&view, &current, &mut expected).unwrap();
        assert_eq!(reused, expected, "phases ({h_phase}, {v_phase})");
    }
}

#[test]
fn bilinear_horizontal_overlap_clips_physical_plane_borders() {
    let ref_w = 48usize;
    let ref_h = 32usize;
    let samples = build_ref(
        (0..ref_w * ref_h)
            .map(|index| ((index * 37 + index / ref_w * 19) % 1024) as u16)
            .collect(),
        ref_w,
        ref_h,
    );
    let view = ReferencePlaneView::new(&samples, ref_w, ref_h).unwrap();

    for (x0, y0) in [(-10, -2), (26, 18)] {
        let previous = SubpelPredictParams {
            interp: InterpolationFilter::Bilinear,
            w: 16,
            h: 16,
            start_x: x0 * (1 << SCALE_SUBPEL_BITS) + (5 << 6),
            start_y: y0 * (1 << SCALE_SUBPEL_BITS) + (7 << 6),
            step_x: 1 << SCALE_SUBPEL_BITS,
            step_y: 1 << SCALE_SUBPEL_BITS,
            first_x: x0 + 1,
            first_y: y0 + 1,
            last_x: x0 + 15,
            last_y: y0 + 15,
            bit_depth: BitDepth::Ten,
        };
        let current = SubpelPredictParams {
            start_x: previous.start_x + (8 << SCALE_SUBPEL_BITS),
            first_x: previous.first_x + 8,
            last_x: previous.last_x + 8,
            ..previous
        };
        let mut reused = vec![0; previous.w * previous.h];
        subpel_predict_block_into(&view, &previous, &mut reused).unwrap();
        assert!(
            subpel_predict_16x16_bilinear_horizontal_overlap_into(&view, &current, &mut reused)
                .unwrap()
        );

        let mut expected = vec![0; current.w * current.h];
        subpel_predict_block_into(&view, &current, &mut expected).unwrap();
        assert_eq!(reused, expected, "previous origin ({x0}, {y0})");
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
        last_x: ref_w as i32 - 1,
        last_y: ref_h as i32 - 1,
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
        last_x: ref_w as i32 - 1,
        last_y: ref_h as i32 - 1,
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
fn half_pel_vertical_with_horizontal_zero_phase_matches_reference() {
    let ref_w = 16usize;
    let ref_h = 16usize;
    let samples: Vec<u16> = (0..ref_w * ref_h)
        .map(|index| ((index * 17 + 23) % 256) as u16)
        .collect();
    let samples = build_ref(samples, ref_w, ref_h);
    let view = ReferencePlaneView::new(&samples, ref_w, ref_h).unwrap();
    let params = SubpelPredictParams {
        interp: InterpolationFilter::EightTapSharp,
        w: 8,
        h: 8,
        start_x: 4 << SCALE_SUBPEL_BITS,
        start_y: (4 << SCALE_SUBPEL_BITS) + (8 << 6),
        step_x: 1 << SCALE_SUBPEL_BITS,
        step_y: 1 << SCALE_SUBPEL_BITS,
        first_x: 0,
        first_y: 0,
        last_x: ref_w as i32 - 1,
        last_y: ref_h as i32 - 1,
        bit_depth: BitDepth::Eight,
    };

    assert_eq!(
        subpel_predict_block(&view, &params).unwrap(),
        reference_subpel(&samples, ref_w, ref_h, &params)
    );
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
        last_x: ref_w as i32 - 1,
        last_y: ref_h as i32 - 1,
        bit_depth: BitDepth::Eight,
    };
    let out = subpel_predict_block(&view, &params).unwrap();
    let want = reference_subpel(&samples, ref_w, ref_h, &params);
    assert_eq!(out, want);
}

#[test]
fn bilinear_fixed_window_clips_physical_plane_borders() {
    let ref_w = 32usize;
    let ref_h = 32usize;
    let samples = build_ref(
        (0..ref_w * ref_h)
            .map(|index| ((index * 37 + index / ref_w * 19) % 1024) as u16)
            .collect(),
        ref_w,
        ref_h,
    );
    let view = ReferencePlaneView::new(&samples, ref_w, ref_h).unwrap();

    for (h_phase, v_phase) in [(8, 0), (0, 8), (8, 8)] {
        for (x0, y0) in [(-2, 4), (18, 4), (4, -2), (4, 18)] {
            let params = SubpelPredictParams {
                interp: InterpolationFilter::Bilinear,
                w: 16,
                h: 16,
                start_x: x0 * (1 << SCALE_SUBPEL_BITS) + (h_phase << 6),
                start_y: y0 * (1 << SCALE_SUBPEL_BITS) + (v_phase << 6),
                step_x: 1 << SCALE_SUBPEL_BITS,
                step_y: 1 << SCALE_SUBPEL_BITS,
                first_x: x0 + 1,
                first_y: y0 + 1,
                last_x: x0 + 15,
                last_y: y0 + 15,
                bit_depth: BitDepth::Ten,
            };

            let mut actual = vec![0; params.w * params.h];
            subpel_predict_block_into(&view, &params, &mut actual).unwrap();
            assert_eq!(
                actual,
                reference_subpel(&samples, ref_w, ref_h, &params),
                "origin ({x0}, {y0}), phases ({h_phase}, {v_phase})"
            );
        }
    }
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

        let max_base_x = (ref_w - w - 4) as i32;
        let max_base_y = (ref_h - h - 4) as i32;
        let base_x = 4 + (next() as i32 % (max_base_x - 3));
        let base_y = 4 + (next() as i32 % (max_base_y - 3));
        let phase_x = (next() % 16) as i32;
        let phase_y = (next() % 16) as i32;

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
            last_x: ref_w as i32 - 1,
            last_y: ref_h as i32 - 1,
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

        let base_x = (next() as i32 % (ref_w as i32 + 12)) - 6;
        let base_y = (next() as i32 % (ref_h as i32 + 12)) - 6;
        let phase_x = (next() % 16) as i32;
        let phase_y = (next() % 16) as i32;

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
            last_x: ref_w as i32 - 1,
            last_y: ref_h as i32 - 1,
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

/// The § 7.13.3.16 horizontal-pass value range is what makes the 16-bit
/// `intermediate` storage exact, so the kernel has to stay reference-exact at
/// both ends of it. Aligning 10-bit extremes with the sign pattern of the
/// widest § 7.13.3.18 filter row (`EIGHTTAP_SHARP` phase 8, positive taps at
/// `t` in 1/3/4/6) makes some window reach `Round2(184 * 1023, 3) == 23529`
/// and another reach `Round2(-56 * 1023, 3) == -7161`; gating whole rows the
/// same way carries those extremes into the vertical accumulator.
#[test]
fn extreme_ten_bit_contrast_matches_independent_reference() {
    let ref_w = 24usize;
    let ref_h = 24usize;
    let widest = SUBPEL_FILTERS[EIGHTTAP_SHARP as usize][8];
    let extreme = |index: usize| u16::from(widest[index % NUM_TAPS] > 0) * 1023;

    for (label, samples) in [
        (
            "columns",
            (0..(ref_w * ref_h))
                .map(|index| extreme(index % ref_w))
                .collect::<Vec<u16>>(),
        ),
        (
            "rows and columns",
            (0..(ref_w * ref_h))
                .map(|index| extreme(index % ref_w).min(extreme(index / ref_w)))
                .collect::<Vec<u16>>(),
        ),
    ] {
        let samples = build_ref(samples, ref_w, ref_h);
        let view = ReferencePlaneView::new(&samples, ref_w, ref_h).unwrap();
        for phase_x in 0..16i32 {
            for phase_y in 0..16i32 {
                let params = SubpelPredictParams {
                    start_x: (4 << SCALE_SUBPEL_BITS) + (phase_x << 6),
                    start_y: (4 << SCALE_SUBPEL_BITS) + (phase_y << 6),
                    bit_depth: BitDepth::Ten,
                    ..full_pel_params(
                        InterpolationFilter::EightTapSharp,
                        16,
                        16,
                        4,
                        4,
                        ref_w as i32,
                        ref_h as i32,
                    )
                };
                assert_eq!(
                    subpel_predict_block(&view, &params).unwrap(),
                    reference_subpel(&samples, ref_w, ref_h, &params),
                    "{label} px={phase_x} py={phase_y}"
                );
                assert_eq!(
                    subpel_predict_block_compound_intermediate(&view, &params).unwrap(),
                    reference_subpel_compound(&samples, ref_w, ref_h, &params),
                    "{label} compound px={phase_x} py={phase_y}"
                );
            }
        }
    }
}

/// Scaled steps (`stepX`/`stepY != 1 << SCALE_SUBPEL_BITS`) make the vertical
/// pass skip intermediate rows between output rows, so the restricted horizontal
/// pass must still match the full-range reference for every phase — including the
/// integer-vertical (`phase == 0`) rows whose bases jump past unread rows.
#[test]
fn scaled_steps_match_independent_reference() {
    let ref_w = 32usize;
    let ref_h = 32usize;
    let mut state: u64 = 0x2b7e_1516_28ae_d2a6;
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
    let dims = [4usize, 8, 16];
    let steps = [1024i32, 1280, 1536, 2048];

    for case in 0..3000 {
        let samples: Vec<u16> = (0..(ref_w * ref_h))
            .map(|_| (next() % 1024) as u16)
            .collect();
        let samples = build_ref(samples, ref_w, ref_h);
        let view = ReferencePlaneView::new(&samples, ref_w, ref_h).unwrap();

        let w = dims[(next() % dims.len() as u32) as usize];
        let h = dims[(next() % dims.len() as u32) as usize];
        let interp = filters[(next() % filters.len() as u32) as usize];
        let step_x = steps[(next() % steps.len() as u32) as usize];
        let step_y = steps[(next() % steps.len() as u32) as usize];

        let base_x = (next() as i32 % (ref_w as i32 + 8)) - 4;
        let base_y = (next() as i32 % (ref_h as i32 + 8)) - 4;
        let phase_x = (next() % 16) as i32;
        let phase_y = (next() % 16) as i32;

        let params = SubpelPredictParams {
            interp,
            w,
            h,
            start_x: (base_x << SCALE_SUBPEL_BITS) + (phase_x << 6),
            start_y: (base_y << SCALE_SUBPEL_BITS) + (phase_y << 6),
            step_x,
            step_y,
            first_x: 0,
            first_y: 0,
            last_x: ref_w as i32 - 1,
            last_y: ref_h as i32 - 1,
            bit_depth: BitDepth::Ten,
        };
        let out = subpel_predict_block(&view, &params).unwrap();
        let want = reference_subpel(&samples, ref_w, ref_h, &params);
        assert_eq!(
            out, want,
            "case {case} w={w} h={h} sx={step_x} sy={step_y} bx={base_x} by={base_y} px={phase_x} py={phase_y}"
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
        (-5i32, -5i32, 8usize, 8usize),
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
            last_x: ref_w as i32 - 1,
            last_y: ref_h as i32 - 1,
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

    for (base_x, base_y, phase) in [(-4i32, -4i32, 5i32), (3, 2, 0), (10, 6, 9), (14, 10, 15)] {
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
            last_x: ref_w as i32 - 1,
            last_y: ref_h as i32 - 1,
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
fn published_view_clamps_reads_to_the_available_prefix() {
    let samples = [1u16, 2, 3, 4, 5, 6];
    let view = ReferencePlaneView::from_published_strided(&samples, 3, 3, 4, 2).unwrap();

    assert_eq!(view.sample(3, 2), 6);
    assert_eq!(view.row(3), &[4, 5, 6]);
}

#[test]
fn published_view_fast_path_stays_within_the_available_prefix() {
    let samples = vec![511u16; 16 * 2];
    let view = ReferencePlaneView::from_published_strided(&samples, 16, 16, 16, 2).unwrap();
    let params = SubpelPredictParams {
        interp: InterpolationFilter::Bilinear,
        w: 16,
        h: 16,
        start_x: (-1 << SCALE_SUBPEL_BITS) + (8 << 6),
        start_y: (-1 << SCALE_SUBPEL_BITS) + (8 << 6),
        step_x: 1 << SCALE_SUBPEL_BITS,
        step_y: 1 << SCALE_SUBPEL_BITS,
        first_x: 0,
        first_y: 0,
        last_x: 14,
        last_y: 14,
        bit_depth: BitDepth::Ten,
    };

    assert!(
        subpel_predict_block(&view, &params)
            .unwrap()
            .iter()
            .all(|&sample| sample == 511)
    );
}

#[test]
fn published_view_zero_phase_copy_stays_within_the_available_prefix() {
    let mut samples = vec![1u16; 16 * 2];
    samples[16..].fill(2);
    let view = ReferencePlaneView::from_published_strided(&samples, 16, 16, 16, 2).unwrap();
    let params = full_pel_params(InterpolationFilter::Bilinear, 8, 4, 0, 8, 16, 16);

    assert_eq!(
        subpel_predict_block(&view, &params).unwrap(),
        vec![2; 8 * 4]
    );
}

#[test]
fn ten_bit_clip_uses_full_range() {
    let ref_w = 12usize;
    let ref_h = 12usize;
    let samples = build_ref(vec![1200u16; ref_w * ref_h], ref_w, ref_h);
    let view = ReferencePlaneView::new(&samples, ref_w, ref_h).unwrap();
    let params = SubpelPredictParams {
        bit_depth: BitDepth::Ten,
        start_x: (3 << SCALE_SUBPEL_BITS) + (8 << 6),
        start_y: (3 << SCALE_SUBPEL_BITS) + (8 << 6),
        ..full_pel_params(
            InterpolationFilter::EightTap,
            4,
            4,
            3,
            3,
            ref_w as i32,
            ref_h as i32,
        )
    };
    let out = subpel_predict_block(&view, &params).unwrap();
    assert!(out.iter().all(|&s| s == 1023), "10-bit clip: {out:?}");

    let mut into = vec![u16::MAX; params.w * params.h + 2];
    subpel_predict_block_into(&view, &params, &mut into).unwrap();
    assert_eq!(&into[..params.w * params.h], out);
    assert_eq!(&into[params.w * params.h..], &[u16::MAX; 2]);
}

#[test]
fn bilinear_2d_into_matches_reference_for_direct_and_clipped_blocks() {
    let ref_w = 17usize;
    let ref_h = 13usize;
    let samples = build_ref(
        (0..ref_w * ref_h)
            .map(|index| ((index * 73 + index / ref_w * 211) % 1200) as u16)
            .collect(),
        ref_w,
        ref_h,
    );
    let view = ReferencePlaneView::new(&samples, ref_w, ref_h).unwrap();

    for (base_x, base_y, w, h, phase_x, phase_y) in [
        (3, 2, 6, 5, 1, 15),
        (-2, -1, 6, 5, 8, 8),
        (14, 11, 6, 5, 15, 1),
    ] {
        let params = SubpelPredictParams {
            interp: InterpolationFilter::Bilinear,
            w,
            h,
            start_x: (base_x << SCALE_SUBPEL_BITS) + (phase_x << 6),
            start_y: (base_y << SCALE_SUBPEL_BITS) + (phase_y << 6),
            step_x: 1 << SCALE_SUBPEL_BITS,
            step_y: 1 << SCALE_SUBPEL_BITS,
            first_x: 0,
            first_y: 0,
            last_x: ref_w as i32 - 1,
            last_y: ref_h as i32 - 1,
            bit_depth: BitDepth::Ten,
        };
        let expected = reference_subpel(&samples, ref_w, ref_h, &params);
        let mut output = vec![u16::MAX; w * h + 2];
        subpel_predict_block_into(&view, &params, &mut output).unwrap();
        assert_eq!(&output[..w * h], expected);
        assert_eq!(&output[w * h..], &[u16::MAX; 2]);
    }
}

#[test]
fn single_prediction_strided_matches_contiguous_and_preserves_padding() {
    let ref_w = 24usize;
    let ref_h = 20usize;
    let samples = build_ref(
        (0..ref_w * ref_h)
            .map(|index| ((index * 73 + 19) & 1023) as u16)
            .collect(),
        ref_w,
        ref_h,
    );
    let view = ReferencePlaneView::new(&samples, ref_w, ref_h).unwrap();
    let base = full_pel_params(
        InterpolationFilter::EightTap,
        8,
        8,
        5,
        4,
        ref_w as i32,
        ref_h as i32,
    );
    for params in [
        base,
        SubpelPredictParams {
            interp: InterpolationFilter::Bilinear,
            start_x: base.start_x + (5 << 6),
            ..base
        },
        SubpelPredictParams {
            start_x: base.start_x + (7 << 6),
            start_y: base.start_y + (11 << 6),
            ..base
        },
    ] {
        let mut expected = vec![0; params.w * params.h];
        subpel_predict_block_into(&view, &params, &mut expected).unwrap();
        let stride = params.w + 5;
        let sentinel = u16::MAX;
        let mut actual = vec![sentinel; stride * params.h];
        subpel_predict_block_strided_into(&view, &params, &mut actual, stride).unwrap();
        for row in 0..params.h {
            assert_eq!(
                &actual[row * stride..row * stride + params.w],
                &expected[row * params.w..(row + 1) * params.w],
            );
            assert!(
                actual[row * stride + params.w..(row + 1) * stride]
                    .iter()
                    .all(|&sample| sample == sentinel)
            );
        }
    }
}

#[test]
fn single_prediction_rows_match_full_block_filter_selection() {
    let ref_w = 32usize;
    let ref_h = 32usize;
    let samples = build_ref(
        (0..ref_w * ref_h)
            .map(|index| ((index * 73 + 19) & 1023) as u16)
            .collect(),
        ref_w,
        ref_h,
    );
    let view = ReferencePlaneView::new(&samples, ref_w, ref_h).unwrap();
    for (interp, width, height) in [
        (InterpolationFilter::EightTap, 8, 8),
        (InterpolationFilter::EightTapSharp, 4, 4),
        (InterpolationFilter::Bilinear, 8, 16),
    ] {
        let params = SubpelPredictParams {
            start_x: (5 << SCALE_SUBPEL_BITS) + (7 << 6),
            start_y: (4 << SCALE_SUBPEL_BITS) + (11 << 6),
            ..full_pel_params(interp, width, height, 5, 4, ref_w as i32, ref_h as i32)
        };
        let mut expected = vec![0; params.w * params.h];
        subpel_predict_block_into(&view, &params, &mut expected).unwrap();
        for row in 0..params.h {
            let mut actual = vec![0; params.w];
            subpel_predict_block_row_into(&view, &params, row, &mut actual).unwrap();
            assert_eq!(actual, expected[row * params.w..(row + 1) * params.w]);
        }
    }
}

#[test]
fn single_prediction_into_rejects_short_output_without_writes() {
    let ref_w = 8usize;
    let ref_h = 8usize;
    let samples = build_ref(vec![80u16; ref_w * ref_h], ref_w, ref_h);
    let view = ReferencePlaneView::new(&samples, ref_w, ref_h).unwrap();
    let params = SubpelPredictParams {
        interp: InterpolationFilter::Bilinear,
        start_x: (2 << SCALE_SUBPEL_BITS) + (3 << 6),
        start_y: (2 << SCALE_SUBPEL_BITS) + (13 << 6),
        ..full_pel_params(
            InterpolationFilter::Bilinear,
            4,
            4,
            2,
            2,
            ref_w as i32,
            ref_h as i32,
        )
    };
    let sentinel = u16::MAX;
    let mut output = vec![sentinel; params.w * params.h - 1];

    assert_eq!(
        subpel_predict_block_into(&view, &params, &mut output),
        Err(ReconError::BufferLengthMismatch {
            expected: params.w * params.h,
            actual: params.w * params.h - 1,
        })
    );
    assert!(output.iter().all(|&sample| sample == sentinel));
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
        ref_w as i32,
        ref_h as i32,
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
fn compound_intermediate_into_matches_owned_copy_and_filtered() {
    let ref_w = 16usize;
    let ref_h = 16usize;
    let samples = build_ref(
        (0..ref_w * ref_h)
            .map(|index| ((index * 13 + 7) % 256) as u16)
            .collect(),
        ref_w,
        ref_h,
    );
    let view = ReferencePlaneView::new(&samples, ref_w, ref_h).unwrap();
    let copy = full_pel_params(
        InterpolationFilter::EightTap,
        4,
        4,
        3,
        4,
        ref_w as i32,
        ref_h as i32,
    );
    let filtered = SubpelPredictParams {
        start_x: (3 << SCALE_SUBPEL_BITS) + (5 << 6),
        start_y: (4 << SCALE_SUBPEL_BITS) + (11 << 6),
        ..copy
    };

    for params in [copy, filtered] {
        let expected = subpel_predict_block_compound_intermediate(&view, &params).unwrap();
        let stride = params.w + 3;
        let sentinel = -12345;
        let mut output = vec![sentinel; stride * params.h + 2];
        subpel_predict_block_compound_intermediate_into(&view, &params, None, &mut output, stride)
            .unwrap();

        for (scratch_len, label) in [(4096usize, "caller scratch"), (1, "undersized scratch")] {
            let mut scratch = vec![0i16; scratch_len];
            let mut scratched = vec![sentinel; stride * params.h + 2];
            subpel_predict_block_compound_intermediate_into(
                &view,
                &params,
                Some(&mut scratch),
                &mut scratched,
                stride,
            )
            .unwrap();
            assert_eq!(scratched, output, "{label}");
        }

        for row in 0..params.h {
            assert_eq!(
                &output[row * stride..row * stride + params.w],
                &expected[row * params.w..(row + 1) * params.w]
            );
            assert!(
                output[row * stride + params.w..(row + 1) * stride]
                    .iter()
                    .all(|&sample| sample == sentinel)
            );
        }
        assert!(
            output[stride * params.h..]
                .iter()
                .all(|&sample| sample == sentinel)
        );
    }
}

#[test]
fn compound_average_into_matches_materialized_second_predictor() {
    let ref_w = 24usize;
    let ref_h = 24usize;
    let samples = build_ref(
        (0..ref_w * ref_h)
            .map(|index| ((index * 73 + (index / ref_w) * 29) % 1024) as u16)
            .collect(),
        ref_w,
        ref_h,
    );
    let view = ReferencePlaneView::new(&samples, ref_w, ref_h).unwrap();
    let base = SubpelPredictParams {
        interp: InterpolationFilter::EightTap,
        w: 7,
        h: 5,
        start_x: 6 << SCALE_SUBPEL_BITS,
        start_y: 7 << SCALE_SUBPEL_BITS,
        step_x: 1 << SCALE_SUBPEL_BITS,
        step_y: 1 << SCALE_SUBPEL_BITS,
        first_x: 0,
        first_y: 0,
        last_x: ref_w as i32 - 1,
        last_y: ref_h as i32 - 1,
        bit_depth: BitDepth::Ten,
    };
    let cases = [
        base,
        SubpelPredictParams {
            start_x: base.start_x + (5 << 6),
            ..base
        },
        SubpelPredictParams {
            start_y: base.start_y + (11 << 6),
            ..base
        },
        SubpelPredictParams {
            start_x: -(3 << 6),
            start_y: base.start_y + (9 << 6),
            step_x: 896,
            step_y: 1152,
            ..base
        },
    ];
    let pred0 = (0..base.w * base.h)
        .map(|index| ((index * 131 + 47) % 1024) as i32 * 16)
        .collect::<Vec<_>>();

    for params in cases {
        let pred1 = subpel_predict_block_compound_intermediate(&view, &params).unwrap();
        for cwp_weight in [8, 12] {
            let expected =
                blend_compound_average_weighted(&pred0, &pred1, params.bit_depth, cwp_weight)
                    .unwrap();
            let mut actual = vec![0; pred0.len()];
            subpel_predict_block_compound_average_into(
                &view,
                &params,
                &pred0,
                cwp_weight,
                &mut actual,
            )
            .unwrap();
            assert_eq!(actual, expected, "{params:?}, cwp_weight={cwp_weight}");

            let stride = params.w + 3;
            let sentinel = u16::MAX;
            let mut strided = vec![sentinel; stride * params.h + 2];
            subpel_predict_block_compound_average_strided_into(
                &view,
                &params,
                &pred0,
                cwp_weight,
                None,
                &mut strided,
                stride,
            )
            .unwrap();
            for row in 0..params.h {
                assert_eq!(
                    &strided[row * stride..row * stride + params.w],
                    &expected[row * params.w..(row + 1) * params.w]
                );
                assert!(
                    strided[row * stride + params.w..(row + 1) * stride]
                        .iter()
                        .all(|&sample| sample == sentinel)
                );
            }
            assert!(
                strided[stride * params.h..]
                    .iter()
                    .all(|&sample| sample == sentinel)
            );
        }
    }

    let mut oversized = vec![0; pred0.len() + 1];
    assert!(matches!(
        subpel_predict_block_compound_average_into(
            &view,
            &base,
            &pred0,
            8,
            &mut oversized,
        ),
        Err(ReconError::BufferLengthMismatch {
            expected,
            actual
        }) if expected == pred0.len() && actual == pred0.len() + 1
    ));
}

#[test]
fn clipped_one_axis_compound_average_preserves_column_order() {
    let ref_w = 32usize;
    let ref_h = 12usize;
    let samples = build_ref(
        (0..ref_w * ref_h)
            .map(|index| ((index * 73 + (index / ref_w) * 29) % 1024) as u16)
            .collect(),
        ref_w,
        ref_h,
    );
    let view = ReferencePlaneView::new(&samples, ref_w, ref_h).unwrap();
    let horizontal = SubpelPredictParams {
        interp: InterpolationFilter::EightTap,
        w: 16,
        h: 5,
        start_x: -(2 << SCALE_SUBPEL_BITS) + (5 << 6),
        start_y: 3 << SCALE_SUBPEL_BITS,
        step_x: 1 << SCALE_SUBPEL_BITS,
        step_y: 1 << SCALE_SUBPEL_BITS,
        first_x: 0,
        first_y: 0,
        last_x: ref_w as i32 - 1,
        last_y: ref_h as i32 - 1,
        bit_depth: BitDepth::Ten,
    };
    let vertical = SubpelPredictParams {
        start_x: -(2 << SCALE_SUBPEL_BITS),
        start_y: (3 << SCALE_SUBPEL_BITS) + (5 << 6),
        ..horizontal
    };
    let pred0 = (0..horizontal.w * horizontal.h)
        .map(|index| ((index * 131 + 47) % 1024) as i32 * 16)
        .collect::<Vec<_>>();
    for params in [horizontal, vertical] {
        let pred1 = subpel_predict_block_compound_intermediate(&view, &params).unwrap();
        let expected =
            blend_compound_average_weighted(&pred0, &pred1, params.bit_depth, 12).unwrap();
        let mut actual = vec![0; pred0.len()];

        subpel_predict_block_compound_average_into(&view, &params, &pred0, 12, &mut actual)
            .unwrap();

        assert_eq!(actual, expected);
    }
}

#[test]
fn compound_average_u8_output_matches_u16_output() {
    let ref_w = 24;
    let ref_h = 20;
    let samples: Vec<u8> = (0..ref_w * ref_h)
        .map(|index| ((index * 37 + index / ref_w * 11) % 256) as u8)
        .collect();
    let view = ReferencePlaneView::new(&samples, ref_w, ref_h).unwrap();
    let params = SubpelPredictParams {
        interp: InterpolationFilter::EightTap,
        w: 12,
        h: 7,
        start_x: (5 << SCALE_SUBPEL_BITS) + (7 << 6),
        start_y: (6 << SCALE_SUBPEL_BITS) + (9 << 6),
        step_x: 1 << SCALE_SUBPEL_BITS,
        step_y: 1 << SCALE_SUBPEL_BITS,
        first_x: 0,
        first_y: 0,
        last_x: ref_w as i32 - 1,
        last_y: ref_h as i32 - 1,
        bit_depth: BitDepth::Eight,
    };
    let pred0 = subpel_predict_block_compound_intermediate(&view, &params).unwrap();
    let stride = params.w + 3;
    let mut expected = vec![u16::MAX; stride * params.h];
    subpel_predict_block_compound_average_strided_into(
        &view,
        &params,
        &pred0,
        12,
        None,
        &mut expected,
        stride,
    )
    .unwrap();
    let mut actual = vec![u8::MAX; stride * params.h];
    subpel_predict_block_compound_average_strided_into_u8(
        &view,
        &params,
        &pred0,
        12,
        None,
        &mut actual,
        stride,
    )
    .unwrap();
    for row in 0..params.h {
        assert_eq!(
            &actual[row * stride..row * stride + params.w],
            &expected[row * stride..row * stride + params.w]
                .iter()
                .map(|&sample| sample as u8)
                .collect::<Vec<_>>()
        );
    }
}

#[test]
fn compound_average_sink_matches_scalar_oracle_across_shapes_and_clamps() {
    let shapes = [
        (1usize, 1usize),
        (2, 2),
        (3, 3),
        (4, 4),
        (5, 7),
        (7, 5),
        (8, 8),
        (9, 3),
        (12, 5),
        (15, 2),
        (16, 4),
        (17, 6),
        (23, 3),
        (31, 2),
        (32, 3),
        (33, 1),
    ];
    let phases = [(0i32, 0i32), (0, 8), (11, 0), (5, 13)];
    let ref_w = 48usize;
    let ref_h = 24usize;
    let samples = build_ref(
        (0..ref_w * ref_h)
            .map(|index| ((index * 73 + index / ref_w * 29) % 1024) as u16)
            .collect(),
        ref_w,
        ref_h,
    );
    let view = ReferencePlaneView::new(&samples, ref_w, ref_h).unwrap();
    let pred0_pattern = |w: usize, h: usize| {
        (0..w * h)
            .map(|index| ((index * 97) % 4001) as i32 * 48 - 96_000)
            .collect::<Vec<_>>()
    };

    for (w, h) in shapes {
        let base = SubpelPredictParams {
            interp: InterpolationFilter::EightTap,
            w,
            h,
            start_x: 0,
            start_y: 0,
            step_x: 1 << SCALE_SUBPEL_BITS,
            step_y: 1 << SCALE_SUBPEL_BITS,
            first_x: 0,
            first_y: 0,
            last_x: ref_w as i32 - 1,
            last_y: ref_h as i32 - 1,
            bit_depth: BitDepth::Ten,
        };
        for (px, py) in phases {
            let params = SubpelPredictParams {
                start_x: (2 + px) << 6,
                start_y: (2 + py) << 6,
                ..base
            };
            let pred0 = pred0_pattern(w, h);
            let pred1 = subpel_predict_block_compound_intermediate(&view, &params).unwrap();
            let stride = w + 5;
            for cwp_weight in [0i16, 1, 7, 8, 9, 15, 16] {
                for depth in [BitDepth::Eight, BitDepth::Ten] {
                    let params = SubpelPredictParams {
                        bit_depth: depth,
                        ..params
                    };
                    let expected = pred0
                        .iter()
                        .zip(&pred1)
                        .map(|(&left, &right)| {
                            blend_compound_average_weighted_sample(left, right, depth, cwp_weight)
                        })
                        .collect::<Vec<_>>();

                    let mut actual = vec![u16::MAX; stride * h + 2];
                    subpel_predict_block_compound_average_strided_into(
                        &view,
                        &params,
                        &pred0,
                        cwp_weight,
                        None,
                        &mut actual,
                        stride,
                    )
                    .unwrap();

                    for row in 0..h {
                        assert_eq!(
                            &actual[row * stride..row * stride + w],
                            &expected[row * w..(row + 1) * w],
                            "{depth:?} w={w} h={h} px={px} py={py} cwp={cwp_weight} row={row}"
                        );
                        assert!(
                            actual[row * stride + w..(row + 1) * stride]
                                .iter()
                                .all(|&sample| sample == u16::MAX)
                        );
                    }
                    assert!(
                        actual[stride * h..]
                            .iter()
                            .all(|&sample| sample == u16::MAX),
                        "trailing sentinel disturbed"
                    );

                    if depth == BitDepth::Eight {
                        let mut actual_u8 = vec![u8::MAX; stride * h];
                        subpel_predict_block_compound_average_strided_into_u8(
                            &view,
                            &params,
                            &pred0,
                            cwp_weight,
                            None,
                            &mut actual_u8,
                            stride,
                        )
                        .unwrap();
                        for row in 0..h {
                            assert_eq!(
                                &actual_u8[row * stride..row * stride + w]
                                    .iter()
                                    .map(|&sample| u16::from(sample))
                                    .collect::<Vec<_>>(),
                                &expected[row * w..(row + 1) * w],
                                "u8 w={w} h={h} px={px} py={py} cwp={cwp_weight} row={row}"
                            );
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn fullpel_compound_average_matches_materialized_predictors() {
    let ref_w = 24usize;
    let ref_h = 20usize;
    let samples0 = build_ref(
        (0..ref_w * ref_h)
            .map(|index| ((index * 17 + 3) % 1024) as u16)
            .collect(),
        ref_w,
        ref_h,
    );
    let samples1 = build_ref(
        (0..ref_w * ref_h)
            .map(|index| ((index * 29 + 11) % 1024) as u16)
            .collect(),
        ref_w,
        ref_h,
    );
    let view0 = ReferencePlaneView::new(&samples0, ref_w, ref_h).unwrap();
    let view1 = ReferencePlaneView::new(&samples1, ref_w, ref_h).unwrap();
    let params0 = SubpelPredictParams {
        interp: InterpolationFilter::EightTapSharp,
        w: 7,
        h: 5,
        start_x: 6 << SCALE_SUBPEL_BITS,
        start_y: 7 << SCALE_SUBPEL_BITS,
        step_x: 1 << SCALE_SUBPEL_BITS,
        step_y: 1 << SCALE_SUBPEL_BITS,
        first_x: 0,
        first_y: 0,
        last_x: ref_w as i32 - 1,
        last_y: ref_h as i32 - 1,
        bit_depth: BitDepth::Ten,
    };
    let params1 = SubpelPredictParams {
        start_x: 9 << SCALE_SUBPEL_BITS,
        start_y: 4 << SCALE_SUBPEL_BITS,
        ..params0
    };
    let pred0 = subpel_predict_block_compound_intermediate(&view0, &params0).unwrap();
    let pred1 = subpel_predict_block_compound_intermediate(&view1, &params1).unwrap();

    for cwp_weight in [8, 12] {
        let expected =
            blend_compound_average_weighted(&pred0, &pred1, params0.bit_depth, cwp_weight).unwrap();
        let stride = params0.w + 3;
        let mut actual = vec![u16::MAX; stride * params0.h];
        assert!(
            subpel_predict_block_compound_average_fullpel_strided_into(
                &view0,
                &params0,
                &view1,
                &params1,
                cwp_weight,
                &mut actual,
                stride,
            )
            .unwrap()
        );
        for row in 0..params0.h {
            assert_eq!(
                &actual[row * stride..row * stride + params0.w],
                &expected[row * params0.w..(row + 1) * params0.w]
            );
            assert!(
                actual[row * stride + params0.w..(row + 1) * stride]
                    .iter()
                    .all(|&sample| sample == u16::MAX)
            );
        }
    }
}

#[test]
fn clipped_horizontal_compound_matches_materialized_predictors() {
    let ref_w = 24usize;
    let ref_h = 16usize;
    let samples0 = build_ref(
        (0..ref_w * ref_h)
            .map(|index| ((index * 17 + 3) % 1024) as u16)
            .collect(),
        ref_w,
        ref_h,
    );
    let samples1 = build_ref(
        (0..ref_w * ref_h)
            .map(|index| ((index * 29 + 11) % 1024) as u16)
            .collect(),
        ref_w,
        ref_h,
    );
    let view0 = ReferencePlaneView::new(&samples0, ref_w, ref_h).unwrap();
    let view1 = ReferencePlaneView::new(&samples1, ref_w, ref_h).unwrap();
    let params0 = SubpelPredictParams {
        interp: InterpolationFilter::EightTapSharp,
        w: 8,
        h: 5,
        start_x: 5 << 6,
        start_y: 4 << SCALE_SUBPEL_BITS,
        step_x: 1 << SCALE_SUBPEL_BITS,
        step_y: 1 << SCALE_SUBPEL_BITS,
        first_x: -8,
        first_y: 0,
        last_x: ref_w as i32 + 7,
        last_y: ref_h as i32 - 1,
        bit_depth: BitDepth::Ten,
    };
    let params1 = SubpelPredictParams {
        interp: InterpolationFilter::EightTapSmooth,
        start_x: ((ref_w as i32 - 4) << SCALE_SUBPEL_BITS) + (13 << 6),
        ..params0
    };
    let expected_params0 = SubpelPredictParams {
        first_x: 0,
        last_x: ref_w as i32 - 1,
        ..params0
    };
    let expected_params1 = SubpelPredictParams {
        first_x: 0,
        last_x: ref_w as i32 - 1,
        ..params1
    };
    let pred0 = subpel_predict_block_compound_intermediate(&view0, &expected_params0).unwrap();
    let pred1 = subpel_predict_block_compound_intermediate(&view1, &expected_params1).unwrap();
    for weight in [8, 12] {
        let expected =
            blend_compound_average_weighted(&pred0, &pred1, BitDepth::Ten, weight).unwrap();
        let stride = params0.w + 3;
        let mut output = vec![u16::MAX; stride * params0.h];
        assert!(
            subpel_predict_block_compound_average_horizontal_strided_into(
                &view0,
                &params0,
                &view1,
                &params1,
                weight,
                &mut output,
                stride,
            )
            .unwrap()
        );
        for row in 0..params0.h {
            assert_eq!(
                &output[row * stride..row * stride + params0.w],
                &expected[row * params0.w..(row + 1) * params0.w],
            );
            assert!(
                output[row * stride + params0.w..(row + 1) * stride]
                    .iter()
                    .all(|&sample| sample == u16::MAX)
            );
        }
    }
}

#[test]
fn clipped_horizontal_compound_rows_match_materialized_predictors() {
    let ref_w = 24usize;
    let ref_h = 8usize;
    let samples0 = build_ref(
        (0..ref_w * ref_h)
            .map(|index| ((index * 17 + 3) % 1024) as u16)
            .collect(),
        ref_w,
        ref_h,
    );
    let samples1 = build_ref(
        (0..ref_w * ref_h)
            .map(|index| ((index * 29 + 11) % 1024) as u16)
            .collect(),
        ref_w,
        ref_h,
    );
    let view0 = ReferencePlaneView::new(&samples0, ref_w, ref_h).unwrap();
    let view1 = ReferencePlaneView::new(&samples1, ref_w, ref_h).unwrap();
    let params0 = SubpelPredictParams {
        interp: InterpolationFilter::EightTapSharp,
        w: 8,
        h: 5,
        start_x: (8 << SCALE_SUBPEL_BITS) + (5 << 6),
        start_y: -2 * (1 << SCALE_SUBPEL_BITS),
        step_x: 1 << SCALE_SUBPEL_BITS,
        step_y: 1 << SCALE_SUBPEL_BITS,
        first_x: 0,
        first_y: -8,
        last_x: ref_w as i32 - 1,
        last_y: ref_h as i32 + 7,
        bit_depth: BitDepth::Ten,
    };
    let params1 = SubpelPredictParams {
        interp: InterpolationFilter::EightTapSmooth,
        start_y: (ref_h as i32 - 2) << SCALE_SUBPEL_BITS,
        ..params0
    };
    let expected_params0 = SubpelPredictParams {
        first_y: 0,
        last_y: ref_h as i32 - 1,
        ..params0
    };
    let expected_params1 = SubpelPredictParams {
        first_y: 0,
        last_y: ref_h as i32 - 1,
        ..params1
    };
    let pred0 = subpel_predict_block_compound_intermediate(&view0, &expected_params0).unwrap();
    let pred1 = subpel_predict_block_compound_intermediate(&view1, &expected_params1).unwrap();
    let expected = blend_compound_average_weighted(&pred0, &pred1, BitDepth::Ten, 12).unwrap();
    let mut actual = vec![u16::MAX; params0.w * params0.h];

    assert!(
        subpel_predict_block_compound_average_horizontal_strided_into(
            &view0,
            &params0,
            &view1,
            &params1,
            12,
            &mut actual,
            params0.w,
        )
        .unwrap()
    );
    assert_eq!(actual, expected);
}

#[test]
fn fused_two_axis_compound_matches_materialized_predictors() {
    let ref_w = 32usize;
    let ref_h = 24usize;
    let samples0 = build_ref(
        (0..ref_w * ref_h)
            .map(|index| ((index * 17 + 3) % 1024) as u16)
            .collect(),
        ref_w,
        ref_h,
    );
    let samples1 = build_ref(
        (0..ref_w * ref_h)
            .map(|index| ((index * 29 + 11) % 1024) as u16)
            .collect(),
        ref_w,
        ref_h,
    );
    let view0 = ReferencePlaneView::new(&samples0, ref_w, ref_h).unwrap();
    let view1 = ReferencePlaneView::new(&samples1, ref_w, ref_h).unwrap();
    for width in [4, 8] {
        let params0 = SubpelPredictParams {
            interp: InterpolationFilter::EightTapSharp,
            w: width,
            h: 8,
            start_x: (8 << SCALE_SUBPEL_BITS) + (5 << 6),
            start_y: (7 << SCALE_SUBPEL_BITS) + (11 << 6),
            step_x: 1 << SCALE_SUBPEL_BITS,
            step_y: 1 << SCALE_SUBPEL_BITS,
            first_x: 0,
            first_y: 0,
            last_x: ref_w as i32 - 1,
            last_y: ref_h as i32 - 1,
            bit_depth: BitDepth::Ten,
        };
        let params1 = SubpelPredictParams {
            interp: InterpolationFilter::EightTapSmooth,
            start_x: (11 << SCALE_SUBPEL_BITS) + (13 << 6),
            start_y: (5 << SCALE_SUBPEL_BITS) + (3 << 6),
            ..params0
        };
        let pred0 = subpel_predict_block_compound_intermediate(&view0, &params0).unwrap();
        let pred1 = subpel_predict_block_compound_intermediate(&view1, &params1).unwrap();
        for weight in [8, 12] {
            let expected =
                blend_compound_average_weighted(&pred0, &pred1, BitDepth::Ten, weight).unwrap();
            let stride = width + 3;
            let mut output = vec![u16::MAX; stride * params0.h];
            let mut scratch = [0i16; 2 * (8 + NUM_TAPS - 1) * 8];
            assert!(
                subpel_predict_block_compound_average_2d_strided_into(
                    &view0,
                    &params0,
                    &view1,
                    &params1,
                    weight,
                    &mut scratch,
                    &mut output,
                    stride,
                )
                .unwrap()
            );
            for row in 0..params0.h {
                assert_eq!(
                    &output[row * stride..row * stride + width],
                    &expected[row * width..(row + 1) * width],
                );
            }
        }
    }
}

#[test]
fn clipped_two_axis_compound_matches_materialized_predictors() {
    let ref_w = 24usize;
    let ref_h = 20usize;
    let samples0 = build_ref(
        (0..ref_w * ref_h)
            .map(|index| ((index * 17 + 3) % 1024) as u16)
            .collect(),
        ref_w,
        ref_h,
    );
    let samples1 = build_ref(
        (0..ref_w * ref_h)
            .map(|index| ((index * 29 + 11) % 1024) as u16)
            .collect(),
        ref_w,
        ref_h,
    );
    let view0 = ReferencePlaneView::new(&samples0, ref_w, ref_h).unwrap();
    let view1 = ReferencePlaneView::new(&samples1, ref_w, ref_h).unwrap();
    for width in [4, 8] {
        for (start0, start1) in [(5i32, 11i32), (-8, -6), (30, 32)] {
            let params0 = SubpelPredictParams {
                interp: InterpolationFilter::EightTapSharp,
                w: width,
                h: 6,
                start_x: (start0 << SCALE_SUBPEL_BITS) + (5 << 6),
                start_y: (4 << SCALE_SUBPEL_BITS) + (11 << 6),
                step_x: 1 << SCALE_SUBPEL_BITS,
                step_y: 1 << SCALE_SUBPEL_BITS,
                first_x: 5,
                first_y: 3,
                last_x: 8,
                last_y: 8,
                bit_depth: BitDepth::Ten,
            };
            let params1 = SubpelPredictParams {
                interp: InterpolationFilter::EightTapSmooth,
                start_x: (start1 << SCALE_SUBPEL_BITS) + (13 << 6),
                start_y: (7 << SCALE_SUBPEL_BITS) + (3 << 6),
                first_x: 9,
                first_y: 6,
                last_x: 13,
                last_y: 11,
                ..params0
            };
            let pred0 = subpel_predict_block_compound_intermediate(&view0, &params0).unwrap();
            let pred1 = subpel_predict_block_compound_intermediate(&view1, &params1).unwrap();
            for weight in [8, 12] {
                let expected =
                    blend_compound_average_weighted(&pred0, &pred1, BitDepth::Ten, weight).unwrap();
                let stride = width + 3;
                let mut output = vec![u16::MAX; stride * params0.h];
                let mut scratch = [0i16; 2 * (8 + NUM_TAPS - 1) * 8];
                assert!(
                    subpel_predict_block_compound_average_2d_strided_into(
                        &view0,
                        &params0,
                        &view1,
                        &params1,
                        weight,
                        &mut scratch,
                        &mut output,
                        stride,
                    )
                    .unwrap()
                );
                for row in 0..params0.h {
                    assert_eq!(
                        &output[row * stride..row * stride + width],
                        &expected[row * width..(row + 1) * width],
                    );
                    assert!(
                        output[row * stride + width..(row + 1) * stride]
                            .iter()
                            .all(|&sample| sample == u16::MAX)
                    );
                }
            }
        }
    }
}

#[test]
fn fullpel_compound_average_preserves_validation_and_fallback() {
    let samples = vec![0u16; 16 * 16];
    let view = ReferencePlaneView::new(&samples, 16, 16).unwrap();
    let valid = full_pel_params(InterpolationFilter::EightTap, 4, 4, 2, 2, 16, 16);
    let mut output = [u16::MAX; 16];

    let fractional = SubpelPredictParams {
        start_x: valid.start_x + (1 << 6),
        ..valid
    };
    assert!(
        !subpel_predict_block_compound_average_fullpel_strided_into(
            &view,
            &fractional,
            &view,
            &valid,
            8,
            &mut output,
            4,
        )
        .unwrap()
    );
    assert_eq!(output, [u16::MAX; 16]);

    let zero_width = SubpelPredictParams { w: 0, ..valid };
    assert!(matches!(
        subpel_predict_block_compound_average_fullpel_strided_into(
            &view,
            &zero_width,
            &view,
            &valid,
            8,
            &mut output,
            4,
        ),
        Err(ReconError::ZeroDimension {
            field: "subpel block width"
        })
    ));

    let negative_step = SubpelPredictParams {
        step_x: -1,
        ..valid
    };
    assert!(matches!(
        subpel_predict_block_compound_average_fullpel_strided_into(
            &view,
            &negative_step,
            &view,
            &valid,
            8,
            &mut output,
            4,
        ),
        Err(ReconError::SubpelNegativeStep {
            step_x: -1,
            step_y: 1024
        })
    ));
}

#[test]
fn one_axis_compound_intermediates_match_independent_reference() {
    let ref_w = 24usize;
    let ref_h = 24usize;
    let samples = build_ref(
        (0..ref_w * ref_h)
            .map(|index| ((index * 73 + (index / ref_w) * 211) % 1024) as u16)
            .collect(),
        ref_w,
        ref_h,
    );
    let view = ReferencePlaneView::new(&samples, ref_w, ref_h).unwrap();
    let filters = [
        InterpolationFilter::EightTap,
        InterpolationFilter::EightTapSmooth,
        InterpolationFilter::EightTapSharp,
        InterpolationFilter::Bilinear,
    ];

    for interp in filters {
        for (w, h) in [(4, 8), (8, 4), (5, 7)] {
            for phase in [1, 8, 15] {
                for vertical_only in [false, true] {
                    let params = SubpelPredictParams {
                        interp,
                        w,
                        h,
                        start_x: (7 << SCALE_SUBPEL_BITS)
                            + if vertical_only { 0 } else { phase << 6 },
                        start_y: (6 << SCALE_SUBPEL_BITS)
                            + if vertical_only { phase << 6 } else { 0 },
                        step_x: 1 << SCALE_SUBPEL_BITS,
                        step_y: 1 << SCALE_SUBPEL_BITS,
                        first_x: 0,
                        first_y: 0,
                        last_x: ref_w as i32 - 1,
                        last_y: ref_h as i32 - 1,
                        bit_depth: BitDepth::Ten,
                    };
                    let expected = reference_subpel_compound(&samples, ref_w, ref_h, &params);
                    let stride = w + 3;
                    let sentinel = i32::MIN;
                    let mut output = vec![sentinel; stride * h];
                    subpel_predict_block_compound_intermediate_into(
                        &view,
                        &params,
                        None,
                        &mut output,
                        stride,
                    )
                    .unwrap();

                    for row in 0..h {
                        assert_eq!(
                            &output[row * stride..row * stride + w],
                            &expected[row * w..(row + 1) * w],
                            "{interp:?} {w}x{h} phase={phase} vertical={vertical_only}"
                        );
                        assert!(
                            output[row * stride + w..(row + 1) * stride]
                                .iter()
                                .all(|&sample| sample == sentinel)
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn compound_intermediate_into_rejects_invalid_destination_without_writes() {
    let ref_w = 8usize;
    let ref_h = 8usize;
    let samples = build_ref(vec![80u16; ref_w * ref_h], ref_w, ref_h);
    let view = ReferencePlaneView::new(&samples, ref_w, ref_h).unwrap();
    let params = full_pel_params(
        InterpolationFilter::EightTap,
        4,
        4,
        2,
        2,
        ref_w as i32,
        ref_h as i32,
    );
    let sentinel = -12345;

    let mut short_stride = vec![sentinel; 64];
    assert_eq!(
        subpel_predict_block_compound_intermediate_into(
            &view,
            &params,
            None,
            &mut short_stride,
            params.w - 1,
        ),
        Err(ReconError::StrideTooSmall {
            stride_samples: params.w - 1,
            storage_width: params.w,
        })
    );
    assert!(short_stride.iter().all(|&sample| sample == sentinel));

    let stride = params.w + 3;
    let required = (params.h - 1) * stride + params.w;
    let mut short_output = vec![sentinel; required - 1];
    assert_eq!(
        subpel_predict_block_compound_intermediate_into(
            &view,
            &params,
            None,
            &mut short_output,
            stride,
        ),
        Err(ReconError::BufferLengthMismatch {
            expected: required,
            actual: required - 1,
        })
    );
    assert!(short_output.iter().all(|&sample| sample == sentinel));
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
    assert_eq!(
        blend_compound_average_weighted_sample(900 * 16, 1000 * 16, BitDepth::Ten, 8,),
        950
    );
    assert_eq!(
        blend_compound_average_weighted_sample(-100, 10_000, BitDepth::Eight, 16),
        0
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
        last_x: ref_w as i32 - 1,
        last_y: ref_h as i32 - 1,
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
    params.step_y = i32::MAX;
    assert!(matches!(
        subpel_predict_block(&view, &params),
        Err(ReconError::ArithmeticOverflow { .. })
    ));
    let mut params = full_pel_params(InterpolationFilter::EightTap, 4, 4, 0, 0, 4, 4);
    params.step_x = i32::MAX;
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

/// The vertical-only pass runs a clip-free interior shape whenever the whole
/// tap-row window is inside `[firstY, lastY]` and the plane, and the clipped
/// walk otherwise. Sweeping block shapes, tap-window regimes and bit depths
/// covers both against the § 7.13.3.18 re-trace.
#[test]
fn vertical_only_matches_independent_reference_across_shapes() {
    let ref_w = 160usize;
    let ref_h = 96usize;
    let mut state: u64 = 0x5150_c0de_1357_9bdf;
    let mut next = || {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (state >> 33) as u32
    };
    let eight: Vec<u16> = (0..ref_w * ref_h).map(|_| (next() % 256) as u16).collect();
    let ten: Vec<u16> = (0..ref_w * ref_h).map(|_| (next() % 1024) as u16).collect();
    let eight = build_ref(eight, ref_w, ref_h);
    let ten = build_ref(ten, ref_w, ref_h);

    let filters = [
        InterpolationFilter::EightTap,
        InterpolationFilter::EightTapSmooth,
        InterpolationFilter::EightTapSharp,
        InterpolationFilter::Bilinear,
    ];
    let widths = [2usize, 3, 4, 5, 8, 9, 16, 17, 32, 64, 128];
    let heights = [2usize, 4, 5, 8, 16, 32, 64];
    let mut case = 0usize;
    for &w in &widths {
        for &h in &heights {
            let bottom = (ref_h - h) as i32;
            for &base_y in &[-2, 0, 3, bottom / 2, bottom - 4, bottom] {
                for pad in [0i32, 4, 3] {
                    case += 1;
                    let phase_y = 1 + (case % 15) as i32;
                    let (bit_depth, samples) = if case.is_multiple_of(2) {
                        (BitDepth::Eight, &eight)
                    } else {
                        (BitDepth::Ten, &ten)
                    };
                    let (first_y, last_y) = if pad == 0 {
                        (0, ref_h as i32 - 1)
                    } else {
                        (
                            (base_y - pad + 1).clamp(0, ref_h as i32 - 1),
                            (base_y + h as i32 + pad - 1).clamp(0, ref_h as i32 - 1),
                        )
                    };
                    let params = SubpelPredictParams {
                        interp: filters[case % filters.len()],
                        w,
                        h,
                        start_x: 8 << SCALE_SUBPEL_BITS,
                        start_y: (base_y << SCALE_SUBPEL_BITS) + (phase_y << 6),
                        step_x: 1 << SCALE_SUBPEL_BITS,
                        step_y: 1 << SCALE_SUBPEL_BITS,
                        first_x: 0,
                        first_y,
                        last_x: ref_w as i32 - 1,
                        last_y,
                        bit_depth,
                    };
                    let view = ReferencePlaneView::new(samples, ref_w, ref_h).unwrap();
                    let out = subpel_predict_block(&view, &params).unwrap();
                    let want = reference_subpel(samples, ref_w, ref_h, &params);
                    assert_eq!(
                        out, want,
                        "case {case} w={w} h={h} by={base_y} pad={pad} py={phase_y}"
                    );
                }
            }
        }
    }
}
