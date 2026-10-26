// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Intra block copy search: a table of the 8x8 block hashes of the luma
//! source finds earlier copies of a leaf, and a +-4 sample search refines
//! the closest one.

use std::collections::HashMap;

use splot_decode::encoder_syntax::{IntraTileSyntax, IntrabcModes, LeafBlock, MotionVector};

use crate::intra_frame::SourceResidualCoder;
use crate::kernels::{BlockDims, kernels};
use crate::tile_writer::{LeafChoice, Result};

/// The side of the hashed blocks.
const SIDE: usize = 8;
/// The most accepted earlier positions with the leaf's hash that are priced.
const MAX_CANDIDATES: usize = 32;
/// The most earlier positions with the leaf's hash that are looked at; the
/// closest ones are often within the IntraBC delay.
const MAX_SCAN: usize = 1024;
/// The refinement reach around the best hashed copy, in samples.
const REFINE: i64 = 4;

/// The 8x8 block hashes of one luma plane and the positions of each hash.
pub(crate) struct BvSearch {
    /// Positions per row of `hashes`: `width - 7`.
    columns: usize,
    rows: usize,
    hashes: Vec<u32>,
    /// Raster positions (`y * columns + x`) of each hash, ascending.
    table: HashMap<u32, Vec<u32>>,
}

impl BvSearch {
    /// The hashes of every 8x8 block of the `width x height` plane `luma`
    /// with stride `stride`.
    pub(crate) fn new(luma: &[u16], stride: usize, (width, height): (usize, usize)) -> Self {
        let columns = (width + 1).saturating_sub(SIDE);
        let rows = (height + 1).saturating_sub(SIDE);
        let mut row_hashes = vec![0u32; columns * height];
        for (y, out) in row_hashes.chunks_exact_mut(columns.max(1)).enumerate() {
            if let Some(row) = luma.get(y * stride..y * stride + width) {
                kernels().row_hashes(row, out);
            }
        }
        let mut hashes = vec![0u32; columns * rows];
        for (y, out) in hashes.chunks_exact_mut(columns.max(1)).enumerate() {
            let run = |j: usize| &row_hashes[(y + j) * columns..(y + j + 1) * columns];
            kernels().column_hashes(core::array::from_fn(run), out);
        }
        let mut table: HashMap<u32, Vec<u32>> = HashMap::new();
        for (at, &hash) in hashes.iter().enumerate() {
            table.entry(hash).or_default().push(at as u32);
        }
        Self {
            columns,
            rows,
            hashes,
            table,
        }
    }

    /// The whole-sample block vector `[row, col]` in 1/8 samples, and its
    /// squared error, of the copy of the `w x h` luma block at `(x, y)` with
    /// the lowest squared error over `luma` (stride `stride`) among: the
    /// [`MAX_CANDIDATES`] closest earlier blocks (of the [`MAX_SCAN`]
    /// closest) whose top-left 8x8 hash matches and that `valid` accepts, then every vector within
    /// [`REFINE`] samples of the best of them that `valid` accepts. A tie
    /// keeps the earlier candidate. `None` without an accepted candidate.
    pub(crate) fn search(
        &self,
        (luma, stride): (&[u16], usize),
        (x, y, w, h): (usize, usize, usize, usize),
        valid: impl Fn([i32; 2]) -> bool,
    ) -> Option<([i32; 2], u64)> {
        if w < SIDE || h < SIDE || x >= self.columns || y >= self.rows {
            return None;
        }
        let dims = BlockDims::new(w, h).ok()?;
        let block = luma.get(y * stride + x..)?;
        let cost = |dy: i64, dx: i64| -> Option<([i32; 2], u64)> {
            let (sy, sx) = (y as i64 + dy, x as i64 + dx);
            let mv = [i32::try_from(dy * 8).ok()?, i32::try_from(dx * 8).ok()?];
            if sy < 0 || sx < 0 || !valid(mv) {
                return None;
            }
            let source = luma.get(sy as usize * stride + sx as usize..)?;
            if source.len() < (h - 1) * stride + w {
                return None;
            }
            Some((mv, kernels().sse(block, stride, source, stride, dims)))
        };
        let here = (y * self.columns + x) as u32;
        let positions = self.table.get(self.hashes.get(here as usize)?)?;
        let earlier = positions.partition_point(|&at| at < here);
        let mut best: Option<([i32; 2], u64)> = None;
        let mut priced = 0;
        for &at in positions[..earlier].iter().rev().take(MAX_SCAN) {
            let (sy, sx) = (
                (at as usize / self.columns) as i64,
                (at as usize % self.columns) as i64,
            );
            let Some(found) = cost(sy - y as i64, sx - x as i64) else {
                continue;
            };
            if best.is_none_or(|(_, sse)| found.1 < sse) {
                best = Some(found);
            }
            priced += 1;
            if priced == MAX_CANDIDATES {
                break;
            }
        }
        let ([row, col], _) = best?;
        let (row, col) = (i64::from(row / 8), i64::from(col / 8));
        for dy in -REFINE..=REFINE {
            for dx in -REFINE..=REFINE {
                if let Some(found) = cost(row + dy, col + dx)
                    && best.is_none_or(|(_, sse)| found.1 < sse)
                {
                    best = Some(found);
                }
            }
        }
        best
    }
}

/// The IntraBC candidate of `leaf` in a frame whose block hashes `coder`
/// holds: the [`BvSearch::search`] copy of its luma source among the
/// vectors the leaf may code, with the `RefMvIdx` whose reference vector it
/// equals, else the closest one; `None` without one.
pub(crate) fn intrabc_choice(
    syntax: &mut IntraTileSyntax,
    coder: &SourceResidualCoder<'_>,
    leaf: &LeafBlock,
) -> Result<Option<LeafChoice>> {
    let (Some(search), Some(vectors)) = (coder.intrabc, syntax.intrabc_vectors(leaf)) else {
        return Ok(None);
    };
    let (w, h) = leaf.dimensions()?;
    let (luma, stride, ..) = coder.source.luma();
    let valid = |[row, col]: [i32; 2]| vectors.valid(MotionVector { row, col });
    let rect = (leaf.col4() * 4, leaf.row4() * 4, w, h);
    let Some(([row, col], _)) = search.search((luma, stride), rect, valid) else {
        return Ok(None);
    };
    let bv = MotionVector { row, col };
    let refs = syntax.intrabc_ref_bvs(leaf)?;
    let distance =
        |r: &MotionVector| (bv.row - r.row).unsigned_abs() + (bv.col - r.col).unsigned_abs();
    let drl = (0..refs.len())
        .min_by_key(|&i| distance(&refs[i]))
        .unwrap_or(0);
    Ok(Some(LeafChoice::intrabc(IntrabcModes { bv, drl })))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 64x32 plane of noise with the 16x8 block at (8, 4) copied to
    /// (40, 20) and a near copy (one sample changed) at (24, 12).
    fn plane() -> Vec<u16> {
        let mut plane: Vec<u16> = (0..64 * 32u32)
            .map(|i| ((i.wrapping_mul(2_654_435_761) >> 7) % 1021) as u16)
            .collect();
        for row in 0..8 {
            for col in 0..16 {
                let sample = plane[(4 + row) * 64 + 8 + col];
                plane[(20 + row) * 64 + 40 + col] = sample;
                plane[(12 + row) * 64 + 24 + col] = sample;
            }
        }
        plane[(12 + 7) * 64 + 24 + 15] ^= 1;
        plane
    }

    #[test]
    fn finds_the_exact_earlier_copy() {
        let plane = plane();
        let search = BvSearch::new(&plane, 64, (64, 32));
        let found = search.search((&plane, 64), (40, 20, 16, 8), |_| true);
        assert_eq!(found, Some(([-16 * 8, -32 * 8], 0)));
    }

    /// Without the exact copy, the hash of the top-left 8x8 still finds the
    /// near copy, whose squared error is the one changed sample.
    #[test]
    fn falls_back_to_valid_near_copies() {
        let plane = plane();
        let search = BvSearch::new(&plane, 64, (64, 32));
        let found = search.search((&plane, 64), (40, 20, 16, 8), |mv| mv[0] != -16 * 8);
        assert_eq!(found, Some(([-8 * 8, -16 * 8], 1)));
        assert_eq!(
            search.search((&plane, 64), (40, 20, 16, 8), |_| false),
            None
        );
        assert_eq!(search.search((&plane, 64), (40, 20, 4, 8), |_| true), None);
    }
}
