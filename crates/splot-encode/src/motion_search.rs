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

use std::cell::RefCell;

use splot_decode::encoder_syntax::{IntraTileSyntax, MotionVector};
use splot_recon::{InterpolationFilter, ReferencePlaneView, subpel_predict_block_strided_into};

use std::sync::Arc;

use crate::hme::FrameMotion;
use crate::intra_frame::PaddedSource;
use crate::kernels::motion::Motion;
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
    /// The lookahead's open-loop motion against each list reference, where
    /// it has some.
    open_loop: Vec<Option<Arc<FrameMotion>>>,
    /// [`PresetParams::me_seeds`] and [`PresetParams::compound_skip_sad`].
    me_seeds: u32,
    compound_skip_sad: u64,
}

/// The open-loop view of one leaf against one reference: the vectors of its
/// square, the parent and the four children, and the SAD and sample count of
/// its square.
struct OpenLoopLeaf {
    seeds: [Option<MotionVector>; 6],
    sad: u32,
    samples: u32,
}

thread_local! {
    /// The two predictions and their average that SATD queries reuse; tiles
    /// share one [`MotionSearch`] across threads.
    static PREDICTIONS: RefCell<[Vec<u16>; 3]> = RefCell::default();
}

/// Runs `f` on this thread's prediction buffers, or on new ones if they are
/// in use.
fn with_predictions<T>(f: impl FnOnce(&mut [Vec<u16>; 3]) -> T) -> T {
    PREDICTIONS.with(|cell| match cell.try_borrow_mut() {
        Ok(mut buffers) => f(&mut buffers),
        Err(_) => f(&mut Default::default()),
    })
}

impl MotionSearch {
    /// The pyramids of every reference of the frame `syntax` codes and of
    /// `source`; `None` in an intra frame.
    pub(crate) fn new(
        syntax: &IntraTileSyntax,
        source: &PaddedSource,
        preset: &PresetParams,
        lambda: Lambda,
        open_loop: Vec<Option<Arc<FrameMotion>>>,
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
            open_loop,
            me_seeds: u32::from(preset.me_seeds),
            compound_skip_sad: u64::from(preset.compound_skip_sad),
        }))
    }

    /// The open-loop view of luma `block` against list reference
    /// `ref_frame`; `None` without lookahead motion for the pair.
    fn open_loop_leaf(
        &self,
        ref_frame: usize,
        block: (usize, usize, usize, usize),
    ) -> Option<OpenLoopLeaf> {
        leaf_view(self.open_loop.get(ref_frame)?.as_ref()?, block)
    }

    /// Whether `block` skips the compound search: one reference with
    /// lookahead motion predicts its square with a mean open-loop SAD per
    /// sample of at most [`PresetParams::compound_skip_sad`] / 16.
    pub(crate) fn skips_compound(&self, block: (usize, usize, usize, usize)) -> bool {
        predicts_alone(&self.open_loop, self.compound_skip_sad, block)
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
        let open_loop = (self.me_seeds > 0)
            .then(|| self.open_loop_leaf(ref_frame, block))
            .flatten();
        let open_seeds = open_loop
            .iter()
            .flat_map(|leaf| leaf.seeds.iter().flatten());
        let zero = [MotionVector::default()];
        let mut best = (u64::MAX, (0, 0));
        for &seed in seeds.iter().chain(&zero).chain(open_seeds.clone()) {
            consider(&mut best, coarse, at_level(seed, coarse));
        }
        let confident = open_loop
            .as_ref()
            .is_some_and(|leaf| leaf.sad <= self.me_seeds.saturating_mul(leaf.samples));
        let radius = if confident {
            1
        } else {
            (self.range >> coarse).clamp(1, 32)
        };
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
            for &seed in seeds.iter().chain(open_seeds) {
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
        let motion = (mv, InterpolationFilter::EightTap);
        let satd = with_predictions(|[predicted, ..]| {
            predict_into(syntax, ref_frame, block, motion, predicted)?;
            self.satd(block, predicted)
        })?;
        Ok(satd + self.rate(mv, pred))
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
        with_predictions(|[p0, p1, average]| match *lists {
            [(ref0, mv0), (ref1, mv1)] => {
                predict_into(syntax, ref0, block, (mv0, filter), p0)?;
                predict_into(syntax, ref1, block, (mv1, filter), p1)?;
                average.clear();
                average.extend(
                    p0.iter()
                        .zip(p1.iter())
                        .map(|(&a, &b)| ((u32::from(a) + u32::from(b) + 1) >> 1) as u16),
                );
                self.satd(block, average)
            }
            [(ref0, mv0)] => {
                predict_into(syntax, ref0, block, (mv0, filter), p0)?;
                self.satd(block, p0)
            }
            _ => Err(TileWriterError::Unsupported("one or two prediction lists")),
        })
    }

    /// Fills `[source, p0, p1]` with the row-major source luma of `block`
    /// and the decoder's luma predictions of `block` from the two
    /// `(reference, vector)` `lists` with `filter`.
    pub(crate) fn compound_inputs(
        &self,
        syntax: &IntraTileSyntax,
        (lists, filter): ([(usize, MotionVector); 2], InterpolationFilter),
        block: (usize, usize, usize, usize),
        [rows, p0, p1]: [&mut Vec<u16>; 3],
    ) -> Result<()> {
        let (left, top, width, height) = block;
        let source = &self.source.levels[0];
        let samples = source.at(left as i32, top as i32, width, height).ok_or(
            TileWriterError::Unsupported("source block outside the plane"),
        )?;
        rows.clear();
        for row in samples.chunks(source.stride).take(height) {
            rows.extend_from_slice(&row[..width]);
        }
        let [(ref0, mv0), (ref1, mv1)] = lists;
        predict_into(syntax, ref0, block, (mv0, filter), p0)?;
        predict_into(syntax, ref1, block, (mv1, filter), p1)
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

/// The open-loop view of luma `block` in `field`: the square is the
/// largest one of 8 to 64 samples at the block's corner; `None` when its
/// cell is outside the field or has no vector.
fn leaf_view(
    field: &FrameMotion,
    (left, top, width, height): (usize, usize, usize, usize),
) -> Option<OpenLoopLeaf> {
    let side = width.min(height).clamp(8, 64);
    let size = side.ilog2() as usize - 3;
    let at = |size: usize, (col, row): (usize, usize)| {
        let (stride, blocks) = field.blocks(size);
        (col < stride)
            .then(|| blocks.get(row * stride + col))
            .flatten()
            .filter(|found| found.sad != u32::MAX)
    };
    let (col, row) = (left / side, top / side);
    let own = at(size, (col, row))?;
    let vector = |found: &Motion| MotionVector {
        row: i32::from(found.mv[1]) * FULL,
        col: i32::from(found.mv[0]) * FULL,
    };
    let parent = (size < 3)
        .then(|| at(size + 1, (col / 2, row / 2)))
        .flatten();
    let child = |i: usize| {
        (size > 0)
            .then(|| at(size - 1, (2 * col + i % 2, 2 * row + i / 2)))
            .flatten()
    };
    let seeds = [Some(own), parent, child(0), child(1), child(2), child(3)];
    Some(OpenLoopLeaf {
        seeds: seeds.map(|found| found.map(vector)),
        sad: own.sad,
        samples: (side * side) as u32,
    })
}

/// Whether one of `fields` predicts the square of `block` with a mean
/// open-loop SAD per sample of at most `threshold` / 16; `threshold` 0 never.
fn predicts_alone(
    fields: &[Option<Arc<FrameMotion>>],
    threshold: u64,
    block: (usize, usize, usize, usize),
) -> bool {
    threshold > 0
        && fields
            .iter()
            .filter_map(|field| leaf_view(field.as_ref()?, block))
            .any(|leaf| u64::from(leaf.sad) * 16 <= threshold * u64::from(leaf.samples))
}

/// The decoder's row-major luma prediction of `block` from list reference
/// `ref_frame` with `(mv, filter)` into `predicted`, reusing its allocation.
fn predict_into(
    syntax: &IntraTileSyntax,
    ref_frame: usize,
    block: (usize, usize, usize, usize),
    motion: (MotionVector, InterpolationFilter),
    predicted: &mut Vec<u16>,
) -> Result<()> {
    let width = block.2;
    predicted.clear();
    predicted.resize(width * block.3, 0);
    syntax.with_luma_subpel(ref_frame, block, motion, |view, params| {
        subpel_predict_block_strided_into(view, params, predicted, width)
    })??;
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    /// A field of 2x2 superblocks whose cells tell their size and position:
    /// vector `[col + 10 * size, row]`, SAD `1000 * size + 100 * row + col`;
    /// the 8x8 cell `(15, 15)` has no vector.
    fn field() -> FrameMotion {
        FrameMotion::from_fn((2, 2), |size, col, row| {
            if (size, col, row) == (0, 15, 15) {
                return Motion::NONE;
            }
            Motion {
                sad: (1000 * size + 100 * row + col) as u32,
                mv: [(col + 10 * size) as i16, row as i16],
            }
        })
    }

    fn cell(size: usize, col: usize, row: usize) -> MotionVector {
        MotionVector {
            row: row as i32 * FULL,
            col: (col + 10 * size) as i32 * FULL,
        }
    }

    #[test]
    fn leaves_read_their_square_parent_and_children() {
        let field = field();
        let leaf = leaf_view(&field, (32, 48, 16, 32)).unwrap();
        assert_eq!((leaf.sad, leaf.samples), (1302, 256));
        let children = [
            Some(cell(0, 4, 6)),
            Some(cell(0, 5, 6)),
            Some(cell(0, 4, 7)),
            Some(cell(0, 5, 7)),
        ];
        let expected = [
            Some(cell(1, 2, 3)),
            Some(cell(2, 1, 1)),
            children[0],
            children[1],
            children[2],
            children[3],
        ];
        assert_eq!(leaf.seeds, expected);
        let thin = leaf_view(&field, (8, 0, 32, 8)).unwrap();
        assert_eq!(
            thin.seeds,
            [
                Some(cell(0, 1, 0)),
                Some(cell(1, 0, 0)),
                None,
                None,
                None,
                None
            ]
        );
        for block in [(64, 64, 64, 64), (64, 64, 128, 128)] {
            let large = leaf_view(&field, block).unwrap();
            assert_eq!((large.sad, large.samples), (3101, 4096));
            let quarter = [
                Some(cell(2, 2, 2)),
                Some(cell(2, 3, 2)),
                Some(cell(2, 2, 3)),
                Some(cell(2, 3, 3)),
            ];
            assert_eq!(large.seeds[..2], [Some(cell(3, 1, 1)), None]);
            assert_eq!(large.seeds[2..], quarter);
        }
        let edge = (120, 120, 8, 8);
        assert!(leaf_view(&field, edge).is_none(), "a cell without a vector");
        assert!(
            leaf_view(&field, (128, 0, 64, 64)).is_none(),
            "right of the field"
        );
        assert!(
            leaf_view(&field, (0, 128, 8, 8)).is_none(),
            "below the field"
        );
    }

    /// The skip takes the best reference and compares its SAD per sample
    /// with the threshold in 1/16, inclusive.
    #[test]
    fn compound_skip_needs_one_good_reference() {
        let flat = |sad: u32| {
            Some(Arc::new(FrameMotion::from_fn((1, 1), move |size, _, _| {
                Motion {
                    sad: sad << (2 * size),
                    mv: [1, -1],
                }
            })))
        };
        let block = (16, 16, 16, 8);
        let fields = [None, flat(640), flat(160)];
        assert!(
            predicts_alone(&fields, 40, block),
            "160 per 64 samples is 40/16"
        );
        assert!(!predicts_alone(&fields, 39, block));
        assert!(
            !predicts_alone(&fields[..2], 40, block),
            "only the worse reference"
        );
        assert!(!predicts_alone(&fields, 0, block), "off");
        assert!(
            !predicts_alone(&[None, None], 255, block),
            "no lookahead motion"
        );
    }
}
