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
//! The scalar form searches `lo` from an exact integer division. The SIMD form
//! estimates `t = |c| * 8 * dqDenom / q2` in `f32` (error below 0.2 for every
//! `t` below `2^20`), and `r(l)` stays within `l * step - 1.5` and
//! `l * step + 0.5` with `step >= 1`, so `lo` lies in
//! `floor(t) - 1 ..= floor(t) + 2` (`floor(t) + 3` when the `Clip3` bound
//! saturates it); it tests the six levels above `estimate - 2`.

use std::simd::num::{SimdFloat as _, SimdInt as _, SimdUint as _};
use std::simd::{
    Mask, Select as _, Simd, cmp::SimdOrd as _, cmp::SimdPartialEq as _, cmp::SimdPartialOrd as _,
};

use splot_recon::BitDepth;

use super::KernelError;

/// § 6.19.7.24: `Quant` values are greater than `-(1 << 20)` and less than
/// `1 << 20`.
const MAX_LEVEL: u32 = (1 << 20) - 1;

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
#[derive(Clone, Copy, Debug)]
pub(crate) struct Quantizer {
    dc: Step,
    ac: Step,
    bound: u32,
    rounding_q8: u32,
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
        })
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
        let step = if i == 0 { quantizer.dc } else { quantizer.ac };
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

/// Per-lane [`Step`] fields.
struct StepLanes {
    q2: Simd<u32, LANES>,
    denom_shift: Simd<u32, LANES>,
    max_level: Simd<u32, LANES>,
    limit: Simd<u32, LANES>,
    reciprocal: Simd<f32, LANES>,
}

const LANES: usize = 8;

impl StepLanes {
    /// Lane 0 takes `first`, the other lanes `rest`.
    fn new(first: Step, rest: Step) -> Self {
        let pick = |f: fn(Step) -> u32| {
            Simd::from_array(core::array::from_fn(|i| {
                f(if i == 0 { first } else { rest })
            }))
        };
        Self {
            q2: pick(|s| s.q2),
            denom_shift: pick(|s| s.denom_shift),
            max_level: pick(|s| s.max_level),
            limit: pick(|s| s.limit),
            reciprocal: Simd::from_array(core::array::from_fn(|i| {
                if i == 0 {
                    first.reciprocal
                } else {
                    rest.reciprocal
                }
            })),
        }
    }

    fn raw(&self, level: Simd<u32, LANES>) -> Simd<u32, LANES> {
        ((level * self.q2 + Simd::splat(4)) >> Simd::splat(3)) >> self.denom_shift
    }

    fn recon(&self, level: Simd<u32, LANES>) -> Simd<u32, LANES> {
        self.raw(level).simd_min(self.limit)
    }
}

pub(super) fn quantize_simd(
    quantizer: &Quantizer,
    coeffs: &[i32],
    levels: &mut [i32],
    dequant: &mut [i32],
) -> Result<usize, KernelError> {
    check_lengths(coeffs, levels, dequant)?;
    let (c_chunks, c_tail) = coeffs.as_chunks::<LANES>();
    let (l_chunks, l_tail) = levels.as_chunks_mut::<LANES>();
    let (d_chunks, d_tail) = dequant.as_chunks_mut::<LANES>();
    let first = StepLanes::new(quantizer.dc, quantizer.ac);
    let rest = StepLanes::new(quantizer.ac, quantizer.ac);
    let mut nonzero = Simd::<u32, LANES>::splat(0);
    for (i, ((c, l), d)) in c_chunks.iter().zip(l_chunks).zip(d_chunks).enumerate() {
        let steps = if i == 0 { &first } else { &rest };
        let (level, dq) = quantize_lanes(quantizer, steps, Simd::from_array(*c));
        nonzero += level.simd_ne(Simd::splat(0)).to_simd().cast::<u32>() & Simd::splat(1);
        *l = level.to_array();
        *d = dq.to_array();
    }
    let tail_start = coeffs.len() - c_tail.len();
    let mut count = nonzero.reduce_sum() as usize;
    if tail_start == 0 {
        count += quantize_scalar(quantizer, c_tail, l_tail, d_tail)?;
    } else {
        let ac_only = Quantizer {
            dc: quantizer.ac,
            ..*quantizer
        };
        count += quantize_scalar(&ac_only, c_tail, l_tail, d_tail)?;
    }
    Ok(count)
}

fn quantize_lanes(
    quantizer: &Quantizer,
    steps: &StepLanes,
    c: Simd<i32, LANES>,
) -> (Simd<i32, LANES>, Simd<i32, LANES>) {
    let a = c.abs().cast::<u32>();
    let estimate = (a.cast::<f32>() * steps.reciprocal)
        .cast::<u32>()
        .simd_min(steps.max_level);
    let base = estimate.saturating_sub(Simd::splat(2));
    let mut lo = base;
    for offset in 1..=6 {
        let candidate = (base + Simd::splat(offset)).simd_min(steps.max_level);
        lo = steps
            .recon(candidate)
            .simd_le(a)
            .select(lo.simd_max(candidate), lo);
    }
    let r_lo = steps.recon(lo);
    let below_max = lo.simd_lt(steps.max_level);
    let next = (lo + Simd::splat(1)).simd_min(steps.max_level);
    let gap = steps.recon(next) - r_lo;
    let distance = a.saturating_sub(r_lo).simd_min(gap);
    let up = below_max
        & (distance * Simd::splat(256)).simd_gt(gap * Simd::splat(256 - quantizer.rounding_q8));
    let magnitude = lo + (up.to_simd().cast::<u32>() & Simd::splat(1));
    let r = steps.raw(magnitude);
    let negative: Mask<i32, LANES> = c.simd_lt(Simd::splat(0));
    let level = negative.select(-magnitude.cast::<i32>(), magnitude.cast::<i32>());
    let dq = negative.select(
        -r.simd_min(Simd::splat(quantizer.bound)).cast::<i32>(),
        r.simd_min(Simd::splat(quantizer.bound - 1)).cast::<i32>(),
    );
    (level, dq)
}

#[cfg(test)]
#[path = "quantize_tests.rs"]
mod tests;
