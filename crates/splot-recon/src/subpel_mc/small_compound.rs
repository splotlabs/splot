// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Fused § 7.13.3.18 prediction of both references of a 4x4 compound cell
//! and their § 7.13.3.16 COMPOUND_AVERAGE blend.
//!
//! A 4-sample axis uses a 4-tap small-block filter whose taps sit at indices
//! 2..=5, so a row is one 8-sample window and every intermediate stays in
//! registers. A zero phase skips its pass: `Round2(128 * s, 3) == s << 4` and
//! `Round2(h << 7, 7) == h` hold exactly, so each phase class keeps the
//! two-pass arithmetic.

use super::*;
use std::simd::simd_swizzle;

type Row = Simd<i32, 4>;
type Taps = Simd<i16, 4>;

/// Taps 2..=5 of every `Subpel_Filters` row; only the 4-tap rows are read.
const SMALL_TAPS: [[[i16; 4]; NUM_PHASES]; NUM_FILTER_TYPES] = {
    let mut taps = [[[0; 4]; NUM_PHASES]; NUM_FILTER_TYPES];
    let mut filter = 0;
    while filter < NUM_FILTER_TYPES {
        let mut phase = 0;
        while phase < NUM_PHASES {
            let mut tap = 0;
            while tap < 4 {
                taps[filter][phase][tap] = SUBPEL_FILTERS[filter][phase][tap + 2] as i16;
                tap += 1;
            }
            phase += 1;
        }
        filter += 1;
    }
    taps
};

/// One reference of a cell whose nonzero horizontal taps need no clamping.
struct Cell<'a, T> {
    samples: &'a [T],
    /// Offsets of the clamped source rows `y0 - 1 ..= y0 + 5` at the first
    /// column the cell reads.
    rows: [usize; 7],
    h_taps: Taps,
    v_taps: Taps,
    horizontal: bool,
    vertical: bool,
}

#[allow(clippy::inline_always, reason = "measured subpel hot path")]
#[inline(always)]
fn cell<'a, T: ReconSample>(
    reference: &ReferencePlaneView<'a, T>,
    params: &SubpelPredictParams,
) -> Option<Cell<'a, T>> {
    let filter = params.interp.pass_index(SMALL_BLOCK_DIM) as usize;
    let h_phase = ((params.start_x >> 6) & SUBPEL_MASK) as usize;
    let v_phase = ((params.start_y >> 6) & SUBPEL_MASK) as usize;
    let (tap_start, tap_end) = ACTIVE_TAP_SPANS[filter][h_phase];
    let x0 = params.start_x >> SCALE_SUBPEL_BITS;
    if x0 + tap_start as i32 - 3 < params.first_x || x0 + tap_end as i32 - 1 > params.last_x {
        return None;
    }
    let column = usize::try_from(x0 - i32::from(h_phase != 0)).ok()?;
    let rows = (Simd::<i32, 8>::splat((params.start_y >> SCALE_SUBPEL_BITS) - 1)
        + Simd::from_array([0, 1, 2, 3, 4, 5, 6, 7]))
    .simd_max(Simd::splat(params.first_y))
    .simd_min(Simd::splat(params.last_y));
    Some(Cell {
        samples: reference.samples,
        rows: core::array::from_fn(|row| rows[row] as usize * reference.stride + column),
        h_taps: horizontal_taps::<T>(Simd::from_array(SMALL_TAPS[filter][h_phase])),
        v_taps: Simd::from_array(SMALL_TAPS[filter][v_phase]),
        horizontal: h_phase != 0,
        vertical: v_phase != 0,
    })
}

/// Halves the even taps of an 8-bit row, which keeps its sums in `i16` as in
/// `slide::intermediate_taps`.
#[allow(clippy::inline_always, reason = "measured subpel hot path")]
#[inline(always)]
fn horizontal_taps<T: ReconSample>(taps: Taps) -> Taps {
    if T::MAX_VALUE > u16::from(u8::MAX) {
        taps
    } else {
        taps >> 1
    }
}

#[allow(clippy::inline_always, reason = "measured subpel hot path")]
#[inline(always)]
fn horizontal<T: ReconSample>(window: &[T], taps: Taps) -> Simd<i16, 4> {
    let samples = reference_lanes::<8, T>(window, 0).cast::<i16>();
    let windows = [
        simd_swizzle!(samples, [0, 1, 2, 3]),
        simd_swizzle!(samples, [1, 2, 3, 4]),
        simd_swizzle!(samples, [2, 3, 4, 5]),
        simd_swizzle!(samples, [3, 4, 5, 6]),
    ];
    if T::MAX_VALUE > u16::from(u8::MAX) {
        let mut sum = Row::splat(1 << (INTER_ROUND0 - 1));
        for (tap, window) in windows.into_iter().enumerate() {
            sum = tap_mac(sum, window, i32::from(taps[tap]));
        }
        return (sum >> INTER_ROUND0 as i32).cast();
    }
    let mut half = Simd::splat(1 << (INTER_ROUND0 - 2));
    for (tap, window) in windows.into_iter().enumerate() {
        half += window * Simd::splat(taps[tap]);
    }
    half >> (INTER_ROUND0 - 1) as i16
}

#[allow(clippy::inline_always, reason = "measured subpel hot path")]
#[inline(always)]
fn vertical(rows: &[Simd<i16, 4>], taps: Taps, shift: u32) -> Row {
    let mut sum = Row::splat(1 << (shift - 1));
    for (tap, &row) in rows[..4].iter().enumerate() {
        sum = tap_mac(sum, row, i32::from(taps[tap]));
    }
    sum >> shift as i32
}

/// The cell's compound predictor rows, `Round2(.., InterRound1)` of the
/// two-pass convolution.
#[allow(clippy::inline_always, reason = "measured subpel hot path")]
#[inline(always)]
fn predict_reference<T: ReconSample>(cell: &Cell<'_, T>) -> Option<[Row; 4]> {
    let window = |row: usize| cell.samples.get(cell.rows[row]..cell.rows[row] + 8);
    let mut rows = [Simd::splat(0); 7];
    let mut out = [Row::splat(0); 4];
    match (cell.horizontal, cell.vertical) {
        (true, true) => {
            for (row, value) in rows.iter_mut().enumerate() {
                *value = horizontal(window(row)?, cell.h_taps);
            }
            for (i, out) in out.iter_mut().enumerate() {
                *out = vertical(&rows[i..], cell.v_taps, INTER_ROUND1_COMPOUND);
            }
        }
        (true, false) => {
            for (i, out) in out.iter_mut().enumerate() {
                *out = horizontal(window(i + 1)?, cell.h_taps).cast();
            }
        }
        (false, true) => {
            for (row, value) in rows.iter_mut().enumerate() {
                *value = reference_lanes::<4, T>(window(row)?, 0).cast();
            }
            // `Round2(s << 4, 7) == Round2(s, 3)`: the zero-phase horizontal pass.
            for (i, out) in out.iter_mut().enumerate() {
                *out = vertical(&rows[i..], cell.v_taps, INTER_ROUND0);
            }
        }
        (false, false) => {
            for (i, out) in out.iter_mut().enumerate() {
                *out = reference_lanes::<4, T>(window(i + 1)?, 0).cast::<i32>()
                    << (FILTER_BITS - INTER_ROUND0) as i32;
            }
        }
    }
    Some(out)
}

/// Writes the blended 4x4 cell and returns `true`, or returns `false` without
/// writing when a nonzero horizontal tap of either reference is clamped.
///
/// The parameters are plane-bounded and validated by the caller, and `output`
/// holds the strided 4x4 rectangle.
#[allow(clippy::too_many_arguments)]
#[inline(never)]
pub(super) fn predict<T: ReconSample, O: ReconSample>(
    reference0: &ReferencePlaneView<'_, T>,
    params0: &SubpelPredictParams,
    reference1: &ReferencePlaneView<'_, T>,
    params1: &SubpelPredictParams,
    cwp_weight: i16,
    output: &mut [O],
    output_stride: usize,
) -> bool {
    let (Some(cell0), Some(cell1)) = (cell(reference0, params0), cell(reference1, params1)) else {
        return false;
    };
    let (Some(pred0), Some(pred1)) = (predict_reference(&cell0), predict_reference(&cell1)) else {
        return false;
    };
    let forward = Row::splat(i32::from(cwp_weight));
    let backward = Row::splat(16 - i32::from(cwp_weight));
    let max_sample = Row::splat(i32::from(params0.bit_depth.max_sample().min(O::MAX_VALUE)));
    let shift = 4 + compound_inter_post_round();
    let blended = core::array::from_fn::<_, 4, _>(|i| {
        ((pred0[i] * forward + pred1[i] * backward + Row::splat(1 << (shift - 1))) >> shift as i32)
            .simd_max(Row::splat(0))
            .simd_min(max_sample)
    });
    if let Some(output) = O::u16_slice_mut(output) {
        for (i, row) in blended.into_iter().enumerate() {
            output[i * output_stride..][..4].copy_from_slice(&row.cast::<u16>().to_array()); // splot-copy-ok: publish four blended SIMD prediction lanes
        }
        return true;
    }
    if let Some(output) = O::u8_slice_mut(output) {
        for (i, row) in blended.into_iter().enumerate() {
            output[i * output_stride..][..4].copy_from_slice(&row.cast::<u8>().to_array()); // splot-copy-ok: publish four blended SIMD prediction lanes
        }
        return true;
    }
    false
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    struct Lcg(u64);

    impl Lcg {
        fn below(&mut self, bound: usize) -> usize {
            self.0 = self
                .0
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            (self.0 >> 33) as usize % bound
        }
    }

    const FILTERS: [InterpolationFilter; 4] = [
        InterpolationFilter::EightTap,
        InterpolationFilter::EightTapSmooth,
        InterpolationFilter::EightTapSharp,
        InterpolationFilter::Bilinear,
    ];

    fn random_params(
        rng: &mut Lcg,
        interp: InterpolationFilter,
        size: usize,
        (width, height): (usize, usize),
        bit_depth: BitDepth,
    ) -> SubpelPredictParams {
        let mut phase = || {
            if rng.below(3) == 0 {
                0
            } else {
                rng.below(16) as i32
            }
        };
        let (phase_x, phase_y) = (phase(), phase());
        let x = rng.below(width + 12) as i32 - 6;
        let y = rng.below(height + 12) as i32 - 6;
        let mut window = |extent: usize| {
            if rng.below(3) == 0 {
                let first = rng.below(extent / 2);
                (first as i32, (first + 1 + rng.below(extent / 2)) as i32)
            } else {
                (0, extent as i32 - 1)
            }
        };
        let ((first_x, last_x), (first_y, last_y)) = (window(width), window(height));
        SubpelPredictParams {
            interp,
            w: size,
            h: size,
            start_x: (x << SCALE_SUBPEL_BITS) + (phase_x << 6),
            start_y: (y << SCALE_SUBPEL_BITS) + (phase_y << 6),
            step_x: 1 << SCALE_SUBPEL_BITS,
            step_y: 1 << SCALE_SUBPEL_BITS,
            first_x,
            first_y,
            last_x,
            last_y,
            bit_depth,
        }
    }

    /// Compares random `size x size` compound cells of every phase class with
    /// the two-call path, through the fast dispatch and the fused kernel.
    fn check_cells<T: ReconSample>(bit_depth: BitDepth, size: usize) {
        let (width, height) = (40usize, 36usize);
        let max = bit_depth.max_sample();
        let mut rng = Lcg(u64::from(max) * 31 + size as u64);
        let mut plane = || {
            (0..width * height)
                .map(|_| T::try_from_u16(rng.below(usize::from(max) + 1) as u16).unwrap())
                .collect::<Vec<T>>()
        };
        let planes = [plane(), plane()];
        let views =
            [0, 1].map(|index| ReferencePlaneView::new(&planes[index], width, height).unwrap());
        let stride = size + 3;
        let sentinel = T::try_from_u16(max / 3).unwrap();
        let (mut fused, mut declined) = (0, 0);
        for case in 0..3000 {
            let interp = FILTERS[case % FILTERS.len()];
            let params =
                [(); 2].map(|()| random_params(&mut rng, interp, size, (width, height), bit_depth));
            let weight = [8, 12, 4, 10][case % 4];
            let pred = [0, 1].map(|index| {
                subpel_predict_block_compound_intermediate(&views[index], &params[index]).unwrap()
            });
            let expected =
                blend_compound_average_weighted(&pred[0], &pred[1], bit_depth, weight).unwrap();
            let mut output = vec![sentinel; stride * size];
            assert!(
                subpel_predict_block_compound_average_fast_validated_strided_into(
                    &views[0],
                    &params[0],
                    &views[1],
                    &params[1],
                    weight,
                    &mut [],
                    &mut output,
                    stride,
                )
                .unwrap()
            );
            let bounded = [0, 1].map(|index| plane_bounded(&views[index], &params[index]));
            let mut direct = vec![sentinel; stride * size];
            let accepted = predict(
                &views[0],
                &bounded[0],
                &views[1],
                &bounded[1],
                weight,
                &mut direct,
                stride,
            );
            if accepted {
                fused += 1;
            } else {
                declined += 1;
                assert!(
                    direct
                        .iter()
                        .all(|&sample| sample.to_u16() == sentinel.to_u16())
                );
            }
            for row in 0..size {
                let lanes = row * stride..row * stride + size;
                let expected = &expected[row * size..(row + 1) * size];
                for (results, check) in [(&output, true), (&direct, accepted)] {
                    let actual = results[lanes.clone()].iter().map(|sample| sample.to_u16());
                    assert!(
                        !check || actual.eq(expected.iter().copied()),
                        "{case} {params:?} {weight}"
                    );
                }
                assert!(
                    output[row * stride + size..(row + 1) * stride]
                        .iter()
                        .all(|&sample| sample.to_u16() == sentinel.to_u16())
                );
            }
        }
        assert!(
            fused > 500 && declined > 500,
            "fused {fused} declined {declined}"
        );
    }

    #[test]
    fn fused_four_by_four_cells_match_the_two_call_path() {
        check_cells::<u8>(BitDepth::Eight, 4);
        check_cells::<u16>(BitDepth::Ten, 4);
        check_cells::<u16>(BitDepth::Eight, 4);
    }
}
