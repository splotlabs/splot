// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Lookahead analysis of source frames before they are coded.
//!
//! Each frame's luma is averaged down by 2 per side (by 4 above 1080p area),
//! scaled to 8 bits, and split into 8x8 blocks. A block's intra cost is the
//! SATD of the best of a DC, a vertical and a horizontal prediction from the
//! downscaled source samples above and left of it. Its inter cost against
//! another frame is the SATD at the best full-sample vector of a SAD search:
//! every vector within [`EXHAUSTIVE`] samples, the left block's vector, then
//! a logarithmic search around the best. Costs are integers, so the analysis
//! is deterministic.

use std::collections::{BTreeMap, VecDeque};

use crate::config::Config;
use crate::frame::SourceFrame;
use crate::kernels::{BlockDims, kernels};

/// Side of an analysis block, in downscaled samples.
pub(crate) const BLOCK: usize = 8;
/// Replicated samples around each downscaled plane; it bounds the vectors.
const PAD: usize = 32;
/// Radius of the exhaustive part of the search, in downscaled samples.
const EXHAUSTIVE: i32 = 4;
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
    plane: Vec<u16>,
    intra: Vec<u32>,
}

/// Downscaled frames and their block costs, by display index.
pub(crate) struct Lookahead {
    shift: u32,
    bit_shift: u32,
    cols: usize,
    rows: usize,
    stride: usize,
    dims: BlockDims,
    frames: VecDeque<Analysis>,
    motion: BTreeMap<(u64, u64), Vec<BlockMotion>>,
    /// The scene-cut ratio of the last frame pushed, in percent.
    last_ratio: u64,
}

impl Lookahead {
    /// An empty lookahead for frames of `config`.
    pub(crate) fn new(config: &Config) -> Self {
        let shift = if u64::from(config.width) * u64::from(config.height) > QUARTER_SCALE_ABOVE {
            2
        } else {
            1
        };
        let side = |luma: u32| (luma as usize).div_ceil(1 << shift).div_ceil(BLOCK);
        let (cols, rows) = (side(config.width), side(config.height));
        Self {
            shift,
            bit_shift: u32::from(config.bit_depth.bits() - 8),
            cols,
            rows,
            stride: cols * BLOCK + 2 * PAD,
            dims: BlockDims::SQUARE_8,
            frames: VecDeque::new(),
            motion: BTreeMap::new(),
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
        let plane = self.downscale(frame);
        let intra = self.intra_costs(&plane);
        self.frames.push_back(Analysis {
            display_index,
            plane,
            intra,
        });
        let Some(previous) = display_index.checked_sub(1) else {
            return false;
        };
        if self.motion(display_index, previous).is_none() {
            return false;
        }
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
    /// `reference`; `None` when either frame is not held.
    pub(crate) fn motion(&mut self, current: u64, reference: u64) -> Option<&[BlockMotion]> {
        if !self.motion.contains_key(&(current, reference)) {
            let blocks =
                self.search_frame(&self.frame(current)?.plane, &self.frame(reference)?.plane);
            self.motion.insert((current, reference), blocks);
        }
        self.motion.get(&(current, reference)).map(Vec::as_slice)
    }

    /// Frame `display_index`; the held frames have consecutive indices.
    fn frame(&self, display_index: u64) -> Option<&Analysis> {
        let first = self.frames.front()?.display_index;
        let index = usize::try_from(display_index.checked_sub(first)?).ok()?;
        self.frames
            .get(index)
            .filter(|frame| frame.display_index == display_index)
    }

    /// The padded, downscaled, 8-bit luma of `frame`.
    fn downscale(&self, frame: &SourceFrame) -> Vec<u16> {
        let size = frame.plane_sizes()[0];
        let luma = &frame.planes()[0];
        let scale = 1usize << self.shift;
        let (width, height) = (size.width().div_ceil(scale), size.height().div_ceil(scale));
        let mut small = Vec::with_capacity(width * height);
        for y in 0..height {
            for x in 0..width {
                let mut sum = 0u32;
                for dy in 0..scale {
                    let row = (y * scale + dy).min(size.height() - 1) * size.width();
                    for dx in 0..scale {
                        sum += u32::from(luma[row + (x * scale + dx).min(size.width() - 1)]);
                    }
                }
                small.push((sum >> (2 * self.shift + self.bit_shift)) as u16);
            }
        }
        let padded_rows = self.rows * BLOCK + 2 * PAD;
        let mut plane = Vec::with_capacity(self.stride * padded_rows);
        for y in 0..padded_rows {
            let row = &small[y.saturating_sub(PAD).min(height - 1) * width..][..width];
            plane.extend((0..self.stride).map(|x| row[x.saturating_sub(PAD).min(width - 1)]));
        }
        plane
    }

    /// The offset of block `(col, row)` in a padded plane.
    fn block_at(&self, col: usize, row: usize) -> usize {
        (PAD + row * BLOCK) * self.stride + PAD + col * BLOCK
    }

    fn intra_costs(&self, plane: &[u16]) -> Vec<u32> {
        let mut costs = Vec::with_capacity(self.cols * self.rows);
        let mut prediction = [0u16; BLOCK * BLOCK];
        for row in 0..self.rows {
            for col in 0..self.cols {
                let at = self.block_at(col, row);
                let above = &plane[at - self.stride..][..BLOCK];
                let left: Vec<u16> = (0..BLOCK)
                    .map(|y| plane[at + y * self.stride - 1])
                    .collect();
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
                        kernels().satd(&plane[at..], self.stride, &prediction, BLOCK, self.dims);
                    best = best.min(satd);
                }
                costs.push(u32::try_from(best).unwrap_or(u32::MAX));
            }
        }
        costs
    }

    fn search_frame(&self, current: &[u16], reference: &[u16]) -> Vec<BlockMotion> {
        let mut blocks: Vec<BlockMotion> = Vec::with_capacity(self.cols * self.rows);
        for row in 0..self.rows {
            for col in 0..self.cols {
                let seed = if col > 0 {
                    blocks[blocks.len() - 1].mv
                } else {
                    (0, 0)
                };
                blocks.push(self.search_block(current, reference, (col, row), seed));
            }
        }
        blocks
    }

    /// The search of one block; its logarithmic part halves the step from 8
    /// down to 1, moving to the best of the four neighbours while one improves.
    fn search_block(
        &self,
        current: &[u16],
        reference: &[u16],
        (col, row): (usize, usize),
        seed: (i32, i32),
    ) -> BlockMotion {
        let dims = self.dims;
        let source = &current[self.block_at(col, row)..];
        let start = |position: usize, mv: i32, extent: usize| {
            usize::try_from(position as i32 + mv)
                .ok()
                .filter(|&start| start + BLOCK <= extent)
        };
        let padded_rows = self.rows * BLOCK + 2 * PAD;
        let candidate = |(x, y): (i32, i32)| {
            let x = start(PAD + col * BLOCK, x, self.stride)?;
            let y = start(PAD + row * BLOCK, y, padded_rows)?;
            Some(&reference[y * self.stride + x..])
        };
        let sad = |mv: (i32, i32)| {
            candidate(mv).map(|block| kernels().sad(source, self.stride, block, self.stride, dims))
        };
        let mut best = (u32::MAX, (0, 0));
        let near = (-EXHAUSTIVE..=EXHAUSTIVE)
            .flat_map(|y| (-EXHAUSTIVE..=EXHAUSTIVE).map(move |x| (x, y)));
        for mv in near.chain([seed]) {
            if let Some(cost) = sad(mv)
                && cost < best.0
            {
                best = (cost, mv);
            }
        }
        let mut step = 8;
        while step > 0 {
            for _ in 0..PAD {
                let center = best.1;
                for (dx, dy) in [(step, 0), (-step, 0), (0, step), (0, -step)] {
                    let mv = (center.0 + dx, center.1 + dy);
                    if let Some(cost) = sad(mv)
                        && cost < best.0
                    {
                        best = (cost, mv);
                    }
                }
                if best.1 == center {
                    break;
                }
            }
            step /= 2;
        }
        let cost = candidate(best.1).map_or(u64::MAX, |block| {
            kernels().satd(source, self.stride, block, self.stride, dims)
        });
        BlockMotion {
            cost: u32::try_from(cost).unwrap_or(u32::MAX),
            mv: best.1,
        }
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
        let mut lookahead = Lookahead::new(&config);
        assert_eq!(lookahead.grid(), (6, 4));
        assert!(!lookahead.push(&frame(&config, 0, 0, 0), 0));
        assert!(
            !lookahead.push(&frame(&config, 1, 4, 0), 1),
            "a pan is no cut"
        );
        let motion = lookahead.motion(1, 0).unwrap();
        assert!(
            motion.iter().filter(|block| block.mv == (-2, 0)).count() >= 18,
            "{motion:?}"
        );
        assert!(
            lookahead.push(&frame(&config, 2, 4, 1), 2),
            "a new texture is a cut"
        );
        lookahead.retain_from(1);
        assert!(lookahead.intra(0).is_none() && lookahead.motion(1, 0).is_none());
        assert!(lookahead.motion(2, 1).is_some());
    }
}
