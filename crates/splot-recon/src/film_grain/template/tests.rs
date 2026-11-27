// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use fearless_simd::{Level, Simd};
use splot_core::headers::film_grain::FilmGrainModel;
use splot_core::tables::conversion::GAUSSIAN_SEQUENCE;

use super::{GrainTemplates, generate_grain};
use crate::ReconError;
use crate::film_grain::tests::{TestRng, film_grain_model};
use crate::film_grain::{
    ActiveFilmGrain, GrainRng, LUMA_GRAIN_BASE, LUMA_GRAIN_HEIGHT, LUMA_GRAIN_WIDTH, round2,
};

const MAX_AR_COEFFICIENTS: usize = 24;

/// The § 7.21.7.3 process as written, one sample at a time.
pub(in crate::film_grain) fn scalar_templates(
    grain: &ActiveFilmGrain<'_>,
    sub_x: usize,
    sub_y: usize,
    bit_depth: u8,
    (grain_min, grain_max): (i32, i32),
) -> GrainTemplates {
    let model = grain.model;
    let mut rng = GrainRng::new(grain.grain_seed);
    let shift = 12 - bit_depth + model.grain_scale_shift;
    let mut luma = vec![0; LUMA_GRAIN_WIDTH * LUMA_GRAIN_HEIGHT];
    for row in 0..LUMA_GRAIN_HEIGHT {
        for col in 0..LUMA_GRAIN_WIDTH {
            let g = if model.num_y_points > 0 {
                GAUSSIAN_SEQUENCE[rng.get(11)]
            } else {
                0
            };
            luma[row * LUMA_GRAIN_WIDTH + col] = round2(g, shift);
        }
    }
    apply_luma_ar(model, &mut luma, grain_min, grain_max);

    let chroma_width = if sub_x == 1 { 44 } else { 82 };
    let chroma_height = if sub_y == 1 { 38 } else { 73 };
    let mut cb = vec![0; chroma_width * chroma_height];
    let mut cr = vec![0; chroma_width * chroma_height];
    fill_chroma_white_noise(
        &mut cb,
        chroma_width,
        chroma_height,
        grain.grain_seed ^ 0xb524,
        model.num_cb_points > 0 || model.chroma_scaling_from_luma,
        shift,
    );
    fill_chroma_white_noise(
        &mut cr,
        chroma_width,
        chroma_height,
        grain.grain_seed ^ 0x49d8,
        model.num_cr_points > 0 || model.chroma_scaling_from_luma,
        shift,
    );
    apply_chroma_ar(
        model,
        &luma,
        &mut cb,
        &mut cr,
        chroma_width,
        chroma_height,
        sub_x,
        sub_y,
        grain_min,
        grain_max,
    );

    let narrow = |planes: Vec<i32>| narrow_grain(planes, grain_min, grain_max);
    GrainTemplates {
        y: narrow(luma),
        cb: narrow(cb),
        cr: narrow(cr),
        chroma_width,
    }
}

/// Packs grain into `i16` lanes clamped to the grain range. Every sample a
/// noise stripe reads lies in the autoregressive region, which § 7.21.7.3
/// already clamps, so only the never-read border changes.
fn narrow_grain(planes: Vec<i32>, grain_min: i32, grain_max: i32) -> Vec<i16> {
    planes
        .into_iter()
        .map(|sample| sample.clamp(grain_min, grain_max) as i16)
        .collect()
}

fn apply_luma_ar(model: &FilmGrainModel, luma: &mut [i32], grain_min: i32, grain_max: i32) {
    let (offsets, offset_count) = ar_neighbor_offsets(model.ar_coeff_lag, LUMA_GRAIN_WIDTH);
    let shift = model.ar_coeff_shift_minus_6 + 6;
    for row in LUMA_GRAIN_BASE..LUMA_GRAIN_HEIGHT {
        for col in LUMA_GRAIN_BASE..(LUMA_GRAIN_WIDTH - LUMA_GRAIN_BASE) {
            let index = row * LUMA_GRAIN_WIDTH + col;
            let mut sum = 0;
            for (&offset, &coeff) in offsets[..offset_count].iter().zip(&model.ar_coeffs_y) {
                let source = (index as isize + offset) as usize;
                sum += luma[source] * coeff;
            }
            let sample = luma[index] + round2(sum, shift);
            luma[index] = sample.clamp(grain_min, grain_max);
        }
    }
}

fn fill_chroma_white_noise(
    out: &mut [i32],
    width: usize,
    height: usize,
    seed: u16,
    active: bool,
    shift: u8,
) {
    let mut rng = GrainRng::new(seed);
    for row in 0..height {
        for col in 0..width {
            let g = if active {
                GAUSSIAN_SEQUENCE[rng.get(11)]
            } else {
                0
            };
            out[row * width + col] = round2(g, shift);
        }
    }
}

#[expect(clippy::too_many_arguments)]
fn apply_chroma_ar(
    model: &FilmGrainModel,
    luma: &[i32],
    cb: &mut [i32],
    cr: &mut [i32],
    width: usize,
    height: usize,
    sub_x: usize,
    sub_y: usize,
    grain_min: i32,
    grain_max: i32,
) {
    let (offsets, offset_count) = ar_neighbor_offsets(model.ar_coeff_lag, width);
    let luma_coeff_cb = model.ar_coeffs_cb.get(offset_count).copied().unwrap_or(0);
    let luma_coeff_cr = model.ar_coeffs_cr.get(offset_count).copied().unwrap_or(0);
    let shift = model.ar_coeff_shift_minus_6 + 6;
    for row in LUMA_GRAIN_BASE..height {
        for col in LUMA_GRAIN_BASE..(width - LUMA_GRAIN_BASE) {
            let index = row * width + col;
            let mut sum_cb = 0;
            let mut sum_cr = 0;
            for (pos, &offset) in offsets[..offset_count].iter().enumerate() {
                let source = (index as isize + offset) as usize;
                sum_cb += cb[source] * model.ar_coeffs_cb.get(pos).copied().unwrap_or(0);
                sum_cr += cr[source] * model.ar_coeffs_cr.get(pos).copied().unwrap_or(0);
            }
            if model.num_y_points > 0 {
                let luma_x = ((col - LUMA_GRAIN_BASE) << sub_x) + LUMA_GRAIN_BASE;
                let luma_y = ((row - LUMA_GRAIN_BASE) << sub_y) + LUMA_GRAIN_BASE;
                let mut average = 0;
                for y_offset in 0..=sub_y {
                    for x_offset in 0..=sub_x {
                        average += luma[(luma_y + y_offset) * LUMA_GRAIN_WIDTH + luma_x + x_offset];
                    }
                }
                let average = round2(average, (sub_x + sub_y) as u8);
                sum_cb += average * luma_coeff_cb;
                sum_cr += average * luma_coeff_cr;
            }
            cb[index] = (cb[index] + round2(sum_cb, shift)).clamp(grain_min, grain_max);
            cr[index] = (cr[index] + round2(sum_cr, shift)).clamp(grain_min, grain_max);
        }
    }
}

fn ar_neighbor_offsets(lag: u8, stride: usize) -> ([isize; MAX_AR_COEFFICIENTS], usize) {
    let lag = i32::from(lag);
    let mut offsets = [0; MAX_AR_COEFFICIENTS];
    let mut count = 0;
    'outer: for delta_row in -lag..=0 {
        for delta_col in -lag..=lag {
            if delta_row == 0 && delta_col == 0 {
                break 'outer;
            }
            offsets[count] = delta_row as isize * stride as isize + delta_col as isize;
            count += 1;
        }
    }
    (offsets, count)
}

/// How a test fills the autoregressive coefficients.
#[derive(Clone, Copy)]
enum Coefficients {
    Random,
    Alternating,
    All(i32),
}

/// The fixture's model with the generation syntax a parser would yield for
/// `lag` and the planes that carry scaling points.
fn model(
    rng: &mut TestRng,
    lag: u8,
    points: [bool; 3],
    from_luma: bool,
    fill: Coefficients,
) -> FilmGrainModel {
    let mut model = film_grain_model();
    model.ar_coeff_lag = lag;
    model.chroma_scaling_from_luma = from_luma;
    model.num_y_points = points[0].into();
    model.num_cb_points = points[1].into();
    model.num_cr_points = points[2].into();
    model.grain_scale_shift = (rng.next() % 4) as u8;
    let luma_taps = 2 * usize::from(lag) * (usize::from(lag) + 1);
    let mut coeffs = |active: bool, count: usize| -> Vec<i32> {
        (0..if active { count } else { 0 })
            .map(|index| match fill {
                Coefficients::Random => (rng.next() % 256) as i32 - 128,
                Coefficients::Alternating if index & 1 == 0 => -128,
                Coefficients::Alternating => 127,
                Coefficients::All(value) => value,
            })
            .collect()
    };
    let chroma_taps = luma_taps + usize::from(points[0]);
    model.ar_coeffs_y = coeffs(points[0], luma_taps);
    model.ar_coeffs_cb = coeffs(points[1] || from_luma, chroma_taps);
    model.ar_coeffs_cr = coeffs(points[2] || from_luma, chroma_taps);
    model
}

fn assert_matches_scalar<S: Simd>(simd: S, model: &FilmGrainModel, seed: u16, context: &str) {
    let grain = ActiveFilmGrain {
        model,
        grain_seed: seed,
    };
    for (sub_x, sub_y) in [(1, 1), (1, 0), (0, 0)] {
        for bit_depth in [8, 10] {
            let range = (-(1 << (bit_depth - 1)), (1 << (bit_depth - 1)) - 1);
            let want = scalar_templates(&grain, sub_x, sub_y, bit_depth, range);
            let got = generate_grain(simd, &grain, sub_x, sub_y, bit_depth, range).unwrap();
            let context = format!("{context}, subsampling {sub_x}{sub_y}, {bit_depth} bits");
            assert_eq!(got.chroma_width, want.chroma_width, "{context}");
            assert_eq!(got.y, want.y, "luma, {context}");
            assert_eq!(got.cb, want.cb, "Cb, {context}");
            assert_eq!(got.cr, want.cr, "Cr, {context}");
        }
    }
}

fn templates_match_scalar<S: Simd>(simd: S) {
    let mut rng = TestRng(0x7a31_c0de_91f4_2b6d);
    let fills = [
        Coefficients::Random,
        Coefficients::Alternating,
        Coefficients::All(127),
        Coefficients::All(-128),
    ];
    let planes = [
        ([true, true, true], false),
        ([true, false, false], true),
        ([false, true, true], false),
        ([true, true, false], false),
        ([true, false, true], false),
        ([false, false, false], true),
    ];
    for lag in 0..=3 {
        for ar_shift in [0, 3] {
            for fill in fills {
                for (points, from_luma) in planes {
                    let mut model = model(&mut rng, lag, points, from_luma, fill);
                    model.ar_coeff_shift_minus_6 = ar_shift;
                    let seed = match rng.next() % 4 {
                        0 => 0,
                        1 => u16::MAX,
                        _ => rng.next() as u16,
                    };
                    let context = format!(
                        "lag {lag}, shift {ar_shift}, points {points:?}, from luma {from_luma}, \
                         seed {seed:#06x}, coefficients {:?}",
                        model.ar_coeffs_cb
                    );
                    assert_matches_scalar(simd, &model, seed, &context);
                }
            }
        }
    }
}

#[test]
fn templates_match_the_scalar_process() {
    fearless_simd::dispatch!(Level::new(), simd => templates_match_scalar(simd));
}

/// The 128-bit level builds every 256-bit operation from two halves.
#[cfg(target_arch = "x86_64")]
#[test]
fn templates_match_the_scalar_process_on_128_bit_vectors() {
    if let Some(simd) = Level::new().as_sse4_2() {
        templates_match_scalar(simd);
    }
}

#[test]
fn syntax_the_model_cannot_code_is_an_error() {
    let valid = || model(&mut TestRng(2), 2, [true; 3], false, Coefficients::Random);
    let reject = |model: FilmGrainModel, field: &'static str| {
        let grain = ActiveFilmGrain {
            model: &model,
            grain_seed: 0,
        };
        let result = baseline_simd!(simd => generate_grain(simd, &grain, 1, 1, 8, (-128, 127)));
        assert_eq!(
            result.err(),
            Some(ReconError::FilmGrainModelOutOfRange { field })
        );
    };

    let mut lag = valid();
    lag.ar_coeff_lag = 4;
    reject(lag, "ar_coeff_lag");

    let mut count = valid();
    count.ar_coeffs_cb.pop();
    reject(count, "ar_coeffs_cb");

    let mut value = valid();
    value.ar_coeffs_cr[3] = 128;
    reject(value, "ar_coeffs_cr");
}
