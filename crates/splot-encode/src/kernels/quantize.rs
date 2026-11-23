// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Deadzone quantizer matched to the § 7.14.4 dequantizer.
//!
//! § 7.14.4
//! ([`07-decoding-process.md`](../../../../docs/spec/av2/1.0.0/07-decoding-process.md)
//! `#s-7-14-4`) reconstructs a level `l` as
//! `sign * Clip3(Round2(|l| * q2 & 0xFFFFFF, 3) / dqDenom)`. With
//! `q2 >= 8 * dqDenom` (no quantization matrix, TCQ off) the magnitude
//! `r(l) = ((l * q2 + 4) >> 3) >> log2(dqDenom)` rises by at least one per
//! level, so the quantizer finds `lo`, the largest level with `r(lo) <= |c|`,
//! and rounds up to `lo + 1` when
//! `(|c| - r(lo)) * 256 > (256 - rounding) * (r(lo + 1) - r(lo))`. Rounding 128
//! (one half) picks the nearest reconstruction level; smaller values widen the
//! deadzone toward zero.
//!
//! Levels stay inside the § 6.19.7.24 `Quant` range `|l| < 2^20`, inside
//! `|l| * q2 < 2^24` so the `0xFFFFFF` mask never wraps, and at or below the
//! first level whose reconstruction reaches the `Clip3` bound.
//!
//! The scalar test oracle searches `lo` from an exact integer division. The SIMD form
//! estimates `t = |c| * 8 * dqDenom / q2` in `f32` (error below 0.2 for every
//! `t` below `2^20`), and `r(l)` stays within `l * step - 1.5` and
//! `l * step + 0.5` with `step >= 1`, so `lo` lies in
//! `floor(t) - 1 ..= floor(t) + 2` (`floor(t) + 3` when the `Clip3` bound
//! saturates it); it tests the six levels above `estimate - 2`.

use fearless_simd::{
    Bytes as _, Level, Select as _, Simd, SimdBase as _, SimdCvtFloat as _, SimdCvtTruncate as _,
    SimdInt as _, SimdIntWiden as _, SimdMask as _, dispatch, f32x8, i32x8, u32x8, u64x4,
};

use splot_recon::{BitDepth, qm_weighted_quantizer};

use super::KernelError;

/// The largest coded `Quant` magnitude.
const MAX_LEVEL: u32 = splot_decode::encoder_syntax::MAX_QUANT_MAGNITUDE - 1;

/// The § 7.14.4 `dqHigh & 0xFFFFFF` mask width.
const PRODUCT_BITS: u32 = 24;

/// One quantizer step (DC or AC).
#[derive(Clone, Copy, Debug)]
struct Step {
    q2: u32,
    denom_shift: u32,
    max_level: u32,
    limit: u32,
    reciprocal: f32,
}

impl Step {
    fn new(q2: u32, dq_denom: u32, bound: u32) -> Result<Self, KernelError> {
        if !dq_denom.is_power_of_two()
            || dq_denom > 8
            || q2 < 8 * dq_denom
            || q2 >= 1 << PRODUCT_BITS
        {
            return Err(KernelError::InvalidQuantizer { q2, dq_denom });
        }
        let denom_shift = dq_denom.trailing_zeros();
        let max_level = MAX_LEVEL.min(((1 << PRODUCT_BITS) - 1) / q2);
        let mut step = Self {
            q2,
            denom_shift,
            max_level,
            limit: bound - 1,
            reciprocal: (8.0 * f64::from(dq_denom) / f64::from(q2)) as f32,
        };
        let mut saturated = (((bound - 1) << (3 + denom_shift)) / q2).min(max_level);
        while saturated > 0 && step.raw(saturated - 1) >= step.limit {
            saturated -= 1;
        }
        while saturated < max_level && step.raw(saturated) < step.limit {
            saturated += 1;
        }
        step.max_level = saturated;
        Ok(step)
    }

    /// `r(l)` without the `Clip3`, for `l <= max_level`.
    fn raw(self, level: u32) -> u32 {
        ((level * self.q2 + 4) >> 3) >> self.denom_shift
    }

    /// `r(l)` with the positive `Clip3` bound.
    fn recon(self, level: u32) -> u32 {
        self.raw(level).min(self.limit)
    }
}

/// A validated quantizer for one transform block.
#[derive(Clone, Debug)]
pub(crate) struct Quantizer {
    dc: Step,
    ac: Step,
    bound: u32,
    rounding_q8: u32,
    /// The quantization-matrix weighted step of each raster coefficient.
    weighted: Option<Box<[Step]>>,
}

impl Quantizer {
    /// A quantizer for the § 7.14.2 `dc_q2` / `ac_q2` steps (optionally
    /// quantization-matrix weighted), the § 7.14.4 `dq_denom`, the active
    /// `bit_depth`, and a deadzone `rounding_q8` in `0..=128` (`128` is
    /// round-to-nearest).
    ///
    /// # Errors
    /// Returns [`KernelError::InvalidQuantizer`] if a step is below
    /// `8 * dq_denom` or reaches `2^24`, or `dq_denom` is not a power of two in
    /// `1..=8`, and [`KernelError::InvalidRounding`] if `rounding_q8 > 128`.
    pub(crate) fn new(
        dc_q2: u32,
        ac_q2: u32,
        dq_denom: u32,
        bit_depth: BitDepth,
        rounding_q8: u32,
    ) -> Result<Self, KernelError> {
        if rounding_q8 > 128 {
            return Err(KernelError::InvalidRounding { rounding_q8 });
        }
        let bound = 1u32 << (7 + u32::from(bit_depth.bits()));
        Ok(Self {
            dc: Step::new(dc_q2, dq_denom, bound)?,
            ac: Step::new(ac_q2, dq_denom, bound)?,
            bound,
            rounding_q8,
            weighted: None,
        })
    }

    /// This quantizer with the § 7.14.4 matrix weight `m` of each raster
    /// coefficient in `weights`, whose step is `q2 = Round2(q * m, 5)`.
    ///
    /// # Errors
    /// Returns [`KernelError::InvalidQuantizer`] for a weighted step that
    /// [`Self::new`] rejects.
    pub(crate) fn with_weights(mut self, weights: Option<&[u8]>) -> Result<Self, KernelError> {
        if let Some(weights) = weights {
            let denom = 1 << self.ac.denom_shift;
            let steps = weights.iter().enumerate().map(|(i, &m)| {
                let q = if i == 0 { self.dc.q2 } else { self.ac.q2 };
                Step::new(qm_weighted_quantizer(q, i32::from(m)), denom, self.bound)
            });
            self.weighted = Some(steps.collect::<Result<_, _>>()?);
        }
        Ok(self)
    }

    /// The step of raster coefficient `i`.
    fn step(&self, i: usize) -> Step {
        match self.weighted.as_deref().and_then(|steps| steps.get(i)) {
            Some(&step) => step,
            None if i == 0 => self.dc,
            None => self.ac,
        }
    }

    /// The § 7.14.4 `dqDenom` with `allow_tcq` off for an original
    /// `2^log2_width x 2^log2_height` transform: `1 << ((pels > 256) + (pels > 1024))`.
    pub(crate) fn dq_denom_without_tcq(log2_width: u32, log2_height: u32) -> u32 {
        let pels = 1u32 << (log2_width + log2_height);
        1 << (u32::from(pels > 256) + u32::from(pels > 1024))
    }
}

fn check_lengths(coeffs: &[i32], levels: &[i32], dequant: &[i32]) -> Result<(), KernelError> {
    for (buffer, len) in [("levels", levels.len()), ("dequant", dequant.len())] {
        if len != coeffs.len() {
            return Err(KernelError::BufferLength {
                buffer,
                expected: coeffs.len(),
                actual: len,
            });
        }
    }
    Ok(())
}

#[cfg(test)]
pub(super) fn quantize_scalar(
    quantizer: &Quantizer,
    coeffs: &[i32],
    levels: &mut [i32],
    dequant: &mut [i32],
) -> Result<usize, KernelError> {
    check_lengths(coeffs, levels, dequant)?;
    let mut nonzero = 0;
    for (i, ((&c, level), dq)) in coeffs
        .iter()
        .zip(levels.iter_mut())
        .zip(dequant.iter_mut())
        .enumerate()
    {
        let step = quantizer.step(i);
        let a = c.unsigned_abs();
        let estimate = (u64::from(a) << (3 + step.denom_shift)) / u64::from(step.q2);
        let mut lo = estimate.min(u64::from(step.max_level)) as u32;
        while lo < step.max_level && step.recon(lo + 1) <= a {
            lo += 1;
        }
        while lo > 0 && step.recon(lo) > a {
            lo -= 1;
        }
        let r_lo = step.recon(lo);
        let up = lo < step.max_level
            && (a - r_lo) * 256 > (256 - quantizer.rounding_q8) * (step.recon(lo + 1) - r_lo);
        let magnitude = lo + u32::from(up);
        let r = step.raw(magnitude);
        (*level, *dq) = if c < 0 {
            (-(magnitude as i32), -(r.min(quantizer.bound) as i32))
        } else {
            (magnitude as i32, r.min(quantizer.bound - 1) as i32)
        };
        nonzero += usize::from(magnitude != 0);
    }
    Ok(nonzero)
}

/// [`quantize_scalar`] with `Σ (|c| - |dq|)²`, the squared error of the
/// reconstruction in the coefficient domain (wrapping in `u64`).
#[cfg(test)]
pub(super) fn quantize_error_scalar(
    quantizer: &Quantizer,
    coeffs: &[i32],
    levels: &mut [i32],
    dequant: &mut [i32],
) -> Result<(usize, u64), KernelError> {
    let nonzero = quantize_scalar(quantizer, coeffs, levels, dequant)?;
    let error = coeffs
        .iter()
        .zip(dequant.iter())
        .fold(0u64, |sum, (&c, &dq)| {
            let e = u64::from(c.unsigned_abs().abs_diff(dq.unsigned_abs()));
            sum.wrapping_add(e * e)
        });
    Ok((nonzero, error))
}

/// Per-lane [`Step`] fields.
struct StepLanes<S: Simd> {
    q2: u32x8<S>,
    denom_shift: u32x8<S>,
    max_level: u32x8<S>,
    limit: u32x8<S>,
    reciprocal: f32x8<S>,
    /// `ceil((256 - rounding) * r(1) / 256)`: a magnitude below it is below
    /// `r(1)` and does not round up, so it quantizes to level 0.
    zero_below: u32x8<S>,
}

const LANES: usize = 8;

impl<S: Simd> StepLanes<S> {
    /// Lane `i` takes `step(i)`.
    fn new(simd: S, step: impl Fn(usize) -> Step, rounding_q8: u32) -> Self {
        let pick = |f: fn(Step) -> u32| u32x8::from_fn(simd, |i| f(step(i)));
        Self {
            q2: pick(|s| s.q2),
            denom_shift: pick(|s| s.denom_shift),
            max_level: pick(|s| s.max_level),
            limit: pick(|s| s.limit),
            zero_below: u32x8::from_fn(simd, |i| {
                ((256 - rounding_q8) * step(i).recon(1)).div_ceil(256)
            }),
            reciprocal: f32x8::from_fn(simd, |i| step(i).reciprocal),
        }
    }

    fn raw(&self, level: u32x8<S>) -> u32x8<S> {
        ((level * self.q2 + 4) >> 3) >> self.denom_shift
    }

    fn recon(&self, level: u32x8<S>) -> u32x8<S> {
        self.raw(level).min(self.limit)
    }
}

pub(super) fn quantize_simd(
    quantizer: &Quantizer,
    coeffs: &[i32],
    levels: &mut [i32],
    dequant: &mut [i32],
) -> Result<usize, KernelError> {
    check_lengths(coeffs, levels, dequant)?;
    Ok(
        dispatch!(Level::baseline(), simd => quantize_kernel::<_, false>(simd, quantizer, coeffs, levels, dequant)).0,
    )
}

/// [`quantize_simd`] that also sums the squared coefficient-domain error in
/// the same pass ([`quantize_error_scalar`]).
pub(super) fn quantize_error_simd(
    quantizer: &Quantizer,
    coeffs: &[i32],
    levels: &mut [i32],
    dequant: &mut [i32],
) -> Result<(usize, u64), KernelError> {
    check_lengths(coeffs, levels, dequant)?;
    Ok(
        dispatch!(Level::baseline(), simd => quantize_kernel::<_, true>(simd, quantizer, coeffs, levels, dequant)),
    )
}

/// `(|c| - |dq|)²` of eight lanes, summed into `acc`.
#[inline]
fn add_error<S: Simd>(acc: &mut u64x4<S>, c: i32x8<S>, dq: i32x8<S>) {
    let (a, b) = (
        c.abs().bitcast::<u32x8<S>>(),
        dq.abs().bitcast::<u32x8<S>>(),
    );
    let e = a.max(b) - a.min(b);
    let (lo, hi) = e.widening_mul(e);
    *acc = *acc + lo + hi;
}

#[inline]
fn quantize_kernel<S: Simd, const ERROR: bool>(
    simd: S,
    quantizer: &Quantizer,
    coeffs: &[i32],
    levels: &mut [i32],
    dequant: &mut [i32],
) -> (usize, u64) {
    let (c_chunks, c_tail) = coeffs.as_chunks::<LANES>();
    let (l_chunks, l_tail) = levels.as_chunks_mut::<LANES>();
    let (d_chunks, d_tail) = dequant.as_chunks_mut::<LANES>();
    let rounding = quantizer.rounding_q8;
    let first = StepLanes::new(simd, |i| quantizer.step(i), rounding);
    let rest = StepLanes::new(simd, |_| quantizer.ac, rounding);
    let weighted =
        |chunk: usize| StepLanes::new(simd, |i| quantizer.step(LANES * chunk + i), rounding);
    let mut lanes;
    let mut count = 0;
    let zero = i32x8::splat(simd, 0);
    let mut error = u64x4::splat(simd, 0);
    for (i, ((c, l), d)) in c_chunks.iter().zip(l_chunks).zip(d_chunks).enumerate() {
        let steps = if quantizer.weighted.is_some() {
            lanes = weighted(i);
            &lanes
        } else if i == 0 {
            &first
        } else {
            &rest
        };
        let c = i32x8::load_array_ref(simd, c);
        if c.abs()
            .bitcast::<u32x8<S>>()
            .simd_lt(steps.zero_below)
            .all_true()
        {
            (*l, *d) = ([0; LANES], [0; LANES]);
            if ERROR {
                add_error(&mut error, c, zero);
            }
            continue;
        }
        let (level, dq) = quantize_lanes(quantizer, steps, c);
        if ERROR {
            add_error(&mut error, c, dq);
        }
        count += level.simd_ne(0).to_bitmask().count_ones() as usize;
        level.store_slice(l);
        dq.store_slice(d);
    }
    let n = c_tail.len();
    if n > 0 {
        let steps = if quantizer.weighted.is_some() {
            lanes = weighted(c_chunks.len());
            &lanes
        } else if c_chunks.is_empty() {
            &first
        } else {
            &rest
        };
        let mut c = [0; LANES];
        c[..n].copy_from_slice(c_tail);
        let c = i32x8::load_array_ref(simd, &c);
        let (level, dq) = quantize_lanes(quantizer, steps, c);
        if ERROR {
            add_error(&mut error, c, dq);
        }
        count += level.simd_ne(0).to_bitmask().count_ones() as usize;
        let (mut l, mut d) = ([0; LANES], [0; LANES]);
        level.store_slice(&mut l);
        dq.store_slice(&mut d);
        l_tail.copy_from_slice(&l[..n]);
        d_tail.copy_from_slice(&d[..n]);
    }
    (count, error.reduce_sum())
}

#[expect(
    clippy::inline_always,
    reason = "two callers; out of line it is a call per vector"
)]
#[inline(always)]
fn quantize_lanes<S: Simd>(
    quantizer: &Quantizer,
    steps: &StepLanes<S>,
    c: i32x8<S>,
) -> (i32x8<S>, i32x8<S>) {
    let a = c.abs().bitcast::<u32x8<S>>();
    let estimate =
        u32x8::truncate_from(f32x8::float_from(a) * steps.reciprocal).min(steps.max_level);
    let base = estimate.saturating_sub(2);
    let mut lo = base;
    for offset in 1..=6 {
        let candidate = (base + offset).min(steps.max_level);
        lo = steps
            .recon(candidate)
            .simd_le(a)
            .select(lo.max(candidate), lo);
    }
    let r_lo = steps.recon(lo);
    let below_max = lo.simd_lt(steps.max_level);
    let next = (lo + 1).min(steps.max_level);
    let gap = steps.recon(next) - r_lo;
    let distance = a.saturating_sub(r_lo).min(gap);
    let up = below_max & (distance * 256).simd_gt(gap * (256 - quantizer.rounding_q8));
    let magnitude = lo + (up.to_vector().bitcast::<u32x8<S>>() & 1);
    let r = steps.raw(magnitude);
    let negative = c.simd_lt(0);
    let magnitude = magnitude.bitcast::<i32x8<S>>();
    let level = negative.select(-magnitude, magnitude);
    let dq = negative.select(
        -r.min(quantizer.bound).bitcast::<i32x8<S>>(),
        r.min(quantizer.bound - 1).bitcast::<i32x8<S>>(),
    );
    (level, dq)
}

#[cfg(test)]
#[path = "quantize_tests.rs"]
mod tests;
