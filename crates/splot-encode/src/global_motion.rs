// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Global motion of inter frames (§ 5.18.9.1).
//!
//! Each lookahead block that predicts better from a reference than from
//! intra, and has texture, gives a point pair: its centre and that centre
//! moved by its full-sample vector. RANSAC over the pairs keeps the
//! translation or rotation-zoom model with the most pairs within
//! [`INLIER_SAMPLES`] samples, refits it to them by least squares, and keeps
//! it when they are at least [`MIN_INLIER_PERCENT`] of the pairs and its zoom
//! and rotation move a frame corner by at least [`MIN_CORNER_SAMPLES`]
//! against the translation alone: below that, `GLOBALMV` predicts no better
//! than a translational vector. The model goes to the coded lattice as
//! `ROTZOOM` `gm_params`.

#![allow(
    clippy::many_single_char_names,
    reason = "model terms and point coordinates keep their geometric names"
)]

use std::collections::BTreeMap;

use crate::gop::GopFrame;
use crate::lookahead::{BLOCK, BlockMotion, Lookahead, Sources};
use crate::rate_control::FrameType;

/// The `ROTZOOM` `gm_params` of each `(frame, reference)` pair of display
/// indices that has a global model.
pub(crate) type GlobalModels = BTreeMap<(u64, u64), [i32; 6]>;

/// RANSAC hypotheses per pair of frames.
const ITERATIONS: usize = 64;
/// The distance, in full-resolution luma samples, within which a pair fits.
const INLIER_SAMPLES: f64 = 1.5;
/// The share of pairs a model must fit.
const MIN_INLIER_PERCENT: usize = 50;
/// The least corner motion, in luma samples, of the zoom and rotation of a
/// kept model.
const MIN_CORNER_SAMPLES: f64 = 1.0;
/// The pairs a frame needs before any model is trusted.
const MIN_PAIRS: usize = 16;
/// The intra SATD below which a block is too flat for its vector to count.
const MIN_TEXTURE: u32 = 64;
/// `WARPEDMODEL_PREC_BITS` (§ 3).
const PREC: f64 = 65536.0;
/// The coded steps of the scale and rotation terms and of the translation
/// (`GM_ALPHA_PREC_BITS`, `GM_TRANS_PREC_BITS`), in `PREC` units.
const ALPHA_STEP: i32 = 1 << 6;
const TRANS_STEP: i32 = 1 << 13;
/// The coded ranges of those terms, in steps (`GM_ALPHA_MAX`, `GM_TRANS_MAX`).
const ALPHA_MAX: i32 = 511;
const TRANS_MAX: i32 = 16383;

/// The models of the frames of `plan`, in coding order, against their
/// nearest past and future coded frames; `coded_before` is the last frame
/// coded before the plan. The motion search reads `sources`.
pub(crate) fn estimate(
    lookahead: &mut Lookahead,
    plan: &[GopFrame],
    coded_before: Option<u64>,
    sources: Sources<'_>,
) -> GlobalModels {
    let (cols, rows) = lookahead.grid();
    let block = lookahead.block_luma();
    let mut coded: Vec<u64> = coded_before.into_iter().collect();
    let mut models = GlobalModels::new();
    for frame in plan {
        let display = frame.display_index;
        if frame.frame_type == FrameType::Key {
            coded.clear();
        }
        let past = coded.iter().filter(|&&d| d < display).max().copied();
        let future = coded.iter().filter(|&&d| d > display).min().copied();
        coded.push(display);
        let Some(intra) = lookahead.intra(display).map(<[u32]>::to_vec) else {
            continue;
        };
        for reference in past.into_iter().chain(future) {
            let Some(motion) = lookahead.motion(display, reference, sources) else {
                continue;
            };
            let pairs = pairs(motion, &intra, (cols, rows), block);
            let frame = ((cols * block) as f64, (rows * block) as f64);
            if let Some(model) = fit(&pairs, frame, display ^ reference.rotate_left(32)) {
                models.insert((display, reference), model);
            }
        }
    }
    models
}

/// A block centre and where its vector moves it, in luma samples.
type Pair = [f64; 4];

/// The pairs of the textured blocks that predict better from the reference
/// than from intra.
fn pairs(
    motion: &[BlockMotion],
    intra: &[u32],
    (cols, rows): (usize, usize),
    block: usize,
) -> Vec<Pair> {
    let scale = (block / BLOCK) as f64;
    let mut out = Vec::new();
    for row in 0..rows {
        for col in 0..cols {
            let index = row * cols + col;
            let (Some(motion), Some(&intra)) = (motion.get(index), intra.get(index)) else {
                continue;
            };
            if intra < MIN_TEXTURE || motion.cost >= intra {
                continue;
            }
            let x = (col * block) as f64 + block as f64 / 2.0;
            let y = (row * block) as f64 + block as f64 / 2.0;
            let (dx, dy) = (
                f64::from(motion.mv.0) * scale,
                f64::from(motion.mv.1) * scale,
            );
            out.push([x, y, x + dx, y + dy]);
        }
    }
    out
}

/// A rotation-zoom `[a, c, tx, ty]`: `u = a x + c y + tx`, `v = -c x + a y + ty`.
type Model = [f64; 4];

/// The `gm_params` that RANSAC over `pairs` of a `(width, height)` frame
/// finds, if any; `seed` makes the sampling deterministic.
fn fit(pairs: &[Pair], (width, height): (f64, f64), seed: u64) -> Option<[i32; 6]> {
    if pairs.len() < MIN_PAIRS {
        return None;
    }
    let mut state = seed | 1;
    let mut next = |n: usize| {
        state ^= state >> 12;
        state ^= state << 25;
        state ^= state >> 27;
        (state.wrapping_mul(0x2545_F491_4F6C_DD1D) % n as u64) as usize
    };
    let mut best: Option<(usize, Model)> = None;
    for _ in 0..ITERATIONS {
        let (i, j) = (next(pairs.len()), next(pairs.len()));
        let [x, y, u, v] = pairs[i];
        let translation = [1.0, 0.0, u - x, v - y];
        for model in [Some(translation), two_point(pairs[i], pairs[j])]
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
    let inliers: Vec<Pair> = pairs
        .iter()
        .copied()
        .filter(|pair| fits(&model, pair))
        .collect();
    if inliers.len() * 100 < pairs.len() * MIN_INLIER_PERCENT {
        return None;
    }
    let model = least_squares(&inliers)?;
    let [a, c, ..] = model;
    let corner = ((a - 1.0).abs() * width + c.abs() * height)
        .max((a - 1.0).abs() * height + c.abs() * width)
        / 2.0;
    if corner < MIN_CORNER_SAMPLES {
        return None;
    }
    quantize(model)
}

/// The rotation-zoom through two pairs; `None` when they coincide.
fn two_point([x1, y1, u1, v1]: Pair, [x2, y2, u2, v2]: Pair) -> Option<Model> {
    let (dx, dy, du, dv) = (x2 - x1, y2 - y1, u2 - u1, v2 - v1);
    let norm = dx * dx + dy * dy;
    if norm < 1.0 {
        return None;
    }
    let a = (du * dx + dv * dy) / norm;
    let c = (du * dy - dv * dx) / norm;
    Some([a, c, u1 - a * x1 - c * y1, v1 + c * x1 - a * y1])
}

fn fits([a, c, tx, ty]: &Model, &[x, y, u, v]: &Pair) -> bool {
    let (eu, ev) = (a * x + c * y + tx - u, -c * x + a * y + ty - v);
    eu * eu + ev * ev <= INLIER_SAMPLES * INLIER_SAMPLES
}

/// The least-squares rotation-zoom of `pairs`.
fn least_squares(pairs: &[Pair]) -> Option<Model> {
    let n = pairs.len() as f64;
    let mean = |k: usize| pairs.iter().map(|pair| pair[k]).sum::<f64>() / n;
    let (mx, my, mu, mv) = (mean(0), mean(1), mean(2), mean(3));
    let (mut num_a, mut num_c, mut den) = (0.0, 0.0, 0.0);
    for &[x, y, u, v] in pairs {
        let (x, y, u, v) = (x - mx, y - my, u - mu, v - mv);
        num_a += x * u + y * v;
        num_c += y * u - x * v;
        den += x * x + y * y;
    }
    if den < 1.0 {
        return None;
    }
    let (a, c) = (num_a / den, num_c / den);
    Some([a, c, mu - a * mx - c * my, mv + c * mx - a * my])
}

/// `model` on the coded `ROTZOOM` lattice; `None` for the identity or a
/// model past the coded ranges.
fn quantize([a, c, tx, ty]: Model) -> Option<[i32; 6]> {
    let steps = |value: f64, step: i32, max: i32| {
        let steps = (value * PREC / f64::from(step)).round();
        (steps.abs() <= f64::from(max)).then_some(steps as i32 * step)
    };
    let scale = (1 << 16) + steps(a - 1.0, ALPHA_STEP, ALPHA_MAX)?;
    let turn = steps(c, ALPHA_STEP, ALPHA_MAX)?;
    let (p0, p1) = (
        steps(tx, TRANS_STEP, TRANS_MAX)?,
        steps(ty, TRANS_STEP, TRANS_MAX)?,
    );
    let params = [p0, p1, scale, turn, -turn, scale];
    (params != [0, 0, 1 << 16, 0, 0, 1 << 16]).then_some(params)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    /// Pairs moved by a known zoom, rotation and shift, with a third of them
    /// outliers, give back that model on the coded lattice.
    #[test]
    fn ransac_recovers_a_rotation_zoom_among_outliers() {
        let (a, c, tx, ty) = (1.0 - 1.0 / 64.0, 1.0 / 128.0, 3.25, -1.5);
        let mut pairs = Vec::new();
        for row in 0..12 {
            for col in 0..16 {
                let (x, y) = (f64::from(col) * 16.0 + 8.0, f64::from(row) * 16.0 + 8.0);
                let (u, v) = (a * x + c * y + tx, -c * x + a * y + ty);
                let outlier = (row * 16 + col) % 3 == 0;
                let shift = if outlier { 9.0 + f64::from(col) } else { 0.0 };
                pairs.push([x, y, (u + shift).round(), (v - shift).round()]);
            }
        }
        let params = fit(&pairs, (256.0, 192.0), 7).unwrap();
        let step = |value: f64| (value * PREC / 64.0).round() as i32 * 64;
        assert!(
            (params[2] - (1 << 16) - step(a - 1.0)).abs() <= 64,
            "{params:?}"
        );
        assert!((params[3] - step(c)).abs() <= 64, "{params:?}");
        assert!(
            (params[0] - (tx * PREC) as i32).abs() <= 3 * 8192,
            "{params:?}"
        );
        assert_eq!((params[4], params[5]), (-params[3], params[2]));
        assert_eq!(params[0] % TRANS_STEP, 0);
    }

    /// Too few pairs, scattered vectors, the identity and a pure pan give no
    /// model.
    #[test]
    fn ransac_refuses_noise_and_the_identity() {
        let still: Vec<Pair> = (0..40)
            .map(|i| [f64::from(i) * 8.0, 4.0, f64::from(i) * 8.0, 4.0])
            .collect();
        assert_eq!(fit(&still, (320.0, 8.0), 1), None);
        assert_eq!(fit(&still[..8], (320.0, 8.0), 1), None);
        let noise: Vec<Pair> = (0..40)
            .map(|i| {
                let x = f64::from(i) * 8.0;
                [
                    x,
                    4.0,
                    x + f64::from((i * 37) % 23) - 11.0,
                    4.0 + f64::from((i * 11) % 19) - 9.0,
                ]
            })
            .collect();
        assert_eq!(fit(&noise, (320.0, 8.0), 1), None);
        let pan: Vec<Pair> = (0..40)
            .map(|i| [f64::from(i) * 8.0, 4.0, f64::from(i) * 8.0 + 3.0, 2.0])
            .collect();
        assert_eq!(
            fit(&pan, (320.0, 8.0), 1),
            None,
            "a pan is a translational vector"
        );
    }
}
