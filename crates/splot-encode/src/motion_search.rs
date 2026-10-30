// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Motion estimation of single-reference inter leaves.
//!
//! The luma reconstruction of each reference, and the source luma, become
//! pyramids of 2x2-average levels, built once per frame. A block search starts
//! at the coarsest level its size allows: it scores the seed vectors, searches
//! the preset's range scaled to that level around the best one, and refines
//! by two samples per finer level. Sub-pixel refinement then tries the half,
//! quarter and eighth positions the preset's depth allows around the best
//! vector with the decoder's own interpolation (§ 7.11.3.3 parameters from the
//! facade, `splot_recon` kernels), scored by SATD. Every score adds an
//! approximate vector rate. Compound candidates reuse the single-reference
//! results; they are ranked by the SATD of the averaged predictions of their
//! two lists. The search predicts with `EIGHTTAP`; a block's final vectors
//! then score every switchable filter.

use splot_decode::encoder_syntax::{IntraTileSyntax, MotionVector};
use splot_recon::{InterpolationFilter, ReferencePlaneView, subpel_predict_block_strided_into};

use crate::intra_frame::PaddedSource;
use crate::kernels::{BlockDims, kernels};
use crate::preset::PresetParams;
use crate::rd::{Bits, Lambda, log2_q8};
use crate::tile_writer::{Result, TileWriterError};

/// Coarse pyramid levels below full resolution.
const MAX_LEVEL: usize = 2;
/// Reference samples kept past each frame edge at full resolution: a 64-sample
/// block plus margin. A vector whose block reaches further out is not
/// searched, so a 128-sample leaf near a frame edge searches fewer vectors.
const EDGE: usize = 80;
/// Full samples, in `1/8` units.
const FULL: i32 = 8;
/// The eight neighbours of a sub-pixel refinement step, `(row, col)`.
const NEIGHBOURS: [(i32, i32); 8] = [
    (-1, -1),
    (-1, 0),
    (-1, 1),
    (0, -1),
    (0, 1),
    (1, -1),
    (1, 0),
    (1, 1),
];

/// One pyramid level: a plane padded by `pad` replicated samples per side.
struct Level {
    samples: Vec<u16>,
    stride: usize,
    width: usize,
    height: usize,
    pad: usize,
}

impl Level {
    /// The level of `width x height` samples read through `sample`, which
    /// clamps its arguments to the plane.
    fn padded(
        width: usize,
        height: usize,
        pad: usize,
        sample: impl Fn(usize, usize) -> u16,
    ) -> Self {
        let stride = width + 2 * pad;
        let mut samples = Vec::with_capacity(stride * (height + 2 * pad));
        for y in 0..height + 2 * pad {
            let row = y.saturating_sub(pad).min(height - 1);
            samples.extend((0..stride).map(|x| sample(row, x.saturating_sub(pad).min(width - 1))));
        }
        Self {
            samples,
            stride,
            width,
            height,
            pad,
        }
    }

    /// The 2x2 average of `self`.
    fn halved(&self, pad: usize) -> Self {
        let (width, height) = (self.width.div_ceil(2), self.height.div_ceil(2));
        Self::padded(width, height, pad, |y, x| {
            let at = |yy: usize, xx: usize| {
                let (yy, xx) = (yy.min(self.height - 1), xx.min(self.width - 1));
                u32::from(self.samples[(yy + self.pad) * self.stride + xx + self.pad])
            };
            ((at(2 * y, 2 * x)
                + at(2 * y, 2 * x + 1)
                + at(2 * y + 1, 2 * x)
                + at(2 * y + 1, 2 * x + 1)
                + 2)
                >> 2) as u16
        })
    }

    /// The samples from position `(x, y)` on, which may lie in the padding;
    /// `None` when a `w x h` block there leaves the padded plane.
    fn at(&self, x: i32, y: i32, w: usize, h: usize) -> Option<&[u16]> {
        let px = usize::try_from(x + self.pad as i32).ok()?;
        let py = usize::try_from(y + self.pad as i32).ok()?;
        (px + w <= self.stride && py + h <= self.height + 2 * self.pad)
            .then(|| &self.samples[py * self.stride + px..])
    }
}

/// A plane and its 2x2-average levels.
struct Pyramid {
    levels: Vec<Level>,
}

impl Pyramid {
    fn new(base: Level) -> Self {
        let mut levels = vec![base];
        for level in 1..=MAX_LEVEL {
            let next = levels[level - 1].halved((EDGE >> level) + 4);
            levels.push(next);
        }
        Self { levels }
    }
}

/// The motion search state of one frame.
pub(crate) struct MotionSearch {
    references: Vec<Pyramid>,
    source: Pyramid,
    range: i32,
    /// The preset's sub-pel depth, capped at the frame's vector precision:
    /// a finer position would not be codable.
    subpel_depth: u8,
    /// The frame's `NEWMV` difference step in `1/8` samples.
    mv_step: u32,
    /// `sqrt(lambda)` in Q8, the SAD-domain multiplier of a bit.
    sad_lambda_q8: u64,
}

impl MotionSearch {
    /// The pyramids of every reference of the frame `syntax` codes and of
    /// `source`; `None` in an intra frame.
    pub(crate) fn new(
        syntax: &IntraTileSyntax,
        source: &PaddedSource,
        preset: &PresetParams,
        lambda: Lambda,
    ) -> Result<Option<Self>> {
        if syntax.reference_count() == 0 {
            return Ok(None);
        }
        let mut references = Vec::with_capacity(syntax.reference_count());
        for ref_frame in 0..syntax.reference_count() {
            let base = syntax.with_luma_subpel(
                ref_frame,
                (0, 0, 8, 8),
                (MotionVector::default(), InterpolationFilter::EightTap),
                |view: &ReferencePlaneView<'_, u16>, _| {
                    Level::padded(view.width(), view.height(), EDGE, |y, x| {
                        view.sample(y, x) as u16
                    })
                },
            )?;
            references.push(Pyramid::new(base));
        }
        let (luma, stride, width, height) = source.luma();
        let mv_step = syntax.mv_step().unsigned_abs();
        let source = Pyramid::new(Level::padded(width, height, 0, |y, x| luma[y * stride + x]));
        Ok(Some(Self {
            references,
            source,
            range: i32::from(preset.motion_range),
            subpel_depth: preset.subpel_depth.min(3 - mv_step.ilog2().min(3) as u8),
            mv_step,
            sad_lambda_q8: lambda.sse_per_bit_q16().isqrt(),
        }))
    }

    /// The vector, in `1/8` samples, that predicts the luma `block` (`left,
    /// top, width, height`) best from list reference `ref_frame`, starting
    /// from `seeds`; its rate is approximated as a difference from `pred`.
    pub(crate) fn search(
        &self,
        syntax: &IntraTileSyntax,
        ref_frame: usize,
        block: (usize, usize, usize, usize),
        seeds: &[MotionVector],
        pred: MotionVector,
    ) -> Result<MotionVector> {
        let reference = self
            .references
            .get(ref_frame)
            .ok_or(TileWriterError::Unsupported("reference index"))?;
        let (left, top, width, height) = block;
        let coarse = (width.min(height).ilog2().saturating_sub(2) as usize).min(MAX_LEVEL);
        let cost = |level: usize, (dx, dy): (i32, i32)| -> Option<u64> {
            let dims = BlockDims::new(width >> level, height >> level).ok()?;
            let (sx, sy) = ((left >> level) as i32, (top >> level) as i32);
            let source = &self.source.levels[level];
            let candidate = &reference.levels[level];
            let sad = kernels().sad(
                source.at(sx, sy, dims.width(), dims.height())?,
                source.stride,
                candidate.at(sx + dx, sy + dy, dims.width(), dims.height())?,
                candidate.stride,
                dims,
            );
            let mv = MotionVector {
                row: (dy * FULL) << level,
                col: (dx * FULL) << level,
            };
            Some(u64::from(sad) + self.rate(mv, pred))
        };
        let at_level = |mv: MotionVector, level: usize| {
            let scale = FULL << level;
            (mv.col.div_euclid(scale), mv.row.div_euclid(scale))
        };
        let consider = |best: &mut (u64, (i32, i32)), level: usize, mv: (i32, i32)| {
            if let Some(cost) = cost(level, mv)
                && cost < best.0
            {
                *best = (cost, mv);
            }
        };
        let mut best = (u64::MAX, (0, 0));
        for &seed in seeds.iter().chain(&[MotionVector::default()]) {
            consider(&mut best, coarse, at_level(seed, coarse));
        }
        let radius = (self.range >> coarse).clamp(1, 32);
        let (cx, cy) = best.1;
        for dy in -radius..=radius {
            for dx in -radius..=radius {
                consider(&mut best, coarse, (cx + dx, cy + dy));
            }
        }
        for level in (0..coarse).rev() {
            let (cx, cy) = (best.1.0 * 2, best.1.1 * 2);
            best = (u64::MAX, (cx, cy));
            for dy in -2..=2 {
                for dx in -2..=2 {
                    consider(&mut best, level, (cx + dx, cy + dy));
                }
            }
        }
        if coarse > 0 {
            for &seed in seeds {
                consider(&mut best, 0, at_level(seed, 0));
            }
        }
        let mut mv = MotionVector {
            row: best.1.1 * FULL,
            col: best.1.0 * FULL,
        };
        if self.subpel_depth == 0 {
            return Ok(mv);
        }
        let mut best_cost = self.subpel_cost(syntax, ref_frame, block, mv, pred)?;
        let steps = [FULL / 2, FULL / 4, FULL / 8];
        for &step in &steps[..usize::from(self.subpel_depth).min(steps.len())] {
            let center = mv;
            for (dr, dc) in NEIGHBOURS {
                let candidate = MotionVector {
                    row: center.row + dr * step,
                    col: center.col + dc * step,
                };
                let cost = self.subpel_cost(syntax, ref_frame, block, candidate, pred)?;
                if cost < best_cost {
                    (best_cost, mv) = (cost, candidate);
                }
            }
        }
        Ok(mv)
    }

    /// SATD of the decoder's prediction of the block with `mv`, plus rate.
    fn subpel_cost(
        &self,
        syntax: &IntraTileSyntax,
        ref_frame: usize,
        block: (usize, usize, usize, usize),
        mv: MotionVector,
        pred: MotionVector,
    ) -> Result<u64> {
        let predicted = predict(
            syntax,
            ref_frame,
            block,
            (mv, InterpolationFilter::EightTap),
        )?;
        Ok(self.satd(block, &predicted)? + self.rate(mv, pred))
    }

    /// SATD of the luma prediction of the block with `filter` from the one or
    /// two `(reference, vector)` of `lists`; two lists take the rounded
    /// average, which approximates the decoder's `COMPOUND_AVERAGE`, which
    /// rounds once from higher precision.
    pub(crate) fn prediction_satd(
        &self,
        syntax: &IntraTileSyntax,
        lists: &[(usize, MotionVector)],
        filter: InterpolationFilter,
        block: (usize, usize, usize, usize),
    ) -> Result<u64> {
        match *lists {
            [(ref0, mv0), (ref1, mv1)] => {
                let p0 = predict(syntax, ref0, block, (mv0, filter))?;
                let p1 = predict(syntax, ref1, block, (mv1, filter))?;
                self.average_satd((&p0, &p1), block)
            }
            [(ref0, mv0)] => self.satd(block, &predict(syntax, ref0, block, (mv0, filter))?),
            _ => Err(TileWriterError::Unsupported("one or two prediction lists")),
        }
    }

    /// The row-major source luma of `block` and the decoder's luma
    /// predictions of `block` from the two `(reference, vector)` `lists`
    /// with `filter`.
    pub(crate) fn compound_inputs(
        &self,
        syntax: &IntraTileSyntax,
        lists: [(usize, MotionVector); 2],
        filter: InterpolationFilter,
        block: (usize, usize, usize, usize),
    ) -> Result<[Vec<u16>; 3]> {
        let (left, top, width, height) = block;
        let source = &self.source.levels[0];
        let samples = source.at(left as i32, top as i32, width, height).ok_or(
            TileWriterError::Unsupported("source block outside the plane"),
        )?;
        let mut rows = Vec::with_capacity(width * height);
        for row in samples.chunks(source.stride).take(height) {
            rows.extend_from_slice(&row[..width]);
        }
        let [(ref0, mv0), (ref1, mv1)] = lists;
        Ok([
            rows,
            predict(syntax, ref0, block, (mv0, filter))?,
            predict(syntax, ref1, block, (mv1, filter))?,
        ])
    }

    /// The decoder's row-major luma prediction of `block` from list
    /// reference `ref_frame` with `(mv, filter)`.
    pub(crate) fn luma_prediction(
        syntax: &IntraTileSyntax,
        ref_frame: usize,
        block: (usize, usize, usize, usize),
        motion: (MotionVector, InterpolationFilter),
    ) -> Result<Vec<u16>> {
        predict(syntax, ref_frame, block, motion)
    }

    /// SATD of the rounded average of the row-major predictions `p0` and
    /// `p1` of `block`, as [`Self::prediction_satd`] of two lists.
    pub(crate) fn average_satd(
        &self,
        (p0, p1): (&[u16], &[u16]),
        block: (usize, usize, usize, usize),
    ) -> Result<u64> {
        let average: Vec<u16> = p0
            .iter()
            .zip(p1)
            .map(|(&a, &b)| ((u32::from(a) + u32::from(b) + 1) >> 1) as u16)
            .collect();
        self.satd(block, &average)
    }

    /// The cost of `bits` in SAD units.
    pub(crate) fn bits_cost(&self, bits: Bits) -> u64 {
        (bits.0 * self.sad_lambda_q8) >> 16
    }

    /// SATD between the source luma of `block` and the row-major `predicted`.
    pub(crate) fn satd(
        &self,
        (left, top, width, height): (usize, usize, usize, usize),
        predicted: &[u16],
    ) -> Result<u64> {
        let source = &self.source.levels[0];
        let samples = source.at(left as i32, top as i32, width, height).ok_or(
            TileWriterError::Unsupported("source block outside the plane"),
        )?;
        let dims = BlockDims::new(width, height).map_err(TileWriterError::Kernel)?;
        Ok(kernels().satd(samples, source.stride, predicted, width, dims))
    }

    /// The approximate rate of coding `mv` against `pred`, in SAD units: two
    /// bits per binary order of the joint difference in `NEWMV` steps.
    pub(crate) fn rate(&self, mv: MotionVector, pred: MotionVector) -> u64 {
        let shell = ((mv.row - pred.row).unsigned_abs() + (mv.col - pred.col).unsigned_abs())
            / self.mv_step;
        self.bits_cost(Bits(2 * u64::from(log2_q8(shell + 1)) + 256))
    }
}

/// The decoder's row-major luma prediction of `block` from list reference
/// `ref_frame` with `(mv, filter)`.
fn predict(
    syntax: &IntraTileSyntax,
    ref_frame: usize,
    block: (usize, usize, usize, usize),
    motion: (MotionVector, InterpolationFilter),
) -> Result<Vec<u16>> {
    let width = block.2;
    let mut predicted = vec![0u16; width * block.3];
    syntax.with_luma_subpel(ref_frame, block, motion, |view, params| {
        subpel_predict_block_strided_into(view, params, &mut predicted, width)
    })??;
    Ok(predicted)
}
