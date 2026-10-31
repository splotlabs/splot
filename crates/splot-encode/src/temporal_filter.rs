// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Temporal filtering of key frames and hidden mini-GOP base frames before
//! they are coded.
//!
//! A key frame takes up to the preset's `tf_key_frames` later neighbours and
//! a base frame up to `tf_base_frames` neighbours on each side, nearest
//! first, while the encoder holds them; a frame whose estimated noise is
//! below [`CLEAN_NOISE`] takes one per side. The motion of the frame against each neighbour comes from
//! the lookahead's open-loop search. Each 64x64 block of every plane is a
//! weighted mean of the frame (weight [`MAX_WEIGHT`]) and the
//! motion-compensated neighbours. A neighbour's weight is the same in all
//! planes of a 16x16 luma quadrant: the 16x16 vectors when their SADs beat
//! the 32x32 vector's by [`SPLIT_EIGHTHS`], else the 32x32 vector, and
//! `exp(-error * distance / decay)` from a table, where `error` mixes the
//! quadrant's mean squared prediction error with the mean squared error that
//! the block's SAD implies, `distance` grows with the vector beyond
//! [`FREE_MV`] samples, and `decay` grows with the noise, the qindex and the
//! preset's `tf_key_strength` or `tf_base_strength`. The noise is the mean squared error that the
//! lower quartile of the 8x8 SADs against the nearest neighbour implies.
//!
//! The filtered frame replaces the source for coding only: statistics and the
//! rest of the encoder keep the source. A filtered frame whose luma mean
//! squared error against the source is above `qindex² / KEEP_DIVISOR / 16`,
//! with a high-bit-depth qindex moved down to the 8-bit step,
//! is not used; the frame codes its source, and a hidden frame is output by
//! its show-existing frame either way. Arithmetic is integer and per block,
//! so the result does not depend on the thread count.

use splot_parallel::WorkerPool;
use splot_parallel::prelude::*;
use splot_recon::BitDepth;

use crate::frame::SourceFrame;
use crate::gop::GopFrame;
use crate::hme::{self, FrameMotion, SB};
use crate::kernels::motion::Motion;
use crate::kernels::temporal_filter::{MAX_WEIGHT, QUADRANTS};
use crate::kernels::{BlockDims, kernels};
use crate::lookahead::{Lookahead, Sources};
use crate::rate_control::{FrameType, max_qindex};

/// The side of a weight quadrant in luma samples.
const QUAD: usize = 16;
/// Quadrants per block side.
const GRID: usize = SB / QUAD;
/// A 32x32 block takes its 16x16 vectors when their SADs sum to at most this
/// many eighths of its own SAD.
const SPLIT_EIGHTHS: u64 = 7;
/// Vectors up to this length in luma samples do not lower a weight.
const FREE_MV: u32 = 16;
/// Noise, as a mean squared frame difference in 1/16, below which a frame
/// takes one neighbour per side.
const CLEAN_NOISE: u32 = 4;
/// See the module documentation.
const KEEP_DIVISOR: u64 = 32;
/// The largest quadrant error, in 1/16, that the weights distinguish.
const MAX_ERROR: u32 = 1 << 20;

/// One neighbour of the centre frame and the motion of the centre against it.
struct Neighbour<'a> {
    frame: &'a SourceFrame,
    motion: FrameMotion,
}

/// Whether frames of `plan` are filtered: key frames and hidden frames of
/// mini-GOP layer 0.
fn filters(plan: &GopFrame) -> bool {
    plan.frame_type == FrameType::Key || (!plan.show_frame && plan.layer == 0)
}

/// The filtered frame of `plan` at `qindex` from the frames `sources`
/// holds, with the `(neighbours per side, strength percent)` of `settings`
/// (`[key frames, base frames]`); `None` when the frame is not filtered or
/// the filtered frame is not used. `subsampling` is the chroma subsampling
/// `[x, y]`.
pub(crate) fn filter_frame(
    lookahead: &Lookahead,
    sources: Sources<'_>,
    plan: &GopFrame,
    qindex: u16,
    settings: [(u8, u8); 2],
    subsampling: [u8; 2],
    pool: Option<&WorkerPool>,
) -> Option<SourceFrame> {
    let key = plan.frame_type == FrameType::Key;
    let (frames, strength) = settings[usize::from(!key)];
    if frames == 0 || strength == 0 || !filters(plan) {
        return None;
    }
    let display = plan.display_index;
    let centre = sources(display)?;
    let extra = max_qindex(centre.bit_depth()) - max_qindex(BitDepth::Eight);
    let qindex = qindex.saturating_sub(extra);
    let centre_full = hme::full_level(centre);
    let neighbour = |other: u64| {
        let frame = sources(other)?;
        let full = hme::full_level(frame);
        let motion = lookahead.full_motion((display, other), (&centre_full, &full))?;
        Some(Neighbour { frame, motion })
    };
    let mut open = [true, !key];
    let (mut neighbours, mut noise) = (Vec::new(), 0);
    for distance in 1..=u64::from(frames) {
        for (side, open) in open.iter_mut().enumerate() {
            let other = match side {
                0 => display.checked_add(distance),
                _ => display.checked_sub(distance),
            };
            match other.filter(|_| *open).and_then(neighbour) {
                Some(found) => neighbours.push(found),
                None => *open = false,
            }
        }
        if distance == 1 {
            noise = neighbours.first().map_or(0, |n| noise_level(&n.motion));
        }
        if noise < CLEAN_NOISE {
            break;
        }
    }
    if neighbours.is_empty() {
        return None;
    }
    let (planes, luma_sse) = filter(
        centre,
        &neighbours,
        decay(noise, qindex, strength),
        subsampling,
        pool,
    );
    let size = centre.plane_sizes()[0];
    let mse = luma_sse * 16 / (size.width() * size.height()) as u64;
    (mse * KEEP_DIVISOR <= u64::from(qindex).pow(2)).then(|| centre.with_planes(planes))
}

/// The mean squared frame difference, in 1/16, that the lower quartile of the
/// 8x8 SADs of `motion` implies.
fn noise_level(motion: &FrameMotion) -> u32 {
    let mut sads: Vec<u32> = motion
        .blocks(0)
        .1
        .iter()
        .map(|m| m.sad)
        .filter(|&sad| sad != Motion::NONE.sad)
        .collect();
    if sads.is_empty() {
        return 0;
    }
    let quartile = sads.len() / 4;
    let (_, &mut sad, _) = sads.select_nth_unstable(quartile);
    block_error(sad, 64)
}

/// The mean squared error, in 1/16, of a block of `samples` samples whose
/// SAD is `sad`: about `pi / 2` times the squared mean absolute error.
fn block_error(sad: u32, samples: u64) -> u32 {
    let mad = u64::from(sad) * 16 / samples;
    u32::try_from(mad.saturating_mul(mad).saturating_mul(25) / 256)
        .unwrap_or(MAX_ERROR)
        .min(MAX_ERROR)
}

/// The weight decay at `noise` (in 1/16), `qindex` and `strength` percent.
fn decay(noise: u32, qindex: u16, strength: u8) -> u32 {
    let q = u64::from(qindex.max(16));
    let value = 2 * (64 + 2 * u64::from(noise)) * (q * q / 16) * u64::from(strength) / 25_600;
    u32::try_from(value.max(1)).unwrap_or(u32::MAX)
}

/// The filtered planes of `centre` and their luma SSE against it.
fn filter(
    centre: &SourceFrame,
    neighbours: &[Neighbour<'_>],
    decay: u32,
    subsampling: [u8; 2],
    pool: Option<&WorkerPool>,
) -> ([Vec<u16>; 3], u64) {
    let rows = centre.plane_sizes()[0].height().div_ceil(SB);
    let row = |by: usize| filter_row(centre, neighbours, decay, subsampling, by);
    let strips: Vec<([Vec<u16>; 3], u64)> = match pool {
        Some(pool) => pool.install(|| (0..rows).into_par_iter().map(row).collect()),
        None => (0..rows).map(row).collect(),
    };
    let mut planes: [Vec<u16>; 3] = Default::default();
    let mut sse = 0;
    for (strip, strip_sse) in strips {
        for (plane, rows) in planes.iter_mut().zip(strip) {
            plane.extend(rows);
        }
        sse += strip_sse;
    }
    (planes, sse)
}

/// One plane of a frame: samples, width and height.
type PlaneRef<'a> = (&'a [u16], usize, usize);

/// The planes of `frame` with their sizes.
fn planes(frame: &SourceFrame) -> Vec<PlaneRef<'_>> {
    frame
        .plane_sizes()
        .iter()
        .zip(frame.planes())
        .map(|(size, samples)| (samples.as_slice(), size.width(), size.height()))
        .collect()
}

/// The filtered rows of block row `by` of each plane and their luma SSE.
fn filter_row(
    centre: &SourceFrame,
    neighbours: &[Neighbour<'_>],
    decay: u32,
    subsampling: [u8; 2],
    by: usize,
) -> ([Vec<u16>; 3], u64) {
    let centre_planes = planes(centre);
    let neighbour_planes: Vec<_> = neighbours.iter().map(|n| planes(n.frame)).collect();
    let shift = 2 * u32::from(centre.bit_depth().bits() - 8);
    let cols = centre_planes[0].1.div_ceil(SB);
    let mut out: [Vec<u16>; 3] = Default::default();
    let mut spans = [(0, 0); 3];
    for (p, &(_, width, height)) in centre_planes.iter().enumerate() {
        let sy = if p == 0 { 0 } else { subsampling[1] };
        let top = (by * SB) >> sy;
        let bottom = (((by + 1) * SB) >> sy).min(height);
        spans[p] = (top, bottom);
        out[p] = vec![0; (bottom - top) * width];
    }
    let (mut own, mut pred) = (vec![0u16; SB * SB], vec![0u16; SB * SB]);
    let mut accum = vec![0u32; SB * SB];
    let mut weights = vec![[0u32; QUADRANTS]; neighbours.len()];
    for bx in 0..cols {
        let vectors: Vec<_> = neighbours
            .iter()
            .map(|n| quadrant_vectors(&n.motion, (bx, by)))
            .collect();
        for (p, &(samples, width, height)) in centre_planes.iter().enumerate() {
            let (sx, sy) = if p == 0 {
                (0, 0)
            } else {
                (subsampling[0], subsampling[1])
            };
            let (bw, bh, qw, qh) = (SB >> sx, SB >> sy, QUAD >> sx, QUAD >> sy);
            let (x0, y0) = ((bx * SB) >> sx, (by * SB) >> sy);
            let at = (x0 as isize, y0 as isize);
            fetch((samples, width, height), at, (bw, bh), &mut own);
            accum[..bw * bh].fill(0);
            kernels().tf_accumulate(&own[..bw * bh], MAX_WEIGHT, &mut accum);
            let mut counts = [MAX_WEIGHT; QUADRANTS];
            for (n, vectors) in vectors.iter().enumerate() {
                for (q, vector) in vectors.iter().enumerate() {
                    let mv = vector.map_or([0, 0], |(mv, _)| mv);
                    let round = |v: i16, s: u8| (isize::from(v) + isize::from(s)) >> s;
                    let (qx, qy) = (q % GRID * qw, q / GRID * qh);
                    let origin = (
                        at.0 + (qx as isize) + round(mv[0], sx),
                        at.1 + (qy as isize) + round(mv[1], sy),
                    );
                    let start = qy * bw + qx;
                    fetch_rows(
                        neighbour_planes[n][p],
                        origin,
                        (qw, qh),
                        bw,
                        &mut pred[start..],
                    );
                }
                if p == 0 {
                    weights[n] = quadrant_weights(&own, &pred, vectors, decay, shift);
                }
                for (q, &weight) in weights[n].iter().enumerate() {
                    counts[q] += weight;
                    let start = q / GRID * qh * bw + q % GRID * qw;
                    for row in 0..qh {
                        let i = start + row * bw;
                        kernels().tf_accumulate(&pred[i..i + qw], weight, &mut accum[i..i + qw]);
                    }
                }
            }
            let (top, bottom) = spans[p];
            for row in 0..bottom - top {
                let line = &mut out[p][row * width..][..width];
                for (q, &count) in counts.iter().enumerate().skip(row / qh * GRID).take(GRID) {
                    let x = x0 + q % GRID * qw;
                    if x >= width {
                        break;
                    }
                    let len = qw.min(width - x);
                    let i = row * bw + q % GRID * qw;
                    kernels().tf_normalize(&accum[i..i + len], count, &mut line[x..x + len]);
                }
            }
        }
    }
    let (luma, width, _) = centre_planes[0];
    let (top, _) = spans[0];
    let sse = out[0]
        .chunks(width)
        .enumerate()
        .map(|(row, filtered)| kernels().sse_row(filtered, &luma[(top + row) * width..][..width]))
        .sum::<u64>();
    (out, sse >> shift)
}

/// The vector and SAD-implied error (in 1/16) of each 16x16 quadrant of
/// block `(bx, by)` in `motion`; `None` where the search found none.
fn quadrant_vectors(
    motion: &FrameMotion,
    (bx, by): (usize, usize),
) -> [Option<([i16; 2], u32)>; QUADRANTS] {
    let ((cols16, small), (cols32, large)) = (motion.blocks(1), motion.blocks(2));
    let mut out = [None; QUADRANTS];
    for half in 0..4 {
        let (hx, hy) = (half % 2, half / 2);
        let big = large[(by * 2 + hy) * cols32 + bx * 2 + hx];
        let quads: [usize; 4] = core::array::from_fn(|i| (2 * hy + i / 2) * GRID + 2 * hx + i % 2);
        let parts = quads.map(|q| small[(by * GRID + q / GRID) * cols16 + bx * GRID + q % GRID]);
        let sum: u64 = parts.iter().map(|m| u64::from(m.sad)).sum();
        let split = sum * 8 <= u64::from(big.sad) * SPLIT_EIGHTHS;
        for (q, part) in quads.into_iter().zip(parts) {
            let (chosen, samples) = if split {
                (part, (QUAD * QUAD) as u64)
            } else {
                (big, (4 * QUAD * QUAD) as u64)
            };
            out[q] = (chosen.sad != Motion::NONE.sad)
                .then(|| (chosen.mv, block_error(chosen.sad, samples)));
        }
    }
    out
}

/// The weight of each quadrant of the luma prediction `pred` of the block
/// `own` (both 64x64, stride [`SB`]), at the `vectors` of
/// [`quadrant_vectors`] and `decay`; `shift` scales squared errors to 8 bits.
fn quadrant_weights(
    own: &[u16],
    pred: &[u16],
    vectors: &[Option<([i16; 2], u32)>; QUADRANTS],
    decay: u32,
    shift: u32,
) -> [u32; QUADRANTS] {
    let dims = BlockDims::new(QUAD, QUAD).unwrap_or(BlockDims::SQUARE_8);
    let (mut errors, mut distances) = ([MAX_ERROR; QUADRANTS], [16; QUADRANTS]);
    for (q, vector) in vectors.iter().enumerate() {
        let Some((mv, block)) = *vector else {
            continue;
        };
        let start = q / GRID * QUAD * SB + q % GRID * QUAD;
        let sse = kernels().sse(&own[start..], SB, &pred[start..], SB, dims);
        let window = u32::try_from((sse / 16) >> shift).unwrap_or(MAX_ERROR);
        errors[q] = ((3 * u64::from(window.min(MAX_ERROR)) + u64::from(block)) / 4) as u32;
        let (x, y) = (
            u32::from(mv[0].unsigned_abs()),
            u32::from(mv[1].unsigned_abs()),
        );
        let length = x.max(y) + x.min(y) / 2;
        distances[q] = (length * 16 / FREE_MV).max(16);
    }
    let mut weights = [0; QUADRANTS];
    kernels().tf_weights(&errors, &distances, decay, &mut weights);
    for (weight, vector) in weights.iter_mut().zip(vectors) {
        if vector.is_none() {
            *weight = 0;
        }
    }
    weights
}

/// Copies the `w x h` block of `plane` at `at`, replicating its edge
/// samples, into `out` rows of `w`.
fn fetch(plane: PlaneRef<'_>, at: (isize, isize), (w, h): (usize, usize), out: &mut [u16]) {
    fetch_rows(plane, at, (w, h), w, out);
}

/// [`fetch`] into `out` rows of `stride`.
fn fetch_rows(
    (samples, width, height): PlaneRef<'_>,
    (x, y): (isize, isize),
    (w, h): (usize, usize),
    stride: usize,
    out: &mut [u16],
) {
    let clamp = |v: isize, extent: usize| v.clamp(0, extent as isize - 1) as usize;
    for row in 0..h {
        let line = &samples[clamp(y + row as isize, height) * width..][..width];
        let dst = &mut out[row * stride..][..w];
        match usize::try_from(x) {
            Ok(left) if left + w <= width => dst.copy_from_slice(&line[left..left + w]),
            _ => {
                for (col, sample) in dst.iter_mut().enumerate() {
                    *sample = line[clamp(x + col as isize, width)];
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "temporal_filter_tests.rs"]
mod tests;
