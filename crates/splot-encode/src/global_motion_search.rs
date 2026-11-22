// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The `global_motion` 2 estimator (§ 5.18.9.1, § 7.13.3.19 block warp).
//!
//! Each textured lookahead block refines its vector on the full-resolution
//! source luma to a quarter sample with the decoder's 8-tap filter
//! (§ 7.13.3.18), which gives one point pair. RANSAC over the pairs keeps the
//! `ROTZOOM` or `AFFINE` model (the two non-identity `GmType` values AV2
//! codes) with the most pairs within [`INLIER_SAMPLES`], and least squares
//! refits both types to its inliers. Each fit then descends on the coded
//! `gm_params` lattice: one term at a time moves by a shrinking number of
//! steps while the luma SAD of the decoder's block warp (§ 7.13.3.19) over a
//! grid of 8x8 blocks plus lambda times the exact parameter bits drops. A
//! model is kept only where it beats both the zero model and the best
//! translation, which blocks code as vectors without parameter bits.

#![allow(
    clippy::many_single_char_names,
    reason = "model terms and point coordinates keep their geometric names"
)]

use splot_core::write::BitWriter;
use splot_core::write::global_motion::write_global_param;
use splot_recon::{
    BitDepth, InterpolationFilter, ReferencePlaneView, SubpelPredictParams, WarpPredictBlockParams,
    subpel_predict_block_into, warp_predict_block_into, warp_shear_is_valid,
};

use crate::global_motion::{
    ALPHA_MAX, ALPHA_STEP, MIN_PAIRS, MIN_TEXTURE, PREC, Pair, TRANS_MAX, TRANS_STEP, sampler,
};
use crate::lookahead::BlockMotion;

/// RANSAC hypotheses per pair of frames.
const ITERATIONS: usize = 128;
/// The distance, in full-resolution luma samples, within which a pair fits.
const INLIER_SAMPLES: f64 = 1.0;
/// `Default_Warp_Params`.
const IDENTITY: [i32; 6] = [0, 0, 1 << 16, 0, 0, 1 << 16];
/// The largest step multiple of the lattice descent.
const DESCENT_START: i32 = 8;
/// The grid of the pixel cost: one 8x8 block per this many luma samples.
const COST_GRID: usize = 16;
/// `REF_SCALE_SHIFT` (§ 3): an unscaled reference.
const UNSCALED: i32 = 1 << 14;

/// The luma of the current and the reference source frames.
pub(crate) struct LumaPair<'a> {
    pub(crate) current: &'a [u16],
    pub(crate) reference: &'a [u16],
    pub(crate) width: usize,
    pub(crate) height: usize,
    pub(crate) bit_depth: BitDepth,
}

/// An affine `[m2, m3, m4, m5, tx, ty]`: `u = m2 x + m3 y + tx`,
/// `v = m4 x + m5 y + ty`.
type Model = [f64; 6];

/// The kept `gm_params` of `frames`, from the lookahead vectors `motion` of
/// the textured blocks of side `block` (`(cols, rows)` of them, `intra` their
/// intra SATDs); `sad_per_bit` (Q8) weighs the parameter bits; `seed` makes
/// the sampling deterministic.
pub(crate) fn estimate(
    frames: &LumaPair<'_>,
    (motion, intra): (&[BlockMotion], &[u32]),
    ((cols, rows), block): ((usize, usize), usize),
    sad_per_bit: u64,
    seed: u64,
) -> Option<[i32; 6]> {
    let scale = (block / crate::lookahead::BLOCK) as i32;
    let mut pairs = Vec::new();
    for row in 0..rows {
        for col in 0..cols {
            let index = row * cols + col;
            let (Some(found), Some(&cost)) = (motion.get(index), intra.get(index)) else {
                continue;
            };
            if cost < MIN_TEXTURE || found.cost >= cost {
                continue;
            }
            let (x, y) = (col * block, row * block);
            let start = (found.mv.0 * scale * 8, found.mv.1 * scale * 8);
            if let Some((dx, dy)) = refine_vector(frames, (x, y, block), start, scale * 8) {
                let (cx, cy) = (x as f64 + block as f64 / 2.0, y as f64 + block as f64 / 2.0);
                pairs.push([cx, cy, cx + f64::from(dx) / 8.0, cy + f64::from(dy) / 8.0]);
            }
        }
    }
    let inliers = ransac(&pairs, seed)?;
    let bits = |params: &[i32; 6]| param_bits(params).map(|b| b * sad_per_bit / 256);
    let zero = warp_sad(frames, &IDENTITY)?;
    let n = inliers.len() as f64;
    let shift = [2, 3].map(|k| inliers.iter().map(|p| p[k] - p[k - 2]).sum::<f64>() / n);
    let translation = quantize([1.0, 0.0, 0.0, 1.0, shift[0], shift[1]], true)
        .and_then(|params| warp_sad(frames, &params))
        .unwrap_or(u64::MAX);
    let mut best: Option<(u64, [i32; 6])> = None;
    for rotzoom in [true, false] {
        let Some(start) = least_squares(&inliers, rotzoom).and_then(|m| quantize(m, rotzoom))
        else {
            continue;
        };
        let cost = |params: &[i32; 6]| Some(warp_sad(frames, params)? + bits(params)?);
        if let Some((total, params)) = descend(start, rotzoom, cost)
            && best.is_none_or(|(least, _)| total < least)
        {
            best = Some((total, params));
        }
    }
    best.filter(|&(total, params)| total < zero.min(translation) && params != IDENTITY)
        .map(|(_, params)| params)
}

/// The vector of the block `(x, y, side)` in 1/8 samples from `start`: a
/// diamond descent on the source luma by `range`, then by halves down to a
/// quarter sample; `None` for a block outside the frame.
fn refine_vector(
    frames: &LumaPair<'_>,
    (x, y, side): (usize, usize, usize),
    start: (i32, i32),
    range: i32,
) -> Option<(i32, i32)> {
    if x + side > frames.width || y + side > frames.height {
        return None;
    }
    let mut predicted = vec![0; side * side];
    let mut best = (
        block_sad(frames, (x, y, side), start, &mut predicted)?,
        start,
    );
    let mut step = range.max(8);
    while step >= 2 {
        let mut moved = true;
        while moved {
            moved = false;
            let (_, (mx, my)) = best;
            for (dx, dy) in [
                (-1, 0),
                (1, 0),
                (0, -1),
                (0, 1),
                (-1, -1),
                (1, 1),
                (-1, 1),
                (1, -1),
            ] {
                let mv = (mx + dx * step, my + dy * step);
                if (mv.0 - start.0).abs() > range + 8 || (mv.1 - start.1).abs() > range + 8 {
                    continue;
                }
                if let Some(sad) = block_sad(frames, (x, y, side), mv, &mut predicted)
                    && sad < best.0
                {
                    (best, moved) = ((sad, mv), true);
                }
            }
        }
        step /= 2;
    }
    Some(best.1)
}

/// The luma SAD of the block `(x, y, side)` against its 8-tap prediction
/// from the reference at `mv` (1/8 samples).
fn block_sad(
    frames: &LumaPair<'_>,
    (x, y, side): (usize, usize, usize),
    (mx, my): (i32, i32),
    predicted: &mut [u16],
) -> Option<u64> {
    let view = ReferencePlaneView::new(frames.reference, frames.width, frames.height).ok()?;
    let params = SubpelPredictParams {
        interp: InterpolationFilter::EightTap,
        w: side,
        h: side,
        start_x: ((x as i32) * 8 + mx) << 7,
        start_y: ((y as i32) * 8 + my) << 7,
        step_x: 1 << 10,
        step_y: 1 << 10,
        first_x: 0,
        first_y: 0,
        last_x: frames.width as i32 - 1,
        last_y: frames.height as i32 - 1,
        bit_depth: frames.bit_depth,
    };
    subpel_predict_block_into(&view, &params, predicted).ok()?;
    let rows = frames.current[y * frames.width..].chunks(frames.width);
    let sad = rows
        .zip(predicted.chunks_exact(side))
        .map(|(row, pred)| {
            let row = &row[x..x + side];
            row.iter()
                .zip(pred)
                .map(|(&a, &b)| u64::from(a.abs_diff(b)))
                .sum::<u64>()
        })
        .sum();
    Some(sad)
}

/// The luma SAD of the decoder's block warp with `params` over the cost grid;
/// `None` for a model whose shear the decoder refuses.
fn warp_sad(frames: &LumaPair<'_>, params: &[i32; 6]) -> Option<u64> {
    if !warp_shear_is_valid(*params) {
        return None;
    }
    let view = ReferencePlaneView::new(frames.reference, frames.width, frames.height).ok()?;
    let max = i32::from(frames.bit_depth.max_sample());
    let mut predicted = [0; 64];
    let mut sad = 0;
    for y in (0..frames.height.saturating_sub(7)).step_by(COST_GRID) {
        for x in (0..frames.width.saturating_sub(7)).step_by(COST_GRID) {
            let block = WarpPredictBlockParams {
                warp_params: *params,
                block_x: x as i32,
                block_y: y as i32,
                subsampling_x: 0,
                subsampling_y: 0,
                reference_scale_x: UNSCALED,
                reference_scale_y: UNSCALED,
                first_x: 0,
                first_y: 0,
                last_x: frames.width as i32 - 1,
                last_y: frames.height as i32 - 1,
                bit_depth: frames.bit_depth,
            };
            warp_predict_block_into(&view, &block, false, &mut predicted).ok()?;
            for (r, pred) in predicted.chunks_exact(8).enumerate() {
                let row = &frames.current[(y + r) * frames.width + x..][..8];
                for (&a, &b) in row.iter().zip(pred) {
                    sad += u64::from((i32::from(a) - b.clamp(0, max)).unsigned_abs());
                }
            }
        }
    }
    Some(sad)
}

/// The inliers of the `ROTZOOM` or `AFFINE` hypothesis that most pairs fit;
/// `None` with too few pairs.
fn ransac(pairs: &[Pair], seed: u64) -> Option<Vec<Pair>> {
    if pairs.len() < MIN_PAIRS {
        return None;
    }
    let mut next = sampler(seed);
    let fits = |model: &Model, &[x, y, u, v]: &Pair| {
        let (eu, ev) = (
            model[0] * x + model[1] * y + model[4] - u,
            model[2] * x + model[3] * y + model[5] - v,
        );
        eu * eu + ev * ev <= INLIER_SAMPLES * INLIER_SAMPLES
    };
    let mut best: Option<(usize, Model)> = None;
    for _ in 0..ITERATIONS {
        let picked = [next(pairs.len()), next(pairs.len()), next(pairs.len())].map(|i| pairs[i]);
        for model in [
            least_squares(&picked[..2], true),
            least_squares(&picked, false),
        ]
        .into_iter()
        .flatten()
        {
            let count = pairs.iter().filter(|pair| fits(&model, pair)).count();
            if best.is_none_or(|(most, _)| count > most) {
                best = Some((count, model));
            }
        }
    }
    let (_, model) = best?;
    let inliers: Vec<Pair> = pairs.iter().copied().filter(|p| fits(&model, p)).collect();
    (inliers.len() >= MIN_PAIRS).then_some(inliers)
}

/// The least-squares `ROTZOOM` (with `rotzoom`) or `AFFINE` model of
/// `pairs`; `None` for degenerate points.
fn least_squares(pairs: &[Pair], rotzoom: bool) -> Option<Model> {
    let n = pairs.len() as f64;
    let mean = |k: usize| pairs.iter().map(|pair| pair[k]).sum::<f64>() / n;
    let (mx, my, mu, mv) = (mean(0), mean(1), mean(2), mean(3));
    let centred = pairs
        .iter()
        .map(|&[x, y, u, v]| (x - mx, y - my, u - mu, v - mv));
    let (mut sxx, mut sxy, mut syy, mut sxu, mut syu, mut sxv, mut syv) =
        (0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0);
    for (x, y, u, v) in centred {
        (sxx, sxy, syy) = (sxx + x * x, sxy + x * y, syy + y * y);
        (sxu, syu, sxv, syv) = (sxu + x * u, syu + y * u, sxv + x * v, syv + y * v);
    }
    let [m2, m3, m4, m5] = if rotzoom {
        let den = sxx + syy;
        if den < 1.0 {
            return None;
        }
        let (a, c) = ((sxu + syv) / den, (syu - sxv) / den);
        [a, c, -c, a]
    } else {
        let det = sxx * syy - sxy * sxy;
        if det.abs() < 1.0 {
            return None;
        }
        let solve = |bx: f64, by: f64| ((bx * syy - by * sxy) / det, (by * sxx - bx * sxy) / det);
        let ((m2, m3), (m4, m5)) = (solve(sxu, syu), solve(sxv, syv));
        [m2, m3, m4, m5]
    };
    Some([
        m2,
        m3,
        m4,
        m5,
        mu - m2 * mx - m3 * my,
        mv - m4 * mx - m5 * my,
    ])
}

/// `model` on the coded lattice as `gm_params`, `ROTZOOM` terms tied with
/// `rotzoom`; `None` past the coded ranges.
fn quantize([m2, m3, m4, m5, tx, ty]: Model, rotzoom: bool) -> Option<[i32; 6]> {
    let steps = |value: f64, step: i32, max: i32| {
        let steps = (value * PREC / f64::from(step)).round();
        (steps.abs() <= f64::from(max)).then_some(steps as i32 * step)
    };
    let (m4, m5) = if rotzoom { (-m3, m2) } else { (m4, m5) };
    Some([
        steps(tx, TRANS_STEP, TRANS_MAX)?,
        steps(ty, TRANS_STEP, TRANS_MAX)?,
        (1 << 16) + steps(m2 - 1.0, ALPHA_STEP, ALPHA_MAX)?,
        steps(m3, ALPHA_STEP, ALPHA_MAX)?,
        steps(m4, ALPHA_STEP, ALPHA_MAX)?,
        (1 << 16) + steps(m5 - 1.0, ALPHA_STEP, ALPHA_MAX)?,
    ])
}

/// The coordinate descent of `start` on the coded lattice under `cost`, and
/// its cost; `ROTZOOM` moves its tied terms together.
fn descend(
    start: [i32; 6],
    rotzoom: bool,
    cost: impl Fn(&[i32; 6]) -> Option<u64>,
) -> Option<(u64, [i32; 6])> {
    let mut best = (cost(&start)?, start);
    let terms: &[usize] = if rotzoom {
        &[2, 3, 0, 1]
    } else {
        &[2, 3, 4, 5, 0, 1]
    };
    let mut multiple = DESCENT_START;
    while multiple >= 1 {
        let mut moved = true;
        while moved {
            moved = false;
            for &term in terms {
                for sign in [-1, 1] {
                    let mut params = best.1;
                    let step = if term < 2 { TRANS_STEP } else { ALPHA_STEP };
                    params[term] += sign * multiple * step;
                    if rotzoom {
                        (params[4], params[5]) = (-params[3], params[2]);
                    }
                    if !on_lattice(&params) {
                        continue;
                    }
                    if let Some(total) = cost(&params)
                        && total < best.0
                    {
                        (best, moved) = ((total, params), true);
                    }
                }
            }
        }
        multiple /= 2;
    }
    Some(best)
}

/// Whether every term of `params` lies within its coded range.
fn on_lattice(params: &[i32; 6]) -> bool {
    params.iter().enumerate().all(|(i, &value)| {
        let (step, max) = if i < 2 {
            (TRANS_STEP, TRANS_MAX)
        } else {
            (ALPHA_STEP, ALPHA_MAX)
        };
        let offset = if i % 3 == 2 { value - (1 << 16) } else { value };
        (offset / step).abs() <= max
    })
}

/// The bits of `is_global`, `is_rot_zoom` and the coded terms of `params`
/// against `Default_Warp_Params` (§ 5.18.9.1, § 5.18.9.2).
fn param_bits(params: &[i32; 6]) -> Option<u64> {
    let rotzoom = params[4] == -params[3] && params[5] == params[2];
    let terms: &[usize] = if rotzoom {
        &[2, 3, 0, 1]
    } else {
        &[2, 3, 4, 5, 0, 1]
    };
    let mut writer = BitWriter::new();
    for &term in terms {
        write_global_param(&mut writer, term, IDENTITY[term], params[term]).ok()?;
    }
    Some(writer.bit_len() + 2)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    /// A smooth texture warped by `params` about the frame: the current frame
    /// samples the reference at the model position.
    fn warped(width: usize, height: usize, params: [f64; 6]) -> (Vec<u16>, Vec<u16>) {
        let texture = |x: f64, y: f64| {
            let wave = |t: f64, period: f64| (t.rem_euclid(period) - period / 2.0).abs();
            (wave(x, 37.0) * 3.0 + wave(y, 29.0) * 4.0 + wave(x + y, 53.0) * 2.0) as u16
        };
        let reference = (0..width * height)
            .map(|i| texture((i % width) as f64, (i / width) as f64))
            .collect();
        let current = (0..width * height)
            .map(|i| {
                let (x, y) = ((i % width) as f64, (i / width) as f64);
                let [m2, m3, m4, m5, tx, ty] = params;
                texture(m2 * x + m3 * y + tx, m4 * x + m5 * y + ty)
            })
            .collect();
        (current, reference)
    }

    fn lookahead_vectors(params: [f64; 6], (cols, rows): (usize, usize)) -> Vec<BlockMotion> {
        let [m2, m3, m4, m5, tx, ty] = params;
        (0..cols * rows)
            .map(|i| {
                let (x, y) = ((i % cols * 16 + 8) as f64, (i / cols * 16 + 8) as f64);
                let (u, v) = (m2 * x + m3 * y + tx, m4 * x + m5 * y + ty);
                BlockMotion {
                    cost: 0,
                    mv: (
                        ((u - x) / 2.0).round() as i32,
                        ((v - y) / 2.0).round() as i32,
                    ),
                }
            })
            .collect()
    }

    fn run(params: [f64; 6]) -> Option<[i32; 6]> {
        let (width, height, cols, rows) = (256, 192, 16, 12);
        let (current, reference) = warped(width, height, params);
        let frames = LumaPair {
            current: &current,
            reference: &reference,
            width,
            height,
            bit_depth: BitDepth::Eight,
        };
        let motion = lookahead_vectors(params, (cols, rows));
        estimate(
            &frames,
            (&motion, &vec![1000; cols * rows]),
            ((cols, rows), 16),
            64,
            3,
        )
    }

    /// A zoom with a rotation and a shear gives back an `AFFINE` model within
    /// two coded steps of each term.
    #[test]
    fn the_estimator_recovers_an_affine_model() {
        let params = [
            1.0 + 1.0 / 64.0,
            1.0 / 128.0,
            -1.0 / 96.0,
            1.0 - 1.0 / 80.0,
            2.25,
            -1.5,
        ];
        let found = run(params).unwrap();
        let want = quantize(params, false).unwrap();
        for term in 2..6 {
            assert!(
                (found[term] - want[term]).abs() <= 2 * ALPHA_STEP,
                "{found:?} {want:?}"
            );
        }
        assert!(found[4] != -found[3], "{found:?} is no AFFINE model");
    }

    /// A pure translation codes no model: blocks code it as vectors.
    #[test]
    fn a_translation_codes_no_model() {
        assert_eq!(run([1.0, 0.0, 0.0, 1.0, 3.25, -2.5]), None);
    }
}
