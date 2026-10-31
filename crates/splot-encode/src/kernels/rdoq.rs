// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Rate-distortion optimized quantization of one transform block of any
//! class, for the § 5.20.7.27 `coeffs()` syntax without TCQ, and the parity
//! fix-up of a block that hides a level parity.
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
//!
//! [`hide_parity`] runs after any quantizer. When the block hides the parity
//! of its scan index 0 level and that level has the other parity, it moves the
//! one level by one whose move has the lowest `J` delta: the exact distortion
//! delta plus the rate delta of the level's own symbols in the writer's
//! contexts, among the moves after which the facade rule holds.

use splot_core::coefficient::{COEFF_BASE_RANGE, LF_NUM_BASE_LEVELS, NUM_BASE_LEVELS};
use splot_decode::encoder_syntax::{
    CoeffLevels, MAX_GOLOMB_PREFIX_BITS, MAX_QUANT_MAGNITUDE, TxType, type_scan,
};
use splot_recon::{BitDepth, dequant_coefficient};

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
    /// The coded type: its class picks the scan and the contexts.
    pub(crate) tx_type: TxType,
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
/// The Golomb remainder is exact, so it is not part of the model. A
/// CDF-backed model holds the tile's `IntraTileSyntax` and the
/// `TransformBlock`, and reads the rows that the
/// writer will use: `txb_skip_cdf`, `eob_pt_cdf` and `eob_extra_cdf` (plus the
/// literal bits) for [`Self::eob`]; `coeff_base_eob_cdf` when `is_last`, else
/// `coeff_base_cdf(ctx.levels, ..)` for [`Self::base`];
/// `coeff_br_cdf(ctx.levels, ..)` for [`Self::br`]; `sign_cdf(ctx.levels, ..)`
/// for [`Self::sign`]. The writer derives the same rows, so it reports a facade
/// error; the model can return a high cost for it.
pub(crate) trait CoeffRates {
    /// The cost of `all_zero` and, for `eob >= 1`, of the `eob` syntax elements.
    fn eob(&mut self, block: &RdoqBlock, eob: usize) -> u32;

    /// Index `b` is the cost of base level `b` (`coeff_base_eob` symbol `b - 1`
    /// when `is_last`; index 0 is then not used).
    fn base(&mut self, ctx: &LevelContext<'_>) -> [u32; BASE_SYMBOLS];

    /// Index `s` is the cost of `coeff_br` symbol `s`.
    fn br(&mut self, ctx: &LevelContext<'_>) -> [u32; BR_SYMBOLS];

    /// The cost of the sign of a nonzero level (either value).
    fn sign(&mut self, ctx: &LevelContext<'_>) -> u32;
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

    fn sign(&mut self, _: &LevelContext<'_>) -> u32 {
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
    let mut scratch = RDOQ_SCRATCH.take();
    let eob = rdoq_in(
        &mut scratch,
        kernels,
        block,
        coeffs,
        (rates, lambda_q8),
        levels,
        dequant,
    );
    RDOQ_SCRATCH.set(scratch);
    eob
}

/// The per-block buffers of [`rdoq`].
#[derive(Default)]
struct RdoqScratch {
    probe: Vec<i32>,
    lower: Vec<i32>,
    lower_dq: Vec<i32>,
    pick: Vec<u8>,
    kept: Vec<u128>,
    as_last: Vec<Option<u128>>,
    zeroed: Vec<u128>,
}

std::thread_local! {
    /// [`RdoqScratch`] kept between blocks; a nested call finds it empty.
    static RDOQ_SCRATCH: core::cell::Cell<RdoqScratch> = core::cell::Cell::default();
}

/// `buffer` as `len` copies of `value`, reusing its allocation.
fn refill<T: Clone>(buffer: &mut Vec<T>, len: usize, value: T) -> &mut Vec<T> {
    buffer.clear();
    buffer.resize(len, value);
    buffer
}

/// [`rdoq`] with its buffers in `scratch`.
fn rdoq_in<R: CoeffRates + ?Sized>(
    scratch: &mut RdoqScratch,
    kernels: &dyn EncoderKernels,
    block: &RdoqBlock,
    coeffs: &[i32],
    (rates, lambda_q8): (&mut R, u64),
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
    let probe = &mut scratch.probe;
    probe.clear();
    probe.extend(
        levels
            .iter()
            .zip(&*dequant)
            .map(|(&l, &dq)| l.signum() * (dq.unsigned_abs().min(limit) as i32 - 1)),
    );
    let lower = refill(&mut scratch.lower, n, 0);
    let lower_dq = refill(&mut scratch.lower_dq, n, 0);
    kernels.quantize(&quantizer(0)?, probe, lower, lower_dq)?;

    let lambda = u128::from(lambda_q8);
    let cost = |d: u64, rate: u32| (u128::from(d) << 16) + lambda * u128::from(rate);
    let empty = walk.grid()?;
    let pick = refill(&mut scratch.pick, eob, 0);
    let kept = refill(&mut scratch.kept, eob, 0);
    let as_last = refill(&mut scratch.as_last, eob, None);
    let zeroed = refill(&mut scratch.zeroed, eob, 0);
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

/// Moving one level of a block by one: the `J` delta, the row-major
/// position, the new level and its reconstruction.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) struct ParityMove {
    delta: i128,
    pos: usize,
    level: i32,
    dequant: i32,
}

/// The [`ParityMove`]s of `levels` (adjusted, row-major) below their `eob`,
/// priced in the writer's contexts by `rates` with the scan index 0 level
/// coded as hidden. Without `rates` only the scan index 0 level moves, ranked
/// by distortion.
///
/// # Errors
/// Returns [`KernelError::InvalidTransformShape`] for an invalid block.
pub(crate) fn parity_moves<R: CoeffRates + ?Sized>(
    block: &RdoqBlock,
    coeffs: &[i32],
    mut rates: Option<&mut R>,
    lambda_q8: u64,
    levels: &[i32],
) -> Result<Vec<ParityMove>, KernelError> {
    let shape = || shape_error(block);
    let scan = type_scan(1 << block.log2_width, 1 << block.log2_height, block.tx_type)
        .map_err(|_| shape())?;
    if coeffs.len() != scan.len() || levels.len() != scan.len() {
        return Err(shape());
    }
    let mut walk = rates.is_some().then(|| Walk::new(block)).transpose()?;
    if let Some(walk) = &mut walk {
        walk.levels.set_hidden(true);
    }
    let eob = scan
        .iter()
        .rposition(|&pos| levels[usize::from(pos)] != 0)
        .map_or(0, |last| last + 1);
    let lambda = i128::from(lambda_q8);
    let movable = if rates.is_some() { eob } else { eob.min(1) };
    let mut moves = Vec::with_capacity(2 * movable);
    for c in (0..movable).rev() {
        let pos = usize::from(scan[c]);
        let level = levels[pos].unsigned_abs();
        let costs = rates
            .as_deref_mut()
            .zip(walk.as_ref())
            .map(|(rates, walk)| {
                (
                    walk.costs(rates, c, c + 1 == eob, level + 1, None),
                    walk.hr_level_avg,
                )
            });
        let q2 = if pos == 0 { block.dc_q2 } else { block.ac_q2 };
        let sign = if levels[pos] < 0 || (levels[pos] == 0 && coeffs[pos] < 0) {
            -1
        } else {
            1
        };
        let price = |m: u32| {
            let dq = dequant_coefficient(sign * m as i32, q2, block.dq_denom, block.bit_depth);
            let d = (i64::from(coeffs[pos]) - i64::from(dq))
                .unsigned_abs()
                .pow(2);
            let rate = costs.as_ref().map_or(0, |(costs, hr)| costs.rate(m, *hr).0);
            ((i128::from(d) << 16) + lambda * i128::from(rate), dq)
        };
        let (kept, _) = price(level);
        for m in [level + 1, level.wrapping_sub(1)] {
            if m < MAX_QUANT_MAGNITUDE && (m > 0 || c + 1 < eob) {
                let (j, dequant) = price(m);
                let level = sign * m as i32;
                moves.push(ParityMove {
                    delta: j - kept,
                    pos,
                    level,
                    dequant,
                });
            }
        }
        if let (Some(walk), Some((costs, _))) = (&mut walk, &costs) {
            walk.commit(c, level, costs)?;
        }
    }
    Ok(moves)
}

/// Makes `levels` and their reconstruction `dequant` keep the rule `holds`
/// (the facade's `ParityHiding::holds`): when they hide a parity the scan
/// index 0 level does not have, applies the first move of `moves` in `J`
/// order after which the rule holds.
///
/// # Errors
/// Returns the error of `holds`, and [`KernelError::HiddenParity`] when no
/// move keeps the rule.
pub(crate) fn hide_parity<E: From<KernelError>>(
    holds: impl Fn(&[i32]) -> Result<bool, E>,
    mut moves: Vec<ParityMove>,
    (levels, dequant): (&mut [i32], &mut [i32]),
) -> Result<(), E> {
    if holds(levels)? {
        return Ok(());
    }
    while let Some(best) = (0..moves.len()).min_by_key(|&i| moves[i]) {
        let ParityMove {
            pos,
            level,
            dequant: dq,
            ..
        } = moves.swap_remove(best);
        if pos >= levels.len().min(dequant.len()) {
            continue;
        }
        let kept = core::mem::replace(&mut levels[pos], level);
        if holds(levels)? {
            dequant[pos] = dq;
            return Ok(());
        }
        levels[pos] = kept;
    }
    Err(KernelError::HiddenParity.into())
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
    /// `maxLevel`.
    max: u32,
    /// `1` when the level codes half its magnitude (hidden parity).
    shift: u32,
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
        let levels = CoeffLevels::new(width, height, block.tx_type).map_err(|_| invalid())?;
        Ok(Self {
            block,
            width,
            height,
            scan: levels.scan(),
            levels,
            chosen: vec![0; width * height],
            hr_level_avg: 0,
        })
    }

    fn grid(&self) -> Result<CoeffLevels, KernelError> {
        CoeffLevels::new(self.width, self.height, self.block.tx_type)
            .map_err(|_| shape_error(self.block))
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
        let levels = empty.unwrap_or(&self.levels);
        let is_lf = levels.is_low_frequency(row, col, plane);
        let max = levels.max_level(c, is_lf, plane);
        let shift = u32::from(levels.hides(c));
        #[cfg(test)]
        let (base_class, br_class) = if empty.is_some() {
            (0, 0)
        } else {
            self.classes(row, col)
        };
        let ctx = LevelContext {
            levels,
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
        let has_br =
            |level: u32| (level >> shift).min(max) > base_levels(is_lf) && !(is_lf && plane > 0);
        LevelCosts {
            is_lf,
            plane,
            max,
            shift,
            base: rates.base(&ctx),
            br: if has_br(max_candidate) {
                rates.br(&ctx)
            } else {
                [0; BR_SYMBOLS]
            },
            sign: if levels.sign_has_cdf(row, col, plane) {
                rates.sign(&ctx)
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
    #[inline]
    fn commit(&mut self, c: usize, level: u32, costs: &LevelCosts) -> Result<(), KernelError> {
        let pos = usize::from(self.scan[c]);
        let capped = (level >> costs.shift).min(costs.max);
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
        let (coded, max) = (level >> self.shift, self.max);
        let capped = coded.min(max);
        let base = capped.min(base_levels(self.is_lf) + 1);
        let mut rate = self.base[base as usize] + self.sign;
        if base > base_levels(self.is_lf) && !(self.is_lf && self.plane > 0) {
            rate += self.br[(capped - base) as usize];
        }
        if coded < max {
            return (rate, hr_level_avg);
        }
        let x = coded - max;
        let bits = golomb_bits(x, hr_level_avg >> self.shift);
        (rate + bits, ((x << self.shift) + hr_level_avg) >> 1)
    }
}

pub(crate) const fn base_levels(is_lf: bool) -> u32 {
    if is_lf {
        LF_NUM_BASE_LEVELS
    } else {
        NUM_BASE_LEVELS
    }
}

/// The exact bypass cost of the § 5.20.7.28 remainder `x` under `predLevel` `pred_level`.
pub(crate) fn golomb_bits(x: u32, pred_level: u32) -> u32 {
    let m = pred_level.checked_ilog2().unwrap_or(0).clamp(1, 6);
    let k = m + 1;
    let c_max = (m + 4).min(6);
    let bits = if x >> m < c_max {
        (x >> m) + 1 + m
    } else {
        let length = (x - (c_max << m) + (1 << k)).ilog2();
        c_max + (length - k + 1).min(MAX_GOLOMB_PREFIX_BITS) + length
    };
    bits * BIT
}

#[cfg(test)]
#[path = "rdoq_tests.rs"]
mod tests;
