// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Lookahead analysis of source frames before they are coded.
//!
//! Each frame's luma becomes an [`hme::Pyramid`]; the analysis works on its
//! half level (quarter level above 1080p area) in 8x8 blocks. A block's intra
//! cost is the SATD of the best of a DC, a vertical and a horizontal
//! prediction from the samples above and left of it. Its inter cost against
//! another frame comes from the open-loop motion estimation of the pair: the
//! full-resolution vectors of the block's luma area, of its four quadrants
//! and the left block's choice, scaled down to the analysis level, seed a
//! SAD search one sample around each, and the SATD of the best position is
//! the cost. Costs are integers, so the analysis is deterministic.

use std::collections::{BTreeMap, VecDeque};

use splot_parallel::WorkerPool;

use crate::config::Config;
use crate::frame::SourceFrame;
use crate::hme::{self, FrameMotion, Plane, Pyramid};
use crate::kernels::motion::Motion;
use crate::kernels::{BlockDims, kernels};
use crate::preset::PresetParams;

/// The source frames the encoder still holds, by display index.
pub(crate) type Sources<'a> = &'a dyn Fn(u64) -> Option<&'a SourceFrame>;

/// Side of an analysis block, in downscaled samples.
pub(crate) const BLOCK: usize = 8;
/// A frame is a scene cut when its blocks' best costs against the previous
/// frame sum to at least this percentage of their intra costs (its ratio) ...
const SCENE_CUT_PERCENT: u64 = 70;
/// ... and its ratio is at least this many points above the previous frame's,
/// so that noise, which predicts badly in every frame, makes no cuts.
const SCENE_CUT_JUMP_PERCENT: u64 = 20;
/// Mean intra SATD per block below which a frame is too flat to be a cut.
const MIN_CUT_INTRA_PER_BLOCK: u64 = 256;
/// Downscaling by 4 starts above this many luma samples.
const QUARTER_SCALE_ABOVE: u64 = 1920 * 1088;

/// The best inter prediction of one block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct BlockMotion {
    /// SATD of the prediction.
    pub(crate) cost: u32,
    /// Full-sample vector `(x, y)` in downscaled samples.
    pub(crate) mv: (i32, i32),
}

/// One analysed frame.
struct Analysis {
    display_index: u64,
    pyramid: Pyramid,
    intra: Vec<u32>,
}

/// Analysed frames and the open-loop motion of frame pairs, by display index.
pub(crate) struct Lookahead {
    shift: u32,
    cols: usize,
    rows: usize,
    ranges: hme::Ranges,
    pool: Option<WorkerPool>,
    frames: VecDeque<Analysis>,
    motion: BTreeMap<(u64, u64), Vec<BlockMotion>>,
    /// The full level of the last frame pushed, for its scene-cut pair.
    last_full: Option<(u64, Plane)>,
    /// The scene-cut ratio of the last frame pushed, in percent.
    last_ratio: u64,
}

impl Lookahead {
    /// An empty lookahead for frames of `config`, searching with the ranges
    /// of `preset` on `pool` when there is one.
    pub(crate) fn new(config: &Config, preset: &PresetParams, pool: Option<WorkerPool>) -> Self {
        let shift = if u64::from(config.width) * u64::from(config.height) > QUARTER_SCALE_ABOVE {
            2
        } else {
            1
        };
        let side = |luma: u32| (luma as usize).div_ceil(BLOCK << shift);
        Self {
            shift,
            cols: side(config.width),
            rows: side(config.height),
            ranges: hme::Ranges::new(preset),
            pool,
            frames: VecDeque::new(),
            motion: BTreeMap::new(),
            last_full: None,
            last_ratio: 0,
        }
    }

    /// Blocks per row and per column.
    pub(crate) fn grid(&self) -> (usize, usize) {
        (self.cols, self.rows)
    }

    /// The side of a block in full-resolution luma samples.
    pub(crate) const fn block_luma(&self) -> usize {
        BLOCK << self.shift
    }

    /// Analyses `frame`, the frame after the last one pushed. Returns whether
    /// it is a scene cut against that frame.
    pub(crate) fn push(&mut self, frame: &SourceFrame, display_index: u64) -> bool {
        let full = hme::full_level(frame);
        let pyramid = Pyramid::new(&full);
        let intra = self.intra_costs(pyramid.level(self.shift as usize));
        self.frames.push_back(Analysis {
            display_index,
            pyramid,
            intra,
        });
        let previous_full = self.last_full.replace((display_index, full));
        let Some(previous) = display_index.checked_sub(1) else {
            return false;
        };
        let blocks = match (&previous_full, &self.last_full) {
            (Some((index, previous_full)), Some((_, full))) if *index == previous => {
                self.searched((display_index, previous), (full, previous_full))
            }
            _ => None,
        };
        let Some(blocks) = blocks else {
            return false;
        };
        self.motion.insert((display_index, previous), blocks);
        let (Some(intra), Some(motion)) = (
            self.intra(display_index),
            self.motion.get(&(display_index, previous)),
        ) else {
            return false;
        };
        let intra_sum: u64 = intra.iter().map(|&cost| u64::from(cost)).sum();
        let best_sum: u64 = intra
            .iter()
            .zip(motion)
            .map(|(&intra, inter)| u64::from(intra.min(inter.cost)))
            .sum();
        let ratio = best_sum * 100 / intra_sum.max(1);
        let cut = intra_sum >= MIN_CUT_INTRA_PER_BLOCK * intra.len() as u64
            && ratio >= SCENE_CUT_PERCENT
            && ratio >= self.last_ratio + SCENE_CUT_JUMP_PERCENT;
        self.last_ratio = ratio;
        cut
    }

    /// Drops the frames before `display_index`.
    pub(crate) fn retain_from(&mut self, display_index: u64) {
        while self
            .frames
            .front()
            .is_some_and(|frame| frame.display_index < display_index)
        {
            self.frames.pop_front();
        }
        self.motion
            .retain(|&(current, reference), _| current.min(reference) >= display_index);
    }

    /// The intra cost of each block of frame `display_index`, row by row.
    pub(crate) fn intra(&self, display_index: u64) -> Option<&[u32]> {
        self.frame(display_index)
            .map(|frame| frame.intra.as_slice())
    }

    /// The best inter prediction of each block of frame `current` from frame
    /// `reference`; `None` when either frame is not held. The open-loop
    /// motion of the pair is searched once and its blocks kept; the search
    /// reads the full levels of frames other than the last one pushed from
    /// their `sources`.
    pub(crate) fn motion(
        &mut self,
        current: u64,
        reference: u64,
        sources: Sources<'_>,
    ) -> Option<&[BlockMotion]> {
        if !self.motion.contains_key(&(current, reference)) {
            let built = [current, reference].map(|display| match &self.last_full {
                Some((last, _)) if *last == display => None,
                _ => sources(display).map(hme::full_level),
            });
            let full = |i: usize, display: u64| {
                built[i].as_ref().or_else(|| {
                    let (last, plane) = self.last_full.as_ref()?;
                    (*last == display).then_some(plane)
                })
            };
            let fulls = (full(0, current)?, full(1, reference)?);
            let blocks = self.searched((current, reference), fulls)?;
            self.motion.insert((current, reference), blocks);
        }
        self.motion.get(&(current, reference)).map(Vec::as_slice)
    }

    /// The blocks of the open-loop motion of `current` against `reference`,
    /// whose full levels are `fulls`.
    fn searched(
        &self,
        (current, reference): (u64, u64),
        fulls: (&Plane, &Plane),
    ) -> Option<Vec<BlockMotion>> {
        let motion = self.full_motion((current, reference), fulls)?;
        let (now, then) = (self.frame(current)?, self.frame(reference)?);
        let level = self.shift as usize;
        let planes = (now.pyramid.level(level), then.pyramid.level(level));
        Some(self.block_motion(planes, &motion))
    }

    /// The open-loop motion of `current` against `reference`, whose full
    /// levels are `fulls`, searched without keeping it; `None` when either
    /// frame is not held.
    pub(crate) fn full_motion(
        &self,
        (current, reference): (u64, u64),
        fulls: (&Plane, &Plane),
    ) -> Option<FrameMotion> {
        let (now, then) = (self.frame(current)?, self.frame(reference)?);
        Some(hme::search(
            (fulls.0, &now.pyramid),
            (fulls.1, &then.pyramid),
            current.abs_diff(reference),
            self.ranges,
            self.pool.as_ref(),
        ))
    }

    /// Frame `display_index`; the held frames have consecutive indices.
    fn frame(&self, display_index: u64) -> Option<&Analysis> {
        let first = self.frames.front()?.display_index;
        let index = usize::try_from(display_index.checked_sub(first)?).ok()?;
        self.frames
            .get(index)
            .filter(|frame| frame.display_index == display_index)
    }

    fn intra_costs(&self, plane: &Plane) -> Vec<u32> {
        let mut costs = Vec::with_capacity(self.cols * self.rows);
        let mut prediction = [0u16; BLOCK * BLOCK];
        for row in 0..self.rows {
            for col in 0..self.cols {
                let (x, y) = ((col * BLOCK) as i32, (row * BLOCK) as i32);
                let Some(source) = plane.block8(x, y) else {
                    costs.push(0);
                    continue;
                };
                let above: [u16; BLOCK] =
                    core::array::from_fn(|i| u16::from(plane.sample(x + i as i32, y - 1)));
                let left: [u16; BLOCK] =
                    core::array::from_fn(|i| u16::from(plane.sample(x - 1, y + i as i32)));
                let dc = (above
                    .iter()
                    .chain(&left)
                    .map(|&s| u32::from(s))
                    .sum::<u32>()
                    + BLOCK as u32)
                    / (2 * BLOCK as u32);
                let mut best = u64::MAX;
                for mode in 0..3 {
                    for (index, sample) in prediction.iter_mut().enumerate() {
                        let (y, x) = (index / BLOCK, index % BLOCK);
                        *sample = match mode {
                            0 => dc as u16,
                            1 => above[x],
                            _ => left[y],
                        };
                    }
                    let satd =
                        kernels().satd(&source, BLOCK, &prediction, BLOCK, BlockDims::SQUARE_8);
                    best = best.min(satd);
                }
                costs.push(u32::try_from(best).unwrap_or(u32::MAX));
            }
        }
        costs
    }

    /// Each analysis block's prediction from the open-loop `motion`: the
    /// SATD at the lowest SAD within one sample of the vectors of its luma
    /// area, of its quadrants and of the left block, the first on a tie.
    fn block_motion(&self, planes: (&Plane, &Plane), motion: &FrameMotion) -> Vec<BlockMotion> {
        let size = self.shift as usize;
        let ((stride, blocks), (sub_stride, sub_blocks)) =
            (motion.blocks(size), motion.blocks(size - 1));
        let scaled = |found: &Motion| found.mv.map(|v| i32::from(v) >> self.shift);
        let mut out: Vec<BlockMotion> = Vec::with_capacity(self.cols * self.rows);
        for row in 0..self.rows {
            for col in 0..self.cols {
                let at = ((col * BLOCK) as i32, (row * BLOCK) as i32);
                let sub =
                    |i: usize| sub_blocks.get((2 * row + i / 2) * sub_stride + 2 * col + i % 2);
                let found = [blocks.get(row * stride + col)]
                    .into_iter()
                    .chain((0..4).map(sub));
                let left = (col > 0)
                    .then(|| out[out.len() - 1].mv)
                    .map(|(x, y)| [x, y]);
                let (mut centres, mut count) = ([[0; 2]; 6], 0);
                for centre in found.flatten().map(scaled).chain(left) {
                    if !centres[..count].contains(&centre) {
                        centres[count] = centre;
                        count += 1;
                    }
                }
                let mut chosen: Option<(u32, [i32; 2])> = None;
                for &centre in &centres[..count] {
                    if let Some(near) = hme::best_near(planes, at, centre)
                        && chosen.is_none_or(|(sad, _)| near.0 < sad)
                    {
                        chosen = Some(near);
                    }
                }
                let cost = chosen.and_then(|(_, mv)| {
                    let source = planes.0.block8(at.0, at.1)?;
                    let prediction = planes.1.block8(at.0 + mv[0], at.1 + mv[1])?;
                    let satd =
                        kernels().satd(&source, BLOCK, &prediction, BLOCK, BlockDims::SQUARE_8);
                    Some((u32::try_from(satd).unwrap_or(u32::MAX), mv))
                });
                out.push(cost.map_or(
                    BlockMotion {
                        cost: u32::MAX,
                        mv: (0, 0),
                    },
                    |(cost, mv)| BlockMotion {
                        cost,
                        mv: (mv[0], mv[1]),
                    },
                ));
            }
        }
        out
    }
}

#[cfg(test)]
pub(crate) mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    /// A textured frame shifted right by `shift` luma samples; `scene` changes
    /// the texture.
    pub(crate) fn frame(config: &Config, pts: u64, shift: usize, scene: u32) -> SourceFrame {
        let sizes = config.plane_sizes().unwrap();
        let planes = core::array::from_fn(|plane| {
            let size = sizes[plane];
            (0..size.height())
                .flat_map(|y| (0..size.width()).map(move |x| (x, y)))
                .map(|(x, y)| {
                    let (x, y) = ((x + 1000 - shift) as u32, y as u32);
                    let hash =
                        (x / 3 * 7 + y / 3 * 13).wrapping_mul(2_654_435_761 + 6 * scene) >> 28;
                    let wave = (x * x / (9 + 4 * scene) + y * 5) % 97;
                    ((wave + hash) / (plane as u32 + 1) + 20) as u16
                })
                .collect()
        });
        SourceFrame::new(config, pts, planes).unwrap()
    }

    #[test]
    fn finds_motion_and_scene_cuts() {
        let config = Config::new(96, 64);
        let mut lookahead = Lookahead::new(&config, config.preset.params(), None);
        assert_eq!(lookahead.grid(), (6, 4));
        assert!(!lookahead.push(&frame(&config, 0, 0, 0), 0));
        assert!(
            !lookahead.push(&frame(&config, 1, 4, 0), 1),
            "a pan is no cut"
        );
        let motion = lookahead.motion(1, 0, &|_| None).unwrap();
        assert!(
            motion.iter().filter(|block| block.mv == (-2, 0)).count() >= 18,
            "{motion:?}"
        );
        assert!(
            lookahead.push(&frame(&config, 2, 4, 1), 2),
            "a new texture is a cut"
        );
        lookahead.retain_from(1);
        assert!(lookahead.intra(0).is_none() && lookahead.motion(1, 0, &|_| None).is_none());
        assert!(lookahead.motion(2, 1, &|_| None).is_some());
    }

    /// Above 1080p the analysis runs on the quarter level in blocks of 32
    /// luma samples; each quadrant of the frame moves differently, so a block
    /// takes its own vector, not a neighbour's.
    #[test]
    fn quarter_level_blocks_take_their_own_vectors() {
        let config = Config::new(2048, 1088);
        let mut lookahead = Lookahead::new(&config, config.preset.params(), None);
        assert_eq!((lookahead.grid(), lookahead.block_luma()), ((64, 34), 32));
        let quadrant = |x: usize, y: usize| {
            let dx = if x < 1024 { 8.0 } else { -16.0 };
            (dx, if y < 544 { 0.0 } else { 8.0 })
        };
        lookahead.push(&crate::hme::tests::smooth(&config, 0), 0);
        lookahead.push(&crate::hme::tests::moved(&config, quadrant), 1);
        let intra = lookahead.intra(1).unwrap().to_vec();
        let motion = lookahead.motion(1, 0, &|_| None).unwrap();
        let clear = |v: usize, edge: usize, split: usize| {
            v >= 128 && v + 32 + 128 <= edge && (v + 32 + 128 <= split || v >= split + 128)
        };
        let mut checked = 0;
        for (index, block) in motion.iter().enumerate() {
            let (x, y) = (index % 64 * 32, index / 64 * 32);
            if clear(x, 2048, 1024) && clear(y, 1088, 544) {
                let (dx, dy) = quadrant(x, y);
                assert_eq!(block.mv, (-dx as i32 / 4, -dy as i32 / 4), "block {index}");
                assert!(block.cost < intra[index], "block {index}");
                checked += 1;
            }
        }
        assert!(checked >= 200, "{checked}");
    }

    /// A frame smaller than one block is one analysis block.
    #[test]
    fn frames_smaller_than_a_block_are_one_block() {
        let config = Config::new(5, 3);
        let mut lookahead = Lookahead::new(&config, config.preset.params(), None);
        lookahead.push(&frame(&config, 0, 0, 0), 0);
        lookahead.push(&frame(&config, 1, 1, 0), 1);
        assert_eq!(lookahead.grid(), (1, 1));
        let motion = lookahead.motion(1, 0, &|_| None).unwrap();
        assert!(motion.len() == 1 && motion[0].cost < u32::MAX, "{motion:?}");
    }
}
