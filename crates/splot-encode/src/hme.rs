// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Open-loop hierarchical motion estimation on source frames.
//!
//! Each frame's luma, reduced to 8 bits, becomes a pyramid built once: full
//! resolution and two 2x2-mean levels (half and quarter per side, rounded down), each
//! covering whole 64x64 superblocks and padded by replicated samples. The
//! motion of a frame against another is searched per superblock: a full
//! search on the quarter level over the preset's `hme_range`, scaled by the
//! frames' distance; a refinement of [`REFINE`] samples on the half level;
//! then full searches at full resolution around the zero vector over half
//! the preset's `me_range` and around that centre over `me_range`, shrunk
//! when the half-level SAD and vector are small. Each level first scores the zero (or inherited) vector,
//! so ties keep it. Every
//! position scores the superblock's 8x8 blocks four horizontal positions at
//! a time; their SADs sum to 16x16, 32x32 and 64x64 blocks, and each block
//! keeps its best vector. Scores are integer SADs per superblock, so the
//! result does not depend on the thread count.

use splot_parallel::WorkerPool;
use splot_parallel::prelude::*;

use crate::frame::SourceFrame;
use crate::kernels::kernels;
use crate::kernels::motion::{Motion, WAYS};
use crate::preset::PresetParams;

/// The side of a superblock, the unit of the search, in full samples.
pub(crate) const SB: usize = 64;
/// Replicated samples around the full-resolution plane; level `l` keeps
/// `PAD >> l`. A block that a vector moves further out is not searched.
const PAD: usize = 128;
/// The half-level refinement radius around the quarter-level vector.
const REFINE: i16 = 2;
/// The reference distance beyond which the coarse range stops growing.
const MAX_DISTANCE_SCALE: u64 = 2;
/// Mean half-level SADs per sample at or below which the full-resolution
/// range halves, and quarters when the vector is also within
/// [`STILL_MV`] full samples.
const LOW_SAD: u32 = 4;
const STILL_SAD: u32 = 2;
const STILL_MV: i16 = 8;
/// Where each block size starts in a [`Tree`]: 64 8x8 blocks, then 16
/// 16x16, 4 32x32 and the 64x64 block, each in raster order.
const OFFSETS: [usize; 5] = [0, 64, 80, 84, 85];

/// The best motion of every block of one superblock, laid out by [`OFFSETS`].
type Tree = [Motion; 85];

/// One padded 8-bit plane covering `width x height` samples.
pub(crate) struct Plane {
    samples: Vec<u8>,
    stride: usize,
    pad: usize,
    width: usize,
    height: usize,
}

impl Plane {
    /// The plane whose row `y` `fill` writes, padded by `pad` copies of its
    /// edge samples.
    fn build(
        width: usize,
        height: usize,
        pad: usize,
        mut fill: impl FnMut(usize, &mut [u8]),
    ) -> Self {
        let stride = width + 2 * pad;
        let mut samples = vec![0; stride * (height + 2 * pad)];
        for y in 0..height {
            let row = &mut samples[(y + pad) * stride..][..stride];
            fill(y, &mut row[pad..pad + width]);
            let (left, right) = (row[pad], row[pad + width - 1]);
            row[..pad].fill(left);
            row[pad + width..].fill(right);
        }
        for y in 0..pad {
            samples.copy_within(pad * stride..(pad + 1) * stride, y * stride);
            let last = (pad + height - 1) * stride;
            samples.copy_within(last..last + stride, (pad + height + y) * stride);
        }
        Self {
            samples,
            stride,
            pad,
            width,
            height,
        }
    }

    /// The plane of 2x2 means of `self`.
    fn halved(&self) -> Self {
        let row = |y: usize| &self.samples[(y + self.pad) * self.stride + self.pad..][..self.width];
        Self::build(self.width / 2, self.height / 2, self.pad / 2, |y, out| {
            kernels().downscale_row(row(2 * y), row(2 * y + 1), out);
        })
    }

    /// The samples from `(x, y)` on, which may lie in the padding; `None`
    /// when a `w x h` block there leaves the padded plane.
    fn block(&self, x: i32, y: i32, w: usize, h: usize) -> Option<&[u8]> {
        let px = usize::try_from(x + self.pad as i32).ok()?;
        let py = usize::try_from(y + self.pad as i32).ok()?;
        (px + w <= self.stride && py + h <= self.height + 2 * self.pad)
            .then(|| &self.samples[py * self.stride + px..])
    }

    /// The sample at `(x, y)`, clamped to the padded plane.
    pub(crate) fn sample(&self, x: i32, y: i32) -> u8 {
        let clamp =
            |v: i32, extent: usize| (v + self.pad as i32).clamp(0, extent as i32 - 1) as usize;
        let (x, y) = (clamp(x, self.stride), clamp(y, self.height + 2 * self.pad));
        self.samples[y * self.stride + x]
    }

    /// The 8x8 block at `(x, y)` as `u16` rows of 8, for the distortion
    /// kernels; `None` when it leaves the padded plane.
    pub(crate) fn block8(&self, x: i32, y: i32) -> Option<[u16; 64]> {
        let block = self.block(x, y, 8, 8)?;
        Some(core::array::from_fn(|i| {
            u16::from(block[i / 8 * self.stride + i % 8])
        }))
    }
}

/// The lowest SAD of the 8x8 block of `current` at `(x, y)` against
/// `reference` within one sample of `centre`, and its vector; `centre` wins
/// a tie, then the vectors in `[0, -1, 1]` order by row and column. `None`
/// when no position fits the padded planes.
pub(crate) fn best_near(
    (current, reference): (&Plane, &Plane),
    (x, y): (i32, i32),
    centre: [i32; 2],
) -> Option<(u32, [i32; 2])> {
    let source = current.block(x, y, 8, 8)?;
    let mut best: Option<(u32, [i32; 2])> = None;
    for dy in [0, -1, 1] {
        let (left, top) = (x + centre[0] - 1, y + centre[1] + dy);
        let Some(candidate) = reference.block(left, top, 8 + WAYS - 1, 8) else {
            continue;
        };
        let mut sads = [[0; WAYS]];
        kernels().sad8x8_x4(
            (source, current.stride),
            (candidate, reference.stride),
            1,
            &mut sads,
        );
        for (lane, dx) in [(1, 0), (0, -1), (2, 1)] {
            if best.is_none_or(|(sad, _)| sads[0][lane] < sad) {
                best = Some((sads[0][lane], [centre[0] + dx, centre[1] + dy]));
            }
        }
    }
    best
}

/// The full-resolution, half and quarter levels of one frame's luma.
pub(crate) struct Pyramid {
    levels: [Plane; 3],
}

impl Pyramid {
    /// The pyramid of `frame`'s luma, shifted down to 8 bits.
    pub(crate) fn new(frame: &SourceFrame) -> Self {
        let size = frame.plane_sizes()[0];
        let (width, height) = (size.width(), size.height());
        let luma = &frame.planes()[0];
        let shift = frame.bit_depth().bits() - 8;
        let full = Plane::build(
            width.div_ceil(SB) * SB,
            height.div_ceil(SB) * SB,
            PAD,
            |y, out| {
                let row = &luma[y.min(height - 1) * width..][..width];
                for (x, out) in out.iter_mut().enumerate() {
                    *out = (row[x.min(width - 1)] >> shift) as u8;
                }
            },
        );
        let half = full.halved();
        let quarter = half.halved();
        Self {
            levels: [full, half, quarter],
        }
    }

    /// Level `level`: `0` full resolution, `1` half, `2` quarter per side.
    pub(crate) fn level(&self, level: usize) -> &Plane {
        &self.levels[level.min(2)]
    }
}

/// The search radii of a preset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Ranges {
    /// Quarter-level radius against the adjacent frame.
    coarse: i16,
    /// Full-resolution radius around the coarse vector.
    full: i16,
}

impl Ranges {
    pub(crate) fn new(preset: &PresetParams) -> Self {
        Self {
            coarse: i16::from(preset.hme_range),
            full: i16::from(preset.me_range),
        }
    }
}

/// The best vector and SAD of every 8x8, 16x16, 32x32 and 64x64 block of a
/// frame against one reference.
pub(crate) struct FrameMotion {
    sb_cols: usize,
    blocks: [Vec<Motion>; 4],
}

impl FrameMotion {
    /// The blocks of side `8 << size` in raster order over the superblock
    /// grid, and how many there are per row.
    pub(crate) fn blocks(&self, size: usize) -> (usize, &[Motion]) {
        let size = size.min(3);
        (self.sb_cols << (3 - size), &self.blocks[size])
    }
}

/// The motion of `current` against `reference`, `distance` frames apart, on
/// `pool` by superblock rows when there is one.
pub(crate) fn search(
    current: &Pyramid,
    reference: &Pyramid,
    distance: u64,
    ranges: Ranges,
    pool: Option<&WorkerPool>,
) -> FrameMotion {
    let (sb_cols, sb_rows) = (current.levels[0].width / SB, current.levels[0].height / SB);
    let scale = distance.clamp(1, MAX_DISTANCE_SCALE) as i16;
    let ranges = Ranges {
        coarse: ranges.coarse.saturating_mul(scale),
        ..ranges
    };
    let row = |sy: usize| -> Vec<Tree> {
        (0..sb_cols)
            .map(|sx| search_superblock(current, reference, (sx, sy), ranges))
            .collect()
    };
    let rows: Vec<Vec<Tree>> = match pool {
        Some(pool) => pool.install(|| (0..sb_rows).into_par_iter().map(row).collect()),
        None => (0..sb_rows).map(row).collect(),
    };
    let blocks = core::array::from_fn(|size| {
        let (side, cols) = (8 >> size, sb_cols << (3 - size));
        let mut blocks = vec![Motion::NONE; cols * (sb_rows << (3 - size))];
        for (sy, row) in rows.iter().enumerate() {
            for (sx, tree) in row.iter().enumerate() {
                for (i, &motion) in tree[OFFSETS[size]..OFFSETS[size + 1]].iter().enumerate() {
                    blocks[(sy * side + i / side) * cols + sx * side + i % side] = motion;
                }
            }
        }
        blocks
    });
    FrameMotion { sb_cols, blocks }
}

/// The three-level search of superblock `(sx, sy)`.
fn search_superblock(
    current: &Pyramid,
    reference: &Pyramid,
    (sx, sy): (usize, usize),
    ranges: Ranges,
) -> Tree {
    let level = |level: usize, windows: &[([i32; 2], i32)]| {
        let mut tree = [Motion::NONE; 85];
        let planes = (&current.levels[level], &reference.levels[level]);
        let origin = (((sx * SB) >> level) as i32, ((sy * SB) >> level) as i32);
        for &(centre, radius) in windows {
            scan(planes, origin, SB >> level, centre, radius, &mut tree);
        }
        tree
    };
    let doubled = |motion: Motion| motion.mv.map(|v| 2 * i32::from(v));
    let coarse = i32::from(ranges.coarse);
    let quarter = level(2, &[([0, 0], 0), ([0, 0], coarse)])[OFFSETS[1]];
    let centre = doubled(quarter);
    let half = level(1, &[(centre, 0), (centre, i32::from(REFINE))])[OFFSETS[2]];
    let centre = doubled(half);
    let samples = ((SB / 2) * (SB / 2)) as u32;
    let still = centre.iter().all(|v| v.abs() <= i32::from(STILL_MV));
    let full = i32::from(ranges.full).max(1);
    let shrunk = if half.sad <= STILL_SAD * samples && still {
        full / 4
    } else if half.sad <= LOW_SAD * samples {
        full / 2
    } else {
        full
    };
    let windows = [
        ([0, 0], 0),
        ([0, 0], (full / 2).max(1)),
        (centre, shrunk.max(1)),
    ];
    level(0, &windows[..if centre == [0, 0] { 2 } else { 3 }])
}

/// Scores the `side x side` block at `origin` at every vector within
/// `radius` of `centre`, in rows of [`WAYS`] positions from the window's
/// left edge, and keeps each sub-block's best in `tree`.
fn scan(
    (current, reference): (&Plane, &Plane),
    (x, y): (i32, i32),
    side: usize,
    centre: [i32; 2],
    radius: i32,
    tree: &mut Tree,
) {
    let Some(source) = current.block(x, y, side, side) else {
        return;
    };
    let kernels = kernels();
    let grid = side / 8;
    let mut sums = [[0; WAYS]; 85];
    let groups = (2 * radius + WAYS as i32) / WAYS as i32;
    for dy in -radius..=radius {
        for group in 0..groups {
            let mv = [centre[0] - radius + group * WAYS as i32, centre[1] + dy];
            let (Ok(mv_x), Ok(mv_y)) = (i16::try_from(mv[0]), i16::try_from(mv[1])) else {
                continue;
            };
            let Some(candidate) = reference.block(x + mv[0], y + mv[1], side + WAYS - 1, side)
            else {
                continue;
            };
            let blocks = grid * grid;
            kernels.sad8x8_x4(
                (source, current.stride),
                (candidate, reference.stride),
                grid,
                &mut sums[..blocks],
            );
            kernels.keep_best(&sums[..blocks], [mv_x, mv_y], &mut tree[..blocks]);
            let (mut size, mut cols) = (0, grid);
            while cols > 1 {
                let (lower, upper) = sums.split_at_mut(OFFSETS[size + 1]);
                let blocks = (cols / 2) * (cols / 2);
                kernels.sad_quads(
                    &lower[OFFSETS[size]..][..cols * cols],
                    cols,
                    &mut upper[..blocks],
                );
                let best = &mut tree[OFFSETS[size + 1]..][..blocks];
                kernels.keep_best(&upper[..blocks], [mv_x, mv_y], best);
                (size, cols) = (size + 1, cols / 2);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use splot_parallel::ThreadCount;

    use super::*;
    use crate::config::Config;
    use crate::lookahead::tests::frame;
    use crate::preset::PRESETS;

    /// A frame of smooth, non-repeating texture moved right by `shift`
    /// samples, so that the coarse levels keep its structure.
    fn smooth(config: &Config, shift: usize) -> SourceFrame {
        let sizes = config.plane_sizes().unwrap();
        let planes = core::array::from_fn(|plane| {
            let size = sizes[plane];
            (0..size.width() * size.height())
                .map(|i| {
                    let x = (i % size.width()) as f64 - shift as f64;
                    let y = (i / size.width()) as f64;
                    let v = 120.0
                        + 50.0 * (0.21 * x + 0.05 * y).sin()
                        + 40.0 * (0.13 * y - 0.04 * x).cos()
                        + 25.0 * (0.0011 * x * y).sin();
                    v.round() as u16
                })
                .collect()
        });
        SourceFrame::new(config, shift as u64, planes).unwrap()
    }

    fn pyramids(config: &Config, shift: usize) -> (Pyramid, Pyramid) {
        let reference = Pyramid::new(&smooth(config, 0));
        (Pyramid::new(&smooth(config, shift)), reference)
    }

    /// A pan of 37 samples, beyond the full-resolution range of every preset
    /// row, is found through the coarse levels in every block size of the
    /// superblocks whose samples all move.
    #[test]
    fn finds_a_pan_beyond_the_full_resolution_range() {
        let config = Config::new(200, 136);
        let (current, reference) = pyramids(&config, 37);
        let ranges = Ranges::new(&PRESETS[6]);
        assert!(ranges.full < 37);
        let motion = search(&current, &reference, 1, ranges, None);
        for size in 0..4 {
            let (cols, blocks) = motion.blocks(size);
            assert_eq!(blocks.len(), cols * (3 << (3 - size)));
            let side = 8 << size;
            let interior = |i: usize| {
                let (x, y) = (i % cols * side, i / cols * side);
                (64..192).contains(&x) && y < 128
            };
            let found = blocks
                .iter()
                .enumerate()
                .filter(|&(i, m)| interior(i) && m.mv == [-37, 0] && m.sad == 0)
                .count();
            let total = (0..blocks.len()).filter(|&i| interior(i)).count();
            assert_eq!(found, total, "size {size}");
        }
        let narrow = Ranges {
            coarse: 5,
            ..ranges
        };
        let near = search(&current, &reference, 1, narrow, None);
        assert!(
            near.blocks(1).1.iter().all(|m| m.mv != [-37, 0]),
            "the coarse range bounds the search"
        );
        let far = search(&current, &reference, 2, narrow, None);
        let found = far.blocks(1).1.iter().filter(|m| m.mv == [-37, 0]).count();
        assert!(found >= 30, "distance scales the range: {found}");
    }

    #[test]
    fn worker_pools_give_the_same_motion() {
        let config = Config::new(320, 200);
        let (current, reference) = pyramids(&config, 5);
        let ranges = Ranges::new(&PRESETS[0]);
        let serial = search(&current, &reference, 2, ranges, None);
        for threads in [1usize, 2, 8] {
            let pool = WorkerPool::new(ThreadCount::from(threads)).unwrap();
            let parallel = search(&current, &reference, 2, ranges, Some(&pool));
            for size in 0..4 {
                assert_eq!(
                    serial.blocks(size),
                    parallel.blocks(size),
                    "{threads} threads"
                );
            }
        }
    }

    #[test]
    fn levels_halve_and_pad() {
        let config = Config::new(70, 10);
        let pyramid = Pyramid::new(&frame(&config, 0, 0, 0));
        let (full, half) = (pyramid.level(0), pyramid.level(1));
        assert_eq!(
            (full.width, full.height, half.width, pyramid.level(2).height),
            (128, 64, 64, 16)
        );
        let mean = (0..4)
            .map(|i| u16::from(full.sample(2 + i % 2, 4 + i / 2)))
            .sum::<u16>();
        assert_eq!(u16::from(half.sample(1, 2)), mean / 4);
        assert_eq!(full.sample(-200, 3), full.sample(0, 3));
        assert_eq!(full.sample(127 + 9, 70), full.sample(127, 63));
        assert!(full.block8(-128, -128).is_some() && full.block8(-129, 0).is_none());
    }
}
