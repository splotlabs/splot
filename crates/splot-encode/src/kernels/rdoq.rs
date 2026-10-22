// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Rate-distortion optimized quantization of one 2D-class transform block,
//! for the § 5.20.7.27 `coeffs()` syntax without TCQ or parity hiding.
//!
//! The round-to-nearest levels `L` are the candidates. In reverse scan, each
//! coefficient keeps the cheapest of `{L, L - 1, 0}` by
//! `J = D * 2^16 + lambda_q8 * R`: `D` is the squared error between the
//! coefficient and the exact § 7.14.4 reconstruction, in the scale of the
//! forward transform output, and `R` is in 1/256 bit. Contexts see the levels
//! already chosen at later scan positions, in the order the writer codes them.
//! A forward pass then picks the `eob` with the lowest cost, and zeroes the
//! levels after it. With `lambda_q8 == 0` the result is the round-to-nearest
//! quantization.
//!
//! Both quantization passes run on [`EncoderKernels::quantize`], so
//! [`super::SimdKernels`] vectorizes them; the reverse-scan decisions are
//! sequential.

use splot_core::coefficient::{COEFF_BASE_RANGE, LF_NUM_BASE_LEVELS, NUM_BASE_LEVELS};
use splot_decode::encoder_syntax::{
    CoeffLevels, MAX_GOLOMB_PREFIX_BITS, is_low_frequency, max_level,
};
use splot_recon::{BitDepth, TransformClass, coefficient_scan_slice};

use super::{EncoderKernels, KernelError, Quantizer};

/// One bit in the rate unit (1/256 bit).
pub(crate) const BIT: u32 = 256;
/// Base-level symbol costs per coefficient: the low-frequency `0..=LF_NUM_BASE_LEVELS + 1`.
pub(crate) const BASE_SYMBOLS: usize = LF_NUM_BASE_LEVELS as usize + 2;
/// `coeff_br` symbol costs per coefficient.
pub(crate) const BR_SYMBOLS: usize = COEFF_BASE_RANGE as usize + 1;

/// The quantizer and the adjusted shape of one transform block.
#[derive(Clone, Copy, Debug)]
pub(crate) struct RdoqBlock {
    /// § 7.14.2 DC quantizer step.
    pub(crate) dc_q2: u32,
    /// § 7.14.2 AC quantizer step.
    pub(crate) ac_q2: u32,
    /// § 7.14.4 `dqDenom`.
    pub(crate) dq_denom: u32,
    /// The active bit depth.
    pub(crate) bit_depth: BitDepth,
    /// `log2` of the adjusted width, `2..=5`.
    pub(crate) log2_width: u32,
    /// `log2` of the adjusted height, `2..=5`.
    pub(crate) log2_height: u32,
    /// Plane index (`0` luma).
    pub(crate) plane: usize,
}

/// The context of one coefficient level, as the writer derives it.
pub(crate) struct LevelContext<'a> {
    /// The levels of the later scan positions, as the writer's grid holds them.
    pub(crate) levels: &'a CoeffLevels,
    /// Scan index.
    pub(crate) c: usize,
    /// Adjusted row.
    pub(crate) row: usize,
    /// Adjusted column.
    pub(crate) col: usize,
    /// `get_lf_limits` puts the coefficient in the low-frequency region.
    pub(crate) is_lf: bool,
    /// The coefficient is the last one (`c == eob - 1`), coded by `coeff_base_eob`.
    pub(crate) is_last: bool,
    /// `(mag + 1) >> 1` capped at 4 over the base-level neighbour template.
    #[cfg(test)]
    pub(crate) base_class: usize,
    /// `(mag + 1) >> 1` capped at 6 over the `coeff_br` neighbour template.
    #[cfg(test)]
    pub(crate) br_class: usize,
}

/// Coefficient symbol costs in 1/256 bit.
///
/// Other signs are one bypass bit and the Golomb remainder is exact, so they
/// are not part of the model. A CDF-backed model holds the tile's
/// `IntraTileSyntax` and the `TransformBlock`, and reads the rows that the
/// writer will use: `txb_skip_cdf`, `eob_pt_cdf` and `eob_extra_cdf` (plus the
/// literal bits) for [`Self::eob`]; `coeff_base_eob_cdf` when `is_last`, else
/// `coeff_base_cdf(ctx.levels, ..)` for [`Self::base`];
/// `coeff_br_cdf(ctx.levels, ..)` for [`Self::br`]; `dc_sign_cdf` for
/// [`Self::dc_sign`]. The writer derives the same rows, so it reports a facade
/// error; the model can return a high cost for it.
pub(crate) trait CoeffRates {
    /// The cost of `all_zero` and, for `eob >= 1`, of the `eob` syntax elements.
    fn eob(&mut self, block: &RdoqBlock, eob: usize) -> u32;

    /// Index `b` is the cost of base level `b` (`coeff_base_eob` symbol `b - 1`
    /// when `is_last`; index 0 is then not used).
    fn base(&mut self, ctx: &LevelContext<'_>) -> [u32; BASE_SYMBOLS];

    /// Index `s` is the cost of `coeff_br` symbol `s`.
    fn br(&mut self, ctx: &LevelContext<'_>) -> [u32; BR_SYMBOLS];

    /// The cost of the luma `dc_sign` (either value).
    fn dc_sign(&mut self) -> u32;
}

/// A static rate model that needs no CDF state.
#[cfg(test)]
pub(crate) struct ApproxCoeffRates;

/// Cost of base level 0 by neighbour class: more neighbour energy makes zero rarer.
#[cfg(test)]
const ZERO_COST: [u32; 5] = [
    BIT / 5,
    BIT * 7 / 10,
    BIT * 6 / 5,
    BIT * 9 / 5,
    BIT * 12 / 5,
];
/// Cost of base level 1 by neighbour class.
#[cfg(test)]
const ONE_COST: [u32; 5] = [BIT * 3, BIT * 2, BIT * 3 / 2, BIT * 11 / 10, BIT * 4 / 5];
/// Cost of each base level above 1.
#[cfg(test)]
const LEVEL_STEP_COST: u32 = BIT * 6 / 5;

#[cfg(test)]
impl CoeffRates for ApproxCoeffRates {
    fn eob(&mut self, block: &RdoqBlock, eob: usize) -> u32 {
        if eob == 0 {
            return BIT;
        }
        let eob_pt = if eob <= 2 {
            eob as u32
        } else {
            (eob - 1).ilog2() + 2
        };
        let multisize = block.log2_width + block.log2_height - 4;
        let pt_extra = if eob_pt >= 8 {
            multisize.saturating_sub(3).min(2)
        } else {
            0
        };
        let offset_bits = eob_pt.saturating_sub(2);
        BIT + BIT * eob_pt / 2 + BIT * (pt_extra + offset_bits)
    }

    fn base(&mut self, ctx: &LevelContext<'_>) -> [u32; BASE_SYMBOLS] {
        let class = (ctx.base_class + usize::from(ctx.is_lf)).min(4);
        core::array::from_fn(|b| match (b, ctx.is_last) {
            (_, true) => BIT / 2 + LEVEL_STEP_COST * (b.max(1) as u32 - 1),
            (0, false) => ZERO_COST[class],
            (_, false) => ONE_COST[class] + LEVEL_STEP_COST * (b as u32 - 1),
        })
    }

    fn br(&mut self, ctx: &LevelContext<'_>) -> [u32; BR_SYMBOLS] {
        let tilt = BIT * ctx.br_class as u32 / 8;
        [
            BIT + tilt,
            2 * BIT - tilt / 2,
            3 * BIT - tilt / 2,
            3 * BIT - tilt / 2,
        ]
    }

    fn dc_sign(&mut self) -> u32 {
        BIT
    }
}

/// RD-optimizes the quantization of `coeffs` (adjusted, row-major) into
/// `levels` and their exact § 7.14.4 reconstruction `dequant`; returns `eob`.
///
/// # Errors
/// Returns [`KernelError::InvalidTransformShape`] for an adjusted side outside
/// `4..=32`, [`KernelError::BufferLength`] for a buffer that does not hold the
/// block, and [`KernelError::InvalidQuantizer`] for an invalid step.
pub(crate) fn rdoq<R: CoeffRates + ?Sized>(
    kernels: &dyn EncoderKernels,
    block: &RdoqBlock,
    coeffs: &[i32],
    rates: &mut R,
    lambda_q8: u64,
    levels: &mut [i32],
    dequant: &mut [i32],
) -> Result<usize, KernelError> {
    let mut walk = Walk::new(block)?;
    let n = walk.width * walk.height;
    if coeffs.len() != n {
        return Err(KernelError::BufferLength {
            buffer: "coeffs",
            expected: n,
            actual: coeffs.len(),
        });
    }
    let quantizer = |rounding_q8| {
        Quantizer::new(
            block.dc_q2,
            block.ac_q2,
            block.dq_denom,
            block.bit_depth,
            rounding_q8,
        )
    };
    kernels.quantize(&quantizer(128)?, coeffs, levels, dequant)?;
    let scan = walk.scan;
    let eob = scan
        .iter()
        .rposition(|&pos| levels[usize::from(pos)] != 0)
        .map_or(0, |last| last + 1);

    // A floor quantizer maps `recon(L) - 1` to `L - 1`: recon rises by at least one per level.
    let limit = (1u32 << (7 + u32::from(block.bit_depth.bits()))) - 1;
    let probe: Vec<i32> = levels
        .iter()
        .zip(&*dequant)
        .map(|(&l, &dq)| l.signum() * (dq.unsigned_abs().min(limit) as i32 - 1))
        .collect();
    let mut lower = vec![0; n];
    let mut lower_dq = vec![0; n];
    kernels.quantize(&quantizer(0)?, &probe, &mut lower, &mut lower_dq)?;

    let lambda = u128::from(lambda_q8);
    let cost = |d: u64, rate: u32| (u128::from(d) << 16) + lambda * u128::from(rate);
    let empty = walk.grid()?;
    let mut pick = vec![0u8; eob];
    let mut kept = vec![0u128; eob];
    let mut as_last = vec![None; eob];
    let mut zeroed = vec![0u128; eob];
    for c in (0..eob).rev() {
        let pos = usize::from(scan[c]);
        let is_last = c + 1 == eob;
        let candidates = [
            (levels[pos].unsigned_abs(), dequant[pos]),
            (lower[pos].unsigned_abs(), lower_dq[pos]),
            (0, 0),
        ];
        let distortion = |dq: i32| {
            (i64::from(coeffs[pos]) - i64::from(dq))
                .unsigned_abs()
                .pow(2)
        };
        zeroed[c] = cost(distortion(0), 0);
        let costs = walk.costs(rates, c, is_last, candidates[0].0, None);
        let mut best: Option<(u128, usize, u32)> = None;
        for (k, &(level, dq)) in candidates.iter().enumerate() {
            if (k > 0 && level == candidates[k - 1].0) || (is_last && level == 0) {
                continue;
            }
            let j = cost(distortion(dq), costs.rate(level, walk.hr_level_avg).0);
            if best.is_none_or(|(best_j, ..)| j < best_j) {
                best = Some((j, k, level));
            }
        }
        let Some((j, k, level)) = best else { continue };
        (pick[c], kept[c]) = (k as u8, j);
        if level != 0 {
            as_last[c] = Some(if is_last {
                j
            } else {
                let last = walk.costs(rates, c, true, level, Some(&empty));
                cost(distortion(candidates[k].1), last.rate(level, 0).0)
            });
        }
        walk.commit(c, level, &costs)?;
    }

    let mut suffix: u128 = zeroed.iter().sum();
    let mut best = (0, suffix + cost(0, rates.eob(block, 0)));
    let mut prefix = 0u128;
    for c in 0..eob {
        suffix -= zeroed[c];
        if let Some(last) = as_last[c] {
            let j = prefix + last + suffix + cost(0, rates.eob(block, c + 1));
            if j <= best.1 {
                best = (c + 1, j);
            }
        }
        prefix += kept[c];
    }
    for (c, &k) in pick.iter().enumerate() {
        let pos = usize::from(scan[c]);
        (levels[pos], dequant[pos]) = match k {
            _ if c >= best.0 => (0, 0),
            0 => continue,
            1 => (lower[pos], lower_dq[pos]),
            _ => (0, 0),
        };
    }
    Ok(best.0)
}

/// The writer's reverse-scan state: the level grids and `hrLevelAvg`.
struct Walk<'b> {
    block: &'b RdoqBlock,
    width: usize,
    height: usize,
    scan: &'static [u16],
    levels: CoeffLevels,
    /// The chosen levels by position, capped at `maxLevel`.
    chosen: Vec<u8>,
    hr_level_avg: u32,
}

/// One coefficient's symbol costs.
struct LevelCosts {
    is_lf: bool,
    plane: usize,
    base: [u32; BASE_SYMBOLS],
    br: [u32; BR_SYMBOLS],
    sign: u32,
}

impl<'b> Walk<'b> {
    fn new(block: &'b RdoqBlock) -> Result<Self, KernelError> {
        let invalid = || shape_error(block);
        if !(2..=5).contains(&block.log2_width) || !(2..=5).contains(&block.log2_height) {
            return Err(invalid());
        }
        let (width, height) = (1 << block.log2_width, 1 << block.log2_height);
        let scan =
            coefficient_scan_slice(width, height, TransformClass::TwoD).map_err(|_| invalid())?;
        let levels = CoeffLevels::new(width, height).map_err(|_| invalid())?;
        Ok(Self {
            block,
            width,
            height,
            scan,
            levels,
            chosen: vec![0; width * height],
            hr_level_avg: 0,
        })
    }

    fn grid(&self) -> Result<CoeffLevels, KernelError> {
        CoeffLevels::new(self.width, self.height).map_err(|_| shape_error(self.block))
    }

    /// The costs at scan index `c` for levels up to `max_candidate`; `empty`
    /// replaces the grid (no later levels) for a coefficient priced as the last.
    fn costs<R: CoeffRates + ?Sized>(
        &self,
        rates: &mut R,
        c: usize,
        is_last: bool,
        max_candidate: u32,
        empty: Option<&CoeffLevels>,
    ) -> LevelCosts {
        let pos = usize::from(self.scan[c]);
        let (row, col) = (pos >> self.block.log2_width, pos & (self.width - 1));
        let plane = self.block.plane;
        let is_lf = is_low_frequency(row, col, plane);
        #[cfg(test)]
        let (base_class, br_class) = if empty.is_some() {
            (0, 0)
        } else {
            self.classes(row, col)
        };
        let ctx = LevelContext {
            levels: empty.unwrap_or(&self.levels),
            c,
            row,
            col,
            is_lf,
            is_last,
            #[cfg(test)]
            base_class,
            #[cfg(test)]
            br_class,
        };
        let has_br = |level: u32| {
            level.min(max_level(is_lf, plane)) > base_levels(is_lf) && !(is_lf && plane > 0)
        };
        LevelCosts {
            is_lf,
            plane,
            base: rates.base(&ctx),
            br: if has_br(max_candidate) {
                rates.br(&ctx)
            } else {
                [0; BR_SYMBOLS]
            },
            sign: if pos == 0 && plane == 0 {
                rates.dc_sign()
            } else {
                BIT
            },
        }
    }

    /// The approximate neighbour classes of [`LevelContext`].
    #[cfg(test)]
    fn classes(&self, row: usize, col: usize) -> (usize, usize) {
        let at = |&[dr, dc]: &[i32; 2], cap: u8| {
            let (r, c) = (row + dr as usize, col + dc as usize);
            if r < self.height && c < self.width {
                u32::from(self.chosen[r * self.width + c].min(cap))
            } else {
                0
            }
        };
        let template = &splot_core::tables::conversion::SIG_REF_DIFF_OFFSET[0];
        let near = if self.block.plane == 0 { 5 } else { 3 };
        let base: u32 = template[..near].iter().map(|o| at(o, 3)).sum();
        let br: u32 = template[..3].iter().map(|o| at(o, 5)).sum();
        (
            ((base + 1) >> 1).min(4) as usize,
            ((br + 1) >> 1).min(6) as usize,
        )
    }

    /// Records `level` at scan index `c` as the writer's grid and remainder state do.
    fn commit(&mut self, c: usize, level: u32, costs: &LevelCosts) -> Result<(), KernelError> {
        let pos = usize::from(self.scan[c]);
        let capped = level.min(max_level(costs.is_lf, costs.plane));
        self.hr_level_avg = costs.rate(level, self.hr_level_avg).1;
        self.chosen[pos] = capped as u8;
        self.levels
            .set(pos >> self.block.log2_width, pos & (self.width - 1), capped)
            .map_err(|_| shape_error(self.block))
    }
}

fn shape_error(block: &RdoqBlock) -> KernelError {
    KernelError::InvalidTransformShape {
        log2_width: block.log2_width,
        log2_height: block.log2_height,
    }
}

impl LevelCosts {
    /// The rate of `level` and the next `hrLevelAvg`.
    fn rate(&self, level: u32, hr_level_avg: u32) -> (u32, u32) {
        if level == 0 {
            return (self.base[0], hr_level_avg);
        }
        let max = max_level(self.is_lf, self.plane);
        let capped = level.min(max);
        let base = capped.min(base_levels(self.is_lf) + 1);
        let mut rate = self.base[base as usize] + self.sign;
        if base > base_levels(self.is_lf) && !(self.is_lf && self.plane > 0) {
            rate += self.br[(capped - base) as usize];
        }
        if level < max {
            return (rate, hr_level_avg);
        }
        let (bits, next) = golomb_bits(level - max, hr_level_avg);
        (rate + bits, next)
    }
}

const fn base_levels(is_lf: bool) -> u32 {
    if is_lf {
        LF_NUM_BASE_LEVELS
    } else {
        NUM_BASE_LEVELS
    }
}

/// The exact bypass cost of the § 5.20.7.28 remainder `x` and the next `hrLevelAvg`.
fn golomb_bits(x: u32, hr_level_avg: u32) -> (u32, u32) {
    let m = hr_level_avg.checked_ilog2().unwrap_or(0).clamp(1, 6);
    let k = m + 1;
    let c_max = (m + 4).min(6);
    let bits = if x >> m < c_max {
        (x >> m) + 1 + m
    } else {
        let length = (x - (c_max << m) + (1 << k)).ilog2();
        c_max + (length - k + 1).min(MAX_GOLOMB_PREFIX_BITS) + length
    };
    (bits * BIT, (x + hr_level_avg) >> 1)
}

#[cfg(test)]
#[path = "rdoq_tests.rs"]
mod tests;
