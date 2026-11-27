// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 § 7.21.7.3 grain templates.
//!
//! White noise takes sixteen § 7.21.7.2 register steps at a time. The
//! autoregressive filter splits each row in two. The taps on the rows above
//! are pair products over the whole row. The taps on the row itself read the
//! samples just filtered, so they run one sample a step, with two planes in
//! the lanes: Cb and Cr, or luma twice.

use fearless_simd::{Simd, i16x16, i32x4, i32x8, prelude::*, u32x8};
use splot_core::tables::conversion::GAUSSIAN_SEQUENCE;

use super::{ActiveFilmGrain, LUMA_GRAIN_BASE, LUMA_GRAIN_HEIGHT, LUMA_GRAIN_WIDTH};
use crate::{ReconError, Result};

/// `ar_coeff_lag` is `f(2)`.
const MAX_LAG: usize = 3;
/// Filtered columns of the widest template.
const MAX_COLUMNS: usize = LUMA_GRAIN_WIDTH - 2 * LUMA_GRAIN_BASE;
/// Samples of one vector step.
const STEP: usize = 16;

pub(super) struct GrainTemplates {
    pub(super) y: Vec<i16>,
    pub(super) cb: Vec<i16>,
    pub(super) cr: Vec<i16>,
    pub(super) chroma_width: usize,
}

/// The § 7.21.7.3 templates of `grain`. A plane without scaling points stays
/// zero, as its noise and its coefficients are absent.
pub(super) fn generate_grain<S: Simd>(
    simd: S,
    grain: &ActiveFilmGrain<'_>,
    sub_x: usize,
    sub_y: usize,
    bit_depth: u8,
    range: (i32, i32),
) -> Result<GrainTemplates> {
    let model = grain.model;
    let filter = ArFilter {
        lag: usize::from(model.ar_coeff_lag),
        shift: u32::from(model.ar_coeff_shift_minus_6) + 6,
        range,
    };
    let noise_shift = u32::from(12 - bit_depth + model.grain_scale_shift);

    let luma_seed = (model.num_y_points > 0).then_some(grain.grain_seed);
    let luma_len = LUMA_GRAIN_WIDTH * LUMA_GRAIN_HEIGHT;
    let mut y = white_noise(simd, luma_seed, luma_len, noise_shift);
    if luma_seed.is_some() {
        let plane = ArPlane::new(&mut y, &model.ar_coeffs_y, "ar_coeffs_y", &filter, false)?;
        filter.run(simd, &mut [plane], LUMA_GRAIN_WIDTH, None);
    }

    let chroma_width = if sub_x == 1 { 44 } else { 82 };
    let chroma_height = if sub_y == 1 { 38 } else { 73 };
    let chroma_seed = |points: u8, seed: u16| {
        (points > 0 || model.chroma_scaling_from_luma).then_some(grain.grain_seed ^ seed)
    };
    let cb_seed = chroma_seed(model.num_cb_points, 0xb524);
    let cr_seed = chroma_seed(model.num_cr_points, 0x49d8);
    let chroma_len = chroma_width * chroma_height;
    let mut cb = white_noise(simd, cb_seed, chroma_len, noise_shift);
    let mut cr = white_noise(simd, cr_seed, chroma_len, noise_shift);
    {
        let luma = luma_seed.map(|_| LumaTap {
            grain: &y,
            sub_x,
            sub_y,
        });
        let mut planes = [
            (cb_seed, &mut cb, &model.ar_coeffs_cb, "ar_coeffs_cb"),
            (cr_seed, &mut cr, &model.ar_coeffs_cr, "ar_coeffs_cr"),
        ]
        .into_iter()
        .filter(|plane| plane.0.is_some())
        .map(|(_, grain, coeffs, name)| ArPlane::new(grain, coeffs, name, &filter, luma.is_some()))
        .collect::<Result<Vec<_>>>()?;
        filter.run(simd, &mut planes, chroma_width, luma.as_ref());
    }

    Ok(GrainTemplates {
        y,
        cb,
        cr,
        chroma_width,
    })
}

/// § 7.21.7.3 white noise of `len` samples from `seed`, zero without one.
///
/// Bit `n` of `window` is bit 0 of the § 7.21.7.2 register `n` steps on: the
/// register's feedback yields the bits past the sixteen it holds, four at a
/// time, and the number of step `k` is bits `k + 5` to `k + 15`.
fn white_noise<S: Simd>(simd: S, seed: Option<u16>, len: usize, shift: u32) -> Vec<i16> {
    let mut noise = vec![0; len];
    let Some(seed) = seed else {
        return noise;
    };
    let mut window = u32::from(seed);
    let first_bits = u32x8::from_fn(simd, |lane| lane as u32 + 6);
    let rounding = (1 << shift) >> 1;
    let mut sixteen = || {
        for known in [16, 20, 24, 28] {
            let taps = window >> (known - 16);
            window |= ((taps ^ (taps >> 1) ^ (taps >> 3) ^ (taps >> 12)) & 0xf) << known;
        }
        let gaussian = |first_bits: u32x8<S>| {
            let numbers = (u32x8::splat(simd, window) >> first_bits).to_array();
            let lanes = numbers.map(|number| GAUSSIAN_SEQUENCE[number as usize & 0x7ff]);
            (i32x8::load_array(simd, lanes) + rounding) >> shift
        };
        let samples = simd.narrow_i32x8(gaussian(first_bits), gaussian(first_bits + 8));
        window >>= 16;
        samples
    };
    let (steps, tail) = noise.as_chunks_mut::<STEP>();
    for step in steps {
        sixteen().store_array(step);
    }
    tail.copy_from_slice(&sixteen().as_array()[..tail.len()]); // splot-copy-ok: the last step keeps only the samples the plane has left
    noise
}

/// The § 7.21.7.3 autoregressive filter of a frame.
struct ArFilter {
    lag: usize,
    shift: u32,
    range: (i32, i32),
}

/// A template plane under the filter, with its coefficients.
struct ArPlane<'a> {
    grain: &'a mut [i16],
    /// Per row above, oldest first, the coefficients from `deltaCol = -lag`
    /// on in pairs; each pair fills the lanes of one pair product.
    above: [[[i16; STEP]; MAX_LAG + 1]; MAX_LAG],
    /// Coefficients of the samples one, two and three columns to the left.
    left: [i32; MAX_LAG],
    /// Coefficient of the luma under the sample, zero without that tap.
    luma: i32,
}

/// The luma template as a tap of the chroma filter.
struct LumaTap<'a> {
    grain: &'a [i16],
    sub_x: usize,
    sub_y: usize,
}

const fn out_of_range(field: &'static str) -> ReconError {
    ReconError::FilmGrainModelOutOfRange { field }
}

fn load<S: Simd>(simd: S, grain: &[i16], at: usize) -> i16x16<S> {
    i16x16::from_slice(simd, &grain[at..at + STEP])
}

impl<'a> ArPlane<'a> {
    fn new(
        grain: &'a mut [i16],
        coeffs: &[i32],
        name: &'static str,
        filter: &ArFilter,
        luma_tap: bool,
    ) -> Result<Self> {
        let lag = filter.lag;
        if lag > MAX_LAG {
            return Err(out_of_range("ar_coeff_lag"));
        }
        let row_taps = 2 * lag + 1;
        let same_row = lag * row_taps;
        if coeffs.len() != same_row + lag + usize::from(luma_tap) {
            return Err(out_of_range(name));
        }
        let mut taps = [0_i16; MAX_LAG * (2 * MAX_LAG + 1) + MAX_LAG + 1];
        for (tap, &coeff) in taps.iter_mut().zip(coeffs) {
            *tap = i8::try_from(coeff).map_err(|_| out_of_range(name))?.into();
        }
        let mut above = [[[0; STEP]; MAX_LAG + 1]; MAX_LAG];
        for (row, taps) in above.iter_mut().zip(taps[..same_row].chunks(row_taps)) {
            for (lanes, pair) in row.iter_mut().zip(taps.chunks(2)) {
                for lanes in lanes.chunks_mut(2) {
                    for (lane, &tap) in lanes.iter_mut().zip(pair) {
                        *lane = tap;
                    }
                }
            }
        }
        let mut left = [0; MAX_LAG];
        for (left, &tap) in left.iter_mut().zip(taps[same_row..][..lag].iter().rev()) {
            *left = i32::from(tap);
        }
        Ok(Self {
            grain,
            above,
            left,
            luma: i32::from(taps[same_row + lag]),
        })
    }

    /// The filter sum of the `STEP` samples of `row` from `column`, short of
    /// the taps on the row itself: the noise at the filter's precision, the
    /// rounding offset, the taps on the rows above and the luma tap.
    ///
    /// A pair product weighs two neighbouring samples for every second
    /// sample, so the even and the odd samples sum apart.
    fn predict<S: Simd>(
        &self,
        simd: S,
        filter: &ArFilter,
        width: usize,
        (row, column): (usize, usize),
        luma: Option<(i32x8<S>, i32x8<S>)>,
    ) -> (i32x8<S>, i32x8<S>) {
        let lag = filter.lag;
        let mut even = i32x8::splat(simd, 0);
        let mut odd = even;
        for (above, pairs) in self.above[..lag].iter().enumerate() {
            let line = (row - lag + above) * width + column - lag;
            for (pair, weights) in pairs[..=lag].iter().enumerate() {
                let weights = i16x16::load_array_ref(simd, weights);
                let at = line + 2 * pair;
                even += simd.pairwise_dot_product_i16x16(load(simd, self.grain, at), weights);
                odd += simd.pairwise_dot_product_i16x16(load(simd, self.grain, at + 1), weights);
            }
        }
        let (low, high) = simd.widen_i16x16(load(simd, self.grain, row * width + column));
        let rounding = 1 << (filter.shift - 1);
        let mut low = (low << filter.shift) + simd.zip_low_i32x8(even, odd) + rounding;
        let mut high = (high << filter.shift) + simd.zip_high_i32x8(even, odd) + rounding;
        if let Some(luma) = luma {
            low += luma.0 * self.luma;
            high += luma.1 * self.luma;
        }
        (low, high)
    }
}

impl LumaTap<'_> {
    /// `Round2` of the luma under each of the `STEP` chroma samples of `row`
    /// from `column`.
    fn average<S: Simd>(&self, simd: S, row: usize, column: usize) -> (i32x8<S>, i32x8<S>) {
        let luma_row = ((row - LUMA_GRAIN_BASE) << self.sub_y) + LUMA_GRAIN_BASE;
        let luma_column = ((column - LUMA_GRAIN_BASE) << self.sub_x) + LUMA_GRAIN_BASE;
        let rows = |column: usize| {
            let at = luma_row * LUMA_GRAIN_WIDTH + luma_column + column;
            let top = load(simd, self.grain, at);
            if self.sub_y == 1 {
                top + load(simd, self.grain, at + LUMA_GRAIN_WIDTH)
            } else {
                top
            }
        };
        let (low, high) = if self.sub_x == 1 {
            (
                simd.pairwise_widening_add_i16x16(rows(0)),
                simd.pairwise_widening_add_i16x16(rows(STEP)),
            )
        } else {
            simd.widen_i16x16(rows(0))
        };
        let shift = (self.sub_x + self.sub_y) as u32;
        let rounding = (1 << shift) >> 1;
        ((low + rounding) >> shift, (high + rounding) >> shift)
    }
}

impl ArFilter {
    /// Filters up to two planes together, one in each lane of a sample step;
    /// a plane on its own takes both lanes. The last vector step of a row
    /// ends on its last column and overlaps the step before.
    fn run<S: Simd>(
        &self,
        simd: S,
        planes: &mut [ArPlane<'_>],
        width: usize,
        luma: Option<&LumaTap<'_>>,
    ) {
        let Some((first, rest)) = planes.split_first_mut() else {
            return;
        };
        let mut second = rest.first_mut();
        let columns = width - 2 * LUMA_GRAIN_BASE;
        let steps = (0..columns - STEP).step_by(STEP).chain([columns - STEP]);
        let lanes = |pair: [i32; 2]| i32x4::load_array(simd, [pair[0], pair[1], 0, 0]);
        let weights = {
            let other = second.as_deref().unwrap_or(first);
            [0, 1, 2].map(|tap| lanes([first.left[tap], other.left[tap]]))
        };
        let (min, max) = self.range;
        let mut predicted = [[0; 2]; MAX_COLUMNS];
        let mut filtered = [[0; 2]; MAX_COLUMNS];
        for row in LUMA_GRAIN_BASE..first.grain.len() / width {
            let line = row * width;
            let other = second.as_deref().unwrap_or(first);
            for step in steps.clone() {
                let at = (row, LUMA_GRAIN_BASE + step);
                let luma = luma.map(|tap| tap.average(simd, at.0, at.1));
                let a = first.predict(simd, self, width, at, luma);
                let b = second
                    .as_ref()
                    .map_or(a, |second| second.predict(simd, self, width, at, luma));
                let (slots, _) =
                    predicted.as_flattened_mut()[2 * step..][..2 * STEP].as_chunks_mut::<8>();
                simd.zip_low_i32x8(a.0, b.0).store_array(&mut slots[0]);
                simd.zip_high_i32x8(a.0, b.0).store_array(&mut slots[1]);
                simd.zip_low_i32x8(a.1, b.1).store_array(&mut slots[2]);
                simd.zip_high_i32x8(a.1, b.1).store_array(&mut slots[3]);
            }

            let mut left = [2, 1, 0].map(|column| {
                let at = line + column;
                lanes([first.grain[at].into(), other.grain[at].into()])
            });
            for (predicted, filtered) in predicted[..columns].iter().zip(&mut filtered) {
                let sum = lanes(*predicted)
                    + left[2] * weights[2]
                    + left[1] * weights[1]
                    + left[0] * weights[0];
                let sample = (sum >> self.shift).max(min).min(max);
                let pair = sample.as_array();
                *filtered = [pair[0], pair[1]];
                left = [sample, left[0], left[1]];
            }

            for step in steps.clone() {
                let (lanes, _) = filtered.as_flattened()[2 * step..][..2 * STEP].as_chunks::<8>();
                let pairs = |half: usize| {
                    simd.narrow_i32x8(
                        i32x8::load_array_ref(simd, &lanes[half]),
                        i32x8::load_array_ref(simd, &lanes[half + 1]),
                    )
                };
                let (low, high) = (pairs(0), pairs(2));
                let at = line + LUMA_GRAIN_BASE + step;
                simd.unzip_low_i16x16(low, high)
                    .store_slice(&mut first.grain[at..at + STEP]);
                if let Some(second) = &mut second {
                    simd.unzip_high_i16x16(low, high)
                        .store_slice(&mut second.grain[at..at + STEP]);
                }
            }
        }
    }
}

#[cfg(test)]
pub(super) mod tests;
