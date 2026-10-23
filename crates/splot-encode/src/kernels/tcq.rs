// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Trellis-coded quantization of one block whose levels are TCQ levels
//! (§ 5.20.7.27, § 7.14.4).
//!
//! A Viterbi search runs over the reverse scan, the order the writer codes the
//! levels in, and keeps the cheapest path into each of the eight TCQ states by
//! `J = D * 2^16 + lambda_q8 * R`. In each state a coefficient offers zero and
//! the two levels of the state's quantizer whose reconstructions bracket it;
//! the even and the odd one lead to different next states. `D` is the squared
//! error of the exact § 7.14.4 reconstruction, and `R` comes from the CDF
//! rows of the state's own path: each state keeps its own level grid, and a
//! path that takes over another's grid copies only the levels later contexts
//! can still read ([`CoeffLevels::context_reach`]). A path may also start at a
//! coefficient whose nonzero level beats zero, as the last one, with the zeroed
//! later coefficients and the `eob` syntax in its cost.
//! [`EncoderKernels::tcq_step`] adds the branch metrics of all states at once
//! and keeps the survivors.

use std::simd::cmp::SimdPartialOrd as _;
use std::simd::num::SimdUint as _;
use std::simd::{Select as _, Simd, simd_swizzle};

use splot_decode::encoder_syntax::{CoeffLevels, TCQ_STATES, tcq_next_state, tcq_quantizer};
use splot_decode::encoder_syntax::MAX_QUANT_MAGNITUDE;
use splot_recon::dequant_coefficient;

use super::rdoq::{
    BASE_SYMBOLS, BIT, BR_SYMBOLS, CoeffRates, LevelContext, RdoqBlock, base_levels, golomb_bits,
};
use super::{EncoderKernels, KernelError};

/// The candidate slots of a coefficient in one state: zero, the even nonzero
/// level, and the odd level.
pub(crate) const ZERO: usize = 0;
/// See [`ZERO`].
pub(crate) const EVEN: usize = 1;
/// See [`ZERO`].
pub(crate) const ODD: usize = 2;
const SLOTS: usize = 3;
/// The number of TCQ states.
pub(crate) const STATES: usize = TCQ_STATES;
/// The predecessor of a path that starts at the coefficient, in a pick.
const START: u8 = TCQ_STATES as u8;

/// The two branches into each next state `t`, as lanes `parity * 8 + state`
/// of the predecessor: `Tcq_Next_State[state][parity] == t`. The SIMD
/// survivor selection needs them as constants; a test checks them against
/// [`tcq_next_state`].
pub(crate) const FIRST: [usize; TCQ_STATES] = [0, 2, 12, 14, 8, 10, 4, 6];
/// See [`FIRST`].
pub(crate) const SECOND: [usize; TCQ_STATES] = [9, 11, 5, 7, 1, 3, 13, 15];

/// The largest distortion and `lambda_q8` the branch kernel takes, so no
/// path sum overflows `u64`; a distortion of `MAX_DIST` marks a missing slot.
pub(crate) const MAX_DIST: u64 = 1 << 40;
const MAX_LAMBDA: u64 = 1 << 30;

/// The distortion and rate of each slot in each state at one coefficient.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Branches {
    /// At most [`MAX_DIST`].
    pub(crate) dist: [[u64; TCQ_STATES]; SLOTS],
    /// In 1/256 bit; `lambda * rate` stays below `2^62`.
    pub(crate) rate: [[u32; TCQ_STATES]; SLOTS],
}

/// The path cost into each next state, and its pick: `slot | predecessor << 2`.
pub(crate) type Survivors = ([u64; TCQ_STATES], [u8; TCQ_STATES]);

/// The scalar twin of [`step_simd`].
#[cfg(test)]
pub(crate) fn step_scalar(prev: &[u64; TCQ_STATES], b: &Branches, lambda: u64) -> Survivors {
    let j = |k: usize, s: usize| (b.dist[k][s] << 16).saturating_add(lambda * u64::from(b.rate[k][s]));
    let mut lanes = [(0u64, 0u8); 2 * TCQ_STATES];
    for s in 0..TCQ_STATES {
        let (zero, even) = (j(ZERO, s), j(EVEN, s));
        let (even, slot) = if zero <= even { (zero, ZERO) } else { (even, EVEN) };
        lanes[s] = (prev[s].saturating_add(even), slot as u8 | (s as u8) << 2);
        lanes[TCQ_STATES + s] = (prev[s].saturating_add(j(ODD, s)), ODD as u8 | (s as u8) << 2);
    }
    let mut out = ([0; TCQ_STATES], [0; TCQ_STATES]);
    for t in 0..TCQ_STATES {
        let (a, b) = (lanes[FIRST[t]], lanes[SECOND[t]]);
        (out.0[t], out.1[t]) = if b.0 < a.0 { b } else { a };
    }
    out
}

/// Adds the branch metrics of `b` to the path costs `prev` of every state and
/// keeps the cheaper branch into each next state (the first on a tie).
pub(crate) fn step_simd(prev: &[u64; TCQ_STATES], b: &Branches, lambda: u64) -> Survivors {
    type V = Simd<u64, TCQ_STATES>;
    let lambda = V::splat(lambda);
    let j = |k: usize| {
        (V::from_array(b.dist[k]) << 16)
            .saturating_add(lambda * Simd::from_array(b.rate[k]).cast::<u64>())
    };
    let (zero, even) = (j(ZERO), j(EVEN));
    let take_zero = zero.simd_le(even);
    let prev = V::from_array(*prev);
    let even = prev.saturating_add(take_zero.select(zero, even));
    let odd = prev.saturating_add(j(ODD));
    let lane = V::from_array([0, 1, 2, 3, 4, 5, 6, 7]) << 2;
    let even_pick = lane | take_zero.select(V::splat(ZERO as u64), V::splat(EVEN as u64));
    let odd_pick = lane | V::splat(ODD as u64);
    let first = simd_swizzle!(even, odd, FIRST);
    let second = simd_swizzle!(even, odd, SECOND);
    let take_second = second.simd_lt(first);
    let pick = take_second.select(
        simd_swizzle!(even_pick, odd_pick, SECOND),
        simd_swizzle!(even_pick, odd_pick, FIRST),
    );
    (
        take_second.select(second, first).to_array(),
        pick.cast::<u8>().to_array(),
    )
}

/// One coefficient's candidates per quantizer and slot: the level magnitude
/// and its distortion.
pub(crate) type Candidates = [[(u32, u64); SLOTS]; 2];

/// The sign a level of `coeff` takes: a zero coefficient takes `+`, so every
/// candidate level keeps its parity.
const fn sign(coeff: i32) -> i32 {
    if coeff < 0 { -1 } else { 1 }
}

/// Whether `coeff` at `pos` reaches the first nonzero reconstruction, half a step.
fn reaches_half_step(block: &RdoqBlock, pos: usize, coeff: i32) -> bool {
    let q2 = if pos == 0 { block.dc_q2 } else { block.ac_q2 };
    u64::from(coeff.unsigned_abs()) * 8 * u64::from(block.dq_denom) >= u64::from(q2)
}

/// Zero and the two levels around `coeff` at `pos` of each quantizer, with
/// their distortions; [`MAX_DIST`] marks a missing even level. A level `q` of
/// quantizer `z` reconstructs reconstruction index `2q - z` (§ 5.20.7.27).
pub(crate) fn candidates(block: &RdoqBlock, pos: usize, coeff: i32) -> Candidates {
    let q2 = if pos == 0 { block.dc_q2 } else { block.ac_q2 };
    let below = u64::from(coeff.unsigned_abs()) * 8 * u64::from(block.dq_denom) / u64::from(q2.max(1));
    let dist = |index: u32| {
        let dq = dequant_coefficient(sign(coeff) * index as i32, q2, block.dq_denom, block.bit_depth);
        (i64::from(coeff) - i64::from(dq))
            .unsigned_abs()
            .pow(2)
            .min(MAX_DIST - 1)
    };
    let zero = dist(0);
    core::array::from_fn(|z| {
        let low = ((below + z as u64) / 2).min(u64::from(MAX_QUANT_MAGNITUDE)) as u32;
        let mut out = [(0, zero), (0, MAX_DIST), (0, MAX_DIST)];
        for quant in [low, low + 1].into_iter().filter(|&q| q > 0) {
            out[if quant & 1 == 0 { EVEN } else { ODD }] = (quant, dist(2 * quant - z as u32));
        }
        out
    })
}

/// TCQ-quantizes `coeffs` (adjusted, row-major) into the coded `levels` of a
/// block of type `block.tx_type`, whose `dq_denom` is the TCQ `dqDenom`;
/// returns `eob`.
///
/// # Errors
/// Returns [`KernelError::InvalidTransformShape`] for an adjusted side outside
/// `4..=32`, and [`KernelError::BufferLength`] for a buffer that does not hold
/// the block.
pub(crate) fn tcq<R: CoeffRates + ?Sized>(
    kernels: &dyn EncoderKernels,
    block: &RdoqBlock,
    coeffs: &[i32],
    rates: &mut R,
    lambda_q8: u64,
    levels: &mut [i32],
) -> Result<usize, KernelError> {
    let shape = || KernelError::InvalidTransformShape {
        log2_width: block.log2_width,
        log2_height: block.log2_height,
    };
    if !(2..=5).contains(&block.log2_width) || !(2..=5).contains(&block.log2_height) {
        return Err(shape());
    }
    let (width, height) = (1usize << block.log2_width, 1usize << block.log2_height);
    let new_grid = || {
        let mut grid = CoeffLevels::new(width, height, block.tx_type).map_err(|_| shape())?;
        grid.set_tcq_state(0);
        Ok(grid)
    };
    let empty = new_grid()?;
    let scan = empty.scan();
    for (buffer, len) in [("coeffs", coeffs.len()), ("levels", levels.len())] {
        if len != scan.len() {
            return Err(KernelError::BufferLength {
                buffer,
                expected: scan.len(),
                actual: len,
            });
        }
    }
    levels.fill(0);
    let lambda = lambda_q8.min(MAX_LAMBDA);
    let end = scan
        .iter()
        .rposition(|&pos| reaches_half_step(block, usize::from(pos), coeffs[usize::from(pos)]))
        .map_or(0, |last| last + 1);
    let mut zero_suffix = vec![0u64; end + 1];
    for c in (0..end).rev() {
        let d = u64::from(coeffs[usize::from(scan[c])].unsigned_abs()).pow(2);
        zero_suffix[c] = zero_suffix[c + 1].saturating_add(d.min(MAX_DIST - 1) << 16);
    }
    let all_zero = zero_suffix[0].saturating_add(lambda * u64::from(rates.eob(block, 0)));
    if end == 0 {
        return Ok(0);
    }

    let reach = empty.context_reach();
    let mut grids = (0..TCQ_STATES).map(|_| new_grid()).collect::<Result<Vec<_>, _>>()?;
    let mut grid_of: [usize; TCQ_STATES] = core::array::from_fn(|s| s);
    let mut cost = [u64::MAX; TCQ_STATES];
    let mut hr = [0u32; TCQ_STATES];
    let mut picks = vec![[0u8; TCQ_STATES]; end];
    let mut kept = vec![[[0u32; SLOTS]; 2]; end];
    let plane = block.plane;
    for c in (0..end).rev() {
        let pos = usize::from(scan[c]);
        let (row, col) = (pos >> block.log2_width, pos & (width - 1));
        let at = At {
            c,
            row,
            col,
            is_lf: empty.is_low_frequency(row, col, plane),
        };
        let cand = candidates(block, pos, coeffs[pos]);
        let mut coded = [[Coded::default(); SLOTS]; 2];
        for (coded, cand) in coded.iter_mut().zip(&cand) {
            for (coded, &(quant, _)) in coded.iter_mut().zip(cand) {
                *coded = Coded::new(&empty, &at, plane, quant);
            }
        }
        let has_br = coded.iter().flatten().any(|coded| coded.br != u8::MAX);
        let sign = if empty.sign_has_cdf(row, col, plane) {
            rates.sign(&at.context(&empty, false))
        } else {
            BIT
        };

        let mut branches = Branches::default();
        let mut next_hr = [hr; SLOTS];
        for s in (0..TCQ_STATES).filter(|&s| cost[s] != u64::MAX) {
            let z = tcq_quantizer(s);
            let grid = &mut grids[grid_of[s]];
            grid.set_tcq_state(s);
            let rows = LevelRows::new(rates, &at.context(grid, false), has_br);
            for k in 0..SLOTS {
                let (golomb, hr_after) = coded[z][k].remainder(hr[s]);
                branches.dist[k][s] = cand[z][k].1;
                branches.rate[k][s] = rows.rate(&coded[z][k], sign).saturating_add(golomb);
                next_hr[k][s] = hr_after;
            }
        }
        let (mut next, mut pick) = kernels.tcq_step(&cost, &branches, lambda);

        let z = tcq_quantizer(0);
        let mut start_hr = [0u32; SLOTS];
        let starts = [EVEN, ODD].map(|k| (k, cand[z][k].1 < cand[z][ZERO].1));
        if starts.iter().any(|&(_, start)| start) {
            let eob = rates.eob(block, c + 1);
            let start = zero_suffix[c + 1].saturating_add(lambda * u64::from(eob));
            let rows = LevelRows::new(rates, &at.context(&empty, true), has_br);
            for (k, _) in starts.into_iter().filter(|&(_, start)| start) {
                let (golomb, hr_after) = coded[z][k].remainder(0);
                let rate = rows.rate(&coded[z][k], sign).saturating_add(golomb);
                start_hr[k] = hr_after;
                let j = start
                    .saturating_add(cand[z][k].1 << 16)
                    .saturating_add(lambda * u64::from(rate));
                let t = tcq_next_state(0, cand[z][k].0);
                if j < next[t] {
                    (next[t], pick[t]) = (j, k as u8 | START << 2);
                }
            }
        }

        let mut claimed = [false; TCQ_STATES];
        let mut taken = [None; TCQ_STATES];
        for t in (0..TCQ_STATES).filter(|&t| next[t] != u64::MAX) {
            let (from, k) = (usize::from(pick[t] >> 2), usize::from(pick[t] & 3));
            hr[t] = if from < TCQ_STATES { next_hr[k][from] } else { start_hr[k] };
            if from < TCQ_STATES && !claimed[grid_of[from]] {
                claimed[grid_of[from]] = true;
                taken[t] = Some(grid_of[from]);
            }
        }
        let mut free = (0..TCQ_STATES).filter(|&g| !claimed[g]);
        let window = c + 1..(c + reach).min(end);
        let mut next_grid = [0; TCQ_STATES];
        for t in (0..TCQ_STATES).filter(|&t| next[t] != u64::MAX) {
            let (from, k) = (usize::from(pick[t] >> 2), usize::from(pick[t] & 3));
            let g = if let Some(g) = taken[t] {
                g
            } else {
                let g = free.next().ok_or_else(shape)?;
                if from < TCQ_STATES {
                    let [dst, src] = grids
                        .get_disjoint_mut([g, grid_of[from]])
                        .map_err(|_| shape())?;
                    dst.copy_cells(Some(src), window.clone());
                } else {
                    grids[g].copy_cells(None, window.clone());
                }
                g
            };
            next_grid[t] = g;
            let z = if from < TCQ_STATES { tcq_quantizer(from) } else { z };
            grids[g]
                .set(row, col, coded[z][k].level)
                .map_err(|_| shape())?;
        }
        grid_of = next_grid;
        picks[c] = pick;
        kept[c] = cand.map(|slots| slots.map(|(quant, _)| quant));
        cost = next;
    }

    let (mut state, best) = (0..TCQ_STATES)
        .map(|t| (t, cost[t]))
        .min_by_key(|&(_, j)| j)
        .unwrap_or((0, u64::MAX));
    if all_zero <= best {
        return Ok(0);
    }
    for c in 0..end {
        let (from, k) = (usize::from(picks[c][state] >> 2), usize::from(picks[c][state] & 3));
        let z = tcq_quantizer(if from < TCQ_STATES { from } else { 0 });
        let pos = usize::from(scan[c]);
        levels[pos] = sign(coeffs[pos]) * kept[c][z][k] as i32;
        if from >= TCQ_STATES {
            return Ok(c + 1);
        }
        state = from;
    }
    Ok(end)
}

/// The scan index and adjusted position of one coefficient.
struct At {
    c: usize,
    row: usize,
    col: usize,
    is_lf: bool,
}

impl At {
    const fn context<'a>(&self, levels: &'a CoeffLevels, is_last: bool) -> LevelContext<'a> {
        LevelContext {
            levels,
            c: self.c,
            row: self.row,
            col: self.col,
            is_lf: self.is_lf,
            is_last,
            #[cfg(test)]
            base_class: 0,
            #[cfg(test)]
            br_class: 0,
        }
    }
}

/// The base and `coeff_br` symbol costs of one coefficient in one context.
struct LevelRows {
    base: [u32; BASE_SYMBOLS],
    br: [u32; BR_SYMBOLS],
}

impl LevelRows {
    /// The rows of `ctx`, with `coeff_br` when `has_br`.
    fn new<R: CoeffRates + ?Sized>(rates: &mut R, ctx: &LevelContext<'_>, has_br: bool) -> Self {
        Self {
            base: rates.base(ctx),
            br: if has_br { rates.br(ctx) } else { [0; BR_SYMBOLS] },
        }
    }

    /// The rate of `coded` without its remainder.
    fn rate(&self, coded: &Coded, sign: u32) -> u32 {
        if coded.level == 0 {
            return self.base[0];
        }
        let rate = self.base[usize::from(coded.base)].saturating_add(sign);
        match self.br.get(usize::from(coded.br)) {
            Some(&br) => rate.saturating_add(br),
            None => rate,
        }
    }
}

/// A `Coded::remainder` of a level without a remainder.
const NO_REMAINDER: u32 = u32::MAX;

/// How one level magnitude codes: its `Level`, base symbol, `coeff_br`
/// symbol (`u8::MAX` without one), and § 5.20.7.28 remainder.
#[derive(Clone, Copy)]
struct Coded {
    level: u32,
    base: u8,
    br: u8,
    remainder: u32,
}

impl Default for Coded {
    fn default() -> Self {
        Self {
            level: 0,
            base: 0,
            br: u8::MAX,
            remainder: NO_REMAINDER,
        }
    }
}

impl Coded {
    #[inline]
    fn new(grid: &CoeffLevels, at: &At, plane: usize, quant: u32) -> Self {
        let (level, remainder) = grid.split(at.c, quant, at.is_lf, plane);
        let base = level.min(base_levels(at.is_lf) + 1);
        let has_br = base > base_levels(at.is_lf) && !(at.is_lf && plane > 0);
        Self {
            level,
            base: base as u8,
            br: if has_br { (level - base) as u8 } else { u8::MAX },
            remainder: remainder.unwrap_or(NO_REMAINDER),
        }
    }

    /// The rate of the remainder under `hrLevelAvg` `hr_level_avg`, and the next `hrLevelAvg`.
    fn remainder(&self, hr_level_avg: u32) -> (u32, u32) {
        match self.remainder {
            NO_REMAINDER => (0, hr_level_avg),
            x => (golomb_bits(x, hr_level_avg), (x + hr_level_avg) >> 1),
        }
    }
}

#[cfg(test)]
#[path = "tcq_tests.rs"]
mod tests;
