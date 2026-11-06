// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::unwrap_used)]

use super::*;
use crate::math::round2;

#[allow(clippy::too_many_arguments)]
fn params(
    boundary: usize,
    q_thr: i32,
    max_width_neg: usize,
    max_width_pos: usize,
    q_thresh_mult: i32,
    w_mult_neg: i32,
    w_mult_pos: i32,
    prev_lossless: bool,
    curr_lossless: bool,
) -> DeblockSampleFilter {
    DeblockSampleFilter {
        boundary,
        q_thr,
        max_width_neg,
        max_width_pos,
        q_thresh_mult,
        w_mult_neg,
        w_mult_pos,
        prev_lossless,
        curr_lossless,
        bit_depth: BitDepth::Eight,
    }
}

fn reference(line: &mut [u8], p: &DeblockSampleFilter) {
    let width = p.max_width_neg.max(p.max_width_pos);
    let q0 = i64::from(line[p.boundary]);
    let q1 = i64::from(line[p.boundary + 1]);
    let p0 = i64::from(line[p.boundary - 1]);
    let p1 = i64::from(line[p.boundary - 2]);
    let bound = (i64::from(p.q_thr) * i64::from(p.q_thresh_mult)).max(0);
    let delta = ((p1 - q1 + 3 * (q0 - p0)) * 4).clamp(-bound, bound);
    let dn = delta * i64::from(p.w_mult_neg);
    let dp = delta * i64::from(p.w_mult_pos);
    let max = i64::from(BitDepth::Eight.max_sample());
    for i in 0..width {
        let diff_pos = round2(dp * (p.max_width_pos as i64 - i as i64), 3 + DF_SHIFT);
        if !p.curr_lossless {
            let idx = p.boundary + i;
            line[idx] = (i64::from(line[idx]) - diff_pos).clamp(0, max) as u8;
        }
        if i < p.max_width_neg && !p.prev_lossless {
            let diff_neg = round2(dn * (p.max_width_neg as i64 - i as i64), 3 + DF_SHIFT);
            let idx = p.boundary - 1 - i;
            line[idx] = (i64::from(line[idx]) + diff_neg).clamp(0, max) as u8;
        }
    }
}

#[test]
fn matches_hand_computed_symmetric_width_2() {
    let mut line = [10u8, 20, 60, 50];
    deblock_sample_filter(&mut line, &params(2, 100, 2, 2, 25, 51, 51, false, false)).unwrap();
    assert_eq!(line, [18, 36, 44, 42]);
}

#[test]
fn matches_reference_across_configs() {
    let base = [40u8, 60, 50, 70, 55, 80, 45, 90, 35, 100];
    let configs = [
        params(4, 80, 3, 2, 19, 37, 51, false, false),
        params(4, 200, 2, 3, 19, 51, 37, false, false), // maxWidthPos > maxWidthNeg
        params(5, 20, 4, 4, 19, 28, 28, false, false),  // small q_thr clamps deltaM2
        params(4, 80, 2, 2, 25, 51, 51, true, false),   // prev lossless: p-side skipped
        params(4, 80, 2, 2, 25, 51, 51, false, true),   // curr lossless: q-side skipped
    ];
    for p in &configs {
        let mut produced = base;
        deblock_sample_filter(&mut produced, p).unwrap();
        let mut expected = base;
        reference(&mut expected, p);
        assert_eq!(produced, expected, "config {p:?}");
    }
}

#[test]
fn strided_sample_filter_matches_contiguous_line() {
    let source = [
        40u16, 60, 50, 70, 55, 80, 45, 90, 35, 100, 30, 110, 25, 120, 20, 130, 15,
    ];
    let params = DeblockSampleFilter {
        boundary: 8,
        bit_depth: BitDepth::Ten,
        ..params(8, 80, 6, 8, 17, 20, 15, false, false)
    };
    let mut expected = source;
    deblock_sample_filter(&mut expected, &params).unwrap();

    let stride = 23;
    let boundary = 8 * stride + 4;
    let mut plane = vec![0u16; 17 * stride];
    for (index, sample) in source.into_iter().enumerate() {
        let position = if index < 8 {
            boundary - (8 - index) * stride
        } else {
            boundary + (index - 8) * stride
        };
        plane[position] = sample;
    }
    deblock_sample_filter_strided(
        &mut plane,
        NonZeroUsize::new(stride).unwrap(),
        &DeblockSampleFilter { boundary, ..params },
    )
    .unwrap();
    for (index, expected) in expected.into_iter().enumerate() {
        let position = if index < 8 {
            boundary - (8 - index) * stride
        } else {
            boundary + (index - 8) * stride
        };
        assert_eq!(plane[position], expected);
    }
}

#[test]
fn four_lane_strided_filter_matches_individual_lanes() {
    let stride = 32;
    let boundary = 8 * stride + 4;
    let mut source = vec![0u16; 17 * stride];
    for row in 0..17 {
        for lane in 0..4 {
            source[row * stride + 4 + lane] = (40 + row * 7 + lane * 3) as u16;
        }
    }
    let cases = [
        params(boundary, 80, 6, 8, 17, 20, 15, false, false),
        params(boundary, i32::MAX, 6, 8, i32::MAX, 0, 0, false, false),
        params(
            boundary,
            i32::MAX,
            6,
            8,
            i32::MAX,
            i32::MAX,
            i32::MAX,
            false,
            false,
        ),
    ];
    for params in cases {
        let mut expected = source.clone();
        for lane in 0..4 {
            deblock_sample_filter_strided(
                &mut expected,
                NonZeroUsize::new(stride).unwrap(),
                &DeblockSampleFilter {
                    boundary: boundary + lane,
                    ..params
                },
            )
            .unwrap();
        }
        let mut actual = source.clone();
        deblock_sample_filter_strided_4(
            &mut actual,
            NonZeroUsize::new(stride).unwrap(),
            NonZeroUsize::new(1).unwrap(),
            &params,
        )
        .unwrap();
        assert_eq!(actual, expected);
    }
}

#[test]
fn four_row_contiguous_filter_matches_individual_rows() {
    let lane_stride = 32;
    let boundary = 8;
    let mut source = vec![0u16; 4 * lane_stride];
    for row in 0..4 {
        for col in 0..16 {
            source[row * lane_stride + col] = (40 + row * 17 + col * 7) as u16;
        }
    }
    for width in [4, 8] {
        let params = DeblockSampleFilter {
            boundary,
            bit_depth: BitDepth::Ten,
            ..params(boundary, 80, width, width, 17, 20, 15, false, false)
        };
        let mut expected = source.clone();
        for row in 0..4 {
            deblock_sample_filter(
                &mut expected,
                &DeblockSampleFilter {
                    boundary: boundary + row * lane_stride,
                    ..params
                },
            )
            .unwrap();
        }
        let mut actual = source.clone();
        deblock_sample_filter_strided_4(
            &mut actual,
            NonZeroUsize::new(1).unwrap(),
            NonZeroUsize::new(lane_stride).unwrap(),
            &params,
        )
        .unwrap();
        assert_eq!(actual, expected, "width {width}");
    }
}

#[test]
fn lossless_sides_are_untouched() {
    let base = [40u8, 60, 50, 70, 55, 80];
    let mut both = base;
    deblock_sample_filter(&mut both, &params(3, 100, 2, 2, 25, 51, 51, true, true)).unwrap();
    assert_eq!(both, base);
}

#[test]
fn clip1_clamps_to_bit_depth() {
    let mut line = [10u8, 250, 5, 240, 0, 255];
    deblock_sample_filter(
        &mut line,
        &params(3, 10_000, 2, 2, 32, 85, 85, false, false),
    )
    .unwrap();
    let mut reference_line = [10u8, 250, 5, 240, 0, 255];
    reference(
        &mut reference_line,
        &params(3, 10_000, 2, 2, 32, 85, 85, false, false),
    );
    assert_eq!(line, reference_line);
    let mut wide = [10u16, 1000, 5, 1020, 0, 1023];
    deblock_sample_filter(
        &mut wide,
        &DeblockSampleFilter {
            bit_depth: BitDepth::Ten,
            ..params(3, 10_000, 2, 2, 32, 85, 85, false, false)
        },
    )
    .unwrap();
    assert!(wide.iter().all(|&v| v <= 1023));
}

#[test]
fn rejects_invalid_width_and_short_line() {
    let mut line = [0u8; 8];
    assert!(matches!(
        deblock_sample_filter(&mut line, &params(4, 0, 0, 2, 19, 51, 51, false, false)),
        Err(ReconError::DeblockFilterInvalidWidth {
            max_width_neg: 0,
            max_width_pos: 2
        })
    ));
    assert!(matches!(
        deblock_sample_filter(&mut line, &params(4, 0, 2, 9, 19, 51, 51, false, false)),
        Err(ReconError::DeblockFilterInvalidWidth { .. })
    ));
    assert!(matches!(
        deblock_sample_filter(&mut line, &params(1, 0, 2, 2, 19, 51, 51, false, false)),
        Err(ReconError::DeblockFilterLineTooShort { .. })
    ));
    let mut short = [0u8; 5];
    assert!(matches!(
        deblock_sample_filter(&mut short, &params(4, 0, 2, 2, 19, 51, 51, false, false)),
        Err(ReconError::DeblockFilterLineTooShort { .. })
    ));
}

#[test]
fn is_total_for_extreme_inputs() {
    for &(neg, pos) in &[(1usize, 8usize), (8, 1), (8, 8)] {
        let mut line = [0u8; 24];
        for (i, s) in line.iter_mut().enumerate() {
            *s = if i % 2 == 0 { 0 } else { 255 };
        }
        deblock_sample_filter(
            &mut line,
            &params(10, i32::MAX, neg, pos, 32, 85, 85, false, false),
        )
        .unwrap();
    }
}

#[test]
fn max_width_covers_every_spec_branch() {
    assert_eq!(deblock_filter_max_width(4, false, false), (1, 1));
    assert_eq!(deblock_filter_max_width(2, true, false), (1, 1)); // <= 4
    assert_eq!(deblock_filter_max_width(8, false, false), (3, 3));
    assert_eq!(deblock_filter_max_width(8, true, false), (3, 3));
    assert_eq!(deblock_filter_max_width(16, false, false), (6, 6)); // luma
    assert_eq!(deblock_filter_max_width(16, true, false), (4, 4)); // chroma
    assert_eq!(deblock_filter_max_width(32, false, false), (8, 8)); // luma > 16
    assert_eq!(deblock_filter_max_width(64, true, false), (4, 4)); // chroma > 16

    assert_eq!(deblock_filter_max_width(32, false, true), (6, 8)); // luma: cap 6
    assert_eq!(deblock_filter_max_width(64, true, true), (2, 4)); // chroma: cap 2
    assert_eq!(deblock_filter_max_width(8, true, true), (2, 3)); // chroma: cap 2 < 3
    assert_eq!(deblock_filter_max_width(4, false, true), (1, 1)); // cap 6 but pos 1
}

#[test]
fn side_threshold_index_clips_to_table_range() {
    assert_eq!(deblock_side_threshold_index(10, BitDepth::Eight), 10);
    assert_eq!(deblock_side_threshold_index(0, BitDepth::Eight), 0);
    assert_eq!(deblock_side_threshold_index(400, BitDepth::Eight), 295);
    assert_eq!(deblock_side_threshold_index(10, BitDepth::Ten), 0); // 10 - 48 < 0
    assert_eq!(deblock_side_threshold_index(100, BitDepth::Ten), 52); // 100 - 48
}

#[test]
fn adaptive_filter_strength_matches_spec() {
    assert_eq!(
        deblock_adaptive_filter_strength(40, 100, BitDepth::Eight).1,
        3
    ); // (100+16)>>5
    assert_eq!(
        deblock_adaptive_filter_strength(40, -16, BitDepth::Eight).1,
        0
    ); // (0)>>5
    assert_eq!(
        deblock_adaptive_filter_strength(40, 1678, BitDepth::Eight).1,
        52
    ); // (1694)>>5
    assert_eq!(
        deblock_adaptive_filter_strength(40, 100, BitDepth::Ten).1,
        13
    ); // (104)>>3

    for &(lvl, bd) in &[
        (40u32, BitDepth::Eight),
        (128, BitDepth::Eight),
        (200, BitDepth::Ten),
    ] {
        let expected = (((i64::from(quantizer_value(lvl, 0, bd)) + 4) >> 3) >> 6) as i32;
        assert_eq!(deblock_adaptive_filter_strength(lvl, 0, bd).0, expected);
    }
}

/// The § 9.2 `Q_First` array (`docs/spec/.../03-symbols.md`, DBL_REG_DECIS_LEN).
const Q_FIRST: [i32; DBL_REG_DECIS_LEN] = [45, 43, 40, 35, 32, 32, 32, 32, 32];

fn choice(
    boundary: usize,
    q_thr: i32,
    side_thr: i32,
    max_width_neg: usize,
    max_width_pos: usize,
) -> DeblockFilterChoice {
    DeblockFilterChoice {
        boundary,
        q_thr,
        side_thr,
        max_width_pos,
        max_width_neg,
        q_first: Q_FIRST,
    }
}

#[allow(clippy::too_many_arguments)]
fn reference_choice(
    s: &[u16],
    t: &[u16],
    boundary: usize,
    q_thr: i64,
    side_thr: i64,
    max_width_neg: usize,
    max_width_pos: usize,
    q_first: &[i32; DBL_REG_DECIS_LEN],
) -> usize {
    if q_thr == 0 || side_thr == 0 {
        return 0;
    }
    let g = |a: &[u16], k: i64| -> i64 { i64::from(a[(boundary as i64 + k) as usize]) };
    let mut sd = [0i64; 4]; // index dist + 2, for dist in -2..=1
    for dist in -2i64..=1 {
        let ds = (g(s, dist - 1) - 2 * g(s, dist) + g(s, dist + 1)).abs();
        let dt = (g(t, dist - 1) - 2 * g(t, dist) + g(t, dist + 1)).abs();
        sd[(dist + 2) as usize] = (ds + dt + 1) >> 1;
    }
    let sdv = |d: i64| sd[(d + 2) as usize];
    if sdv(-2) > side_thr || sdv(1) > side_thr {
        return 0;
    }
    if max_width_pos == 1 {
        return 1;
    }
    if sdv(-2) > (side_thr >> 2) || sdv(1) > (side_thr >> 2) {
        return 1;
    }
    if sdv(-1) + sdv(0) > q_thr * 4 {
        return 1;
    }
    if sdv(-2) > (side_thr >> 3) || sdv(1) > (side_thr >> 3) {
        return 2;
    }
    if sdv(-1) + sdv(0) > q_thr * 3 {
        return 2;
    }
    let end_thr = (side_thr * 3) >> 4;
    if max_width_neg > 2 {
        let ds = (g(s, -1) - g(s, -4) - 3 * (g(s, -1) - g(s, -2))).abs();
        let dt = (g(t, -1) - g(t, -4) - 3 * (g(t, -1) - g(t, -2))).abs();
        if ((ds + dt + 1) >> 1) > end_thr {
            return 2;
        }
    }
    let ds = (g(s, 0) - g(s, 3) - 3 * (g(s, 0) - g(s, 1))).abs();
    let dt = (g(t, 0) - g(t, 3) - 3 * (g(t, 0) - g(t, 1))).abs();
    if ((ds + dt + 1) >> 1) > end_thr {
        return 2;
    }
    if max_width_pos == 3 {
        return 3;
    }
    let transition = (sdv(-1) + sdv(0)) << 4;
    let mut prev_dist = 3usize;
    let mut dist = 4usize;
    while dist <= max_width_pos {
        let q_thr4 = q_thr * i64::from(q_first[dist - 4]);
        let end_thr4 = (side_thr * dist as i64) >> 4;
        if transition > q_thr4 {
            return prev_dist;
        }
        let dist2 = dist.min(7) as i64;
        if max_width_neg >= dist2 as usize {
            let ds = (g(s, -1) - g(s, -dist2 - 1) - dist2 * (g(s, -1) - g(s, -2))).abs();
            let dt = (g(t, -1) - g(t, -dist2 - 1) - dist2 * (g(t, -1) - g(t, -2))).abs();
            if ((ds + dt + 1) >> 1) > end_thr4 {
                return prev_dist;
            }
        }
        let ds = (g(s, 0) - g(s, dist2) - dist2 * (g(s, 0) - g(s, 1))).abs();
        let dt = (g(t, 0) - g(t, dist2) - dist2 * (g(t, 0) - g(t, 1))).abs();
        if ((ds + dt + 1) >> 1) > end_thr4 {
            return prev_dist;
        }
        prev_dist = dist;
        dist += 2;
    }
    max_width_pos
}

#[test]
fn filter_choice_zero_threshold_returns_zero() {
    let line = [128u16; 17];
    assert_eq!(
        deblock_filter_choice(&line, &line, &choice(8, 0, 500, 8, 8)).unwrap(),
        0
    );
    assert_eq!(
        deblock_filter_choice(&line, &line, &choice(8, 50, 0, 8, 8)).unwrap(),
        0
    );
}

#[test]
fn filter_choice_flat_returns_full_width() {
    let line = [128u16; 17];
    for pos in [1usize, 3, 4, 6, 8] {
        let neg = pos.min(6);
        let got = deblock_filter_choice(&line, &line, &choice(8, 10, 2000, neg, pos)).unwrap();
        assert_eq!(got, pos, "flat width pos={pos}");
    }
}

#[test]
fn filter_choice_high_curvature_returns_zero() {
    let mut line = [128u16; 17];
    line[8 - 2] = 320; // boundary = 8 -> index 6 is s[-2]
    assert_eq!(
        deblock_filter_choice(&line, &line, &choice(8, 50, 100, 8, 8)).unwrap(),
        0
    );
}

#[test]
fn filter_choice_invalid_width_and_short_line_error() {
    let line = [128u16; 17];
    assert!(matches!(
        deblock_filter_choice(&line, &line, &choice(8, 50, 500, 8, 9)),
        Err(ReconError::DeblockFilterInvalidWidth { .. })
    ));
    let short = [128u16; 10];
    assert!(matches!(
        deblock_filter_choice(&short, &short, &choice(8, 50, 500, 8, 8)),
        Err(ReconError::DeblockFilterLineTooShort { .. })
    ));
}

#[test]
fn filter_choice_matches_independent_reference() {
    let mut state = 0x0123_4567_89ab_cdefu64;
    let mut next = |bound: u32| -> u32 {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        ((state >> 33) as u32) % bound
    };
    let widths = [1usize, 2, 3, 4, 6, 8];
    let thresholds = [0i32, 5, 50, 500, 5000];
    let mut checked = 0u32;
    for _ in 0..4000 {
        let s: Vec<u16> = (0..17).map(|_| next(512) as u16).collect();
        let t: Vec<u16> = (0..17).map(|_| next(512) as u16).collect();
        let max_width_pos = widths[next(widths.len() as u32) as usize];
        let neg_choice = widths[next(widths.len() as u32) as usize];
        let max_width_neg = neg_choice.min(max_width_pos).max(1);
        let q_thr = thresholds[next(thresholds.len() as u32) as usize];
        let side_thr = thresholds[next(thresholds.len() as u32) as usize];
        let params = choice(8, q_thr, side_thr, max_width_neg, max_width_pos);
        let got = deblock_filter_choice(&s, &t, &params).unwrap();
        let expected = reference_choice(
            &s,
            &t,
            8,
            i64::from(q_thr),
            i64::from(side_thr),
            max_width_neg,
            max_width_pos,
            &Q_FIRST,
        );
        assert_eq!(
            got, expected,
            "pos={max_width_pos} neg={max_width_neg} q={q_thr} side={side_thr} s={s:?} t={t:?}"
        );
        checked += 1;
    }
    assert_eq!(checked, 4000);
}

#[test]
fn strided_filter_choice_matches_contiguous_lines() {
    let s = [
        40u16, 60, 50, 70, 55, 80, 45, 90, 35, 100, 30, 110, 25, 120, 20, 130, 15,
    ];
    let t = [
        42u16, 58, 53, 68, 57, 77, 49, 86, 39, 96, 34, 106, 29, 116, 24, 126, 19,
    ];
    let params = choice(8, 80, 200, 6, 8);
    let expected = deblock_filter_choice(&s, &t, &params).unwrap();

    let stride = 23;
    let boundary = 8 * stride + 4;
    let last_boundary = boundary + 3;
    let mut plane = vec![0u16; 17 * stride];
    for index in 0..s.len() {
        let offset = if index < 8 {
            -((8 - index) as isize)
        } else {
            (index - 8) as isize
        };
        let row = boundary
            .checked_add_signed(offset * stride as isize)
            .unwrap();
        plane[row] = s[index];
        plane[row + 3] = t[index];
    }
    let got = deblock_filter_choice_strided(
        &plane,
        last_boundary,
        NonZeroUsize::new(stride).unwrap(),
        &DeblockFilterChoice { boundary, ..params },
    )
    .unwrap();
    assert_eq!(got, expected);
}
