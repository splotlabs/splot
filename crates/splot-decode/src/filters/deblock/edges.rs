// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! One plane pass's § 7.17.2 edge decisions, laid out for the batched
//! kernels: every edge segment the pass owes gets its [`DeblockEdge`], and the
//! kernels then filter whole columns (vertical edges) and rows (horizontal
//! edges) of them.

use splot_core::tables::conversion::{Q_THRESH_MULTS, W_MULT};
use splot_recon::{
    BitDepth, DeblockEdge, DeblockSampleFilter, ReconSample, deblock_horizontal_edges,
    deblock_vertical_edges,
};

use super::grid::{CellFacts, MiGrid};
use super::{
    DeblockError, EdgeContext, GATHER_HALF, MAX_WIDTHS, MI_SIZE, PerpLine, PlaneCtx, PlanePass,
    SB_SIZE, StrengthCache, apply_edge_samples, choose_filter_width, combine_strengths,
    edge_blocks, edge_decision, starts_tile,
};

/// The edges of one plane pass over the mode-info rows it owes, by plane
/// 4x4 cell: column-major for the vertical pass, so that one column's
/// segments lie together, and row-major for the horizontal one.
#[derive(Default)]
pub(crate) struct PassEdges {
    edges: Vec<DeblockEdge>,
    rows: usize,
    cols: usize,
    first_row: usize,
    /// One cell row's transform edges, 64 cells per word.
    words: Vec<u64>,
}

/// Where one plane pass's edges sit: its plane geometry and the frame
/// geometry its edge decisions read.
pub(super) struct PassGeometry<'a> {
    pub(super) mi_rows: usize,
    pub(super) mi_cols: usize,
    pub(super) tile_starts: Option<&'a [u32]>,
    pub(super) disable_loopfilters_across_tiles: bool,
    pub(super) width: usize,
    pub(super) height: usize,
}

/// [`deblock_filter_max_width`](splot_recon::deblock_filter_max_width) as
/// edge shapes, by chroma plane, super-block edge and filter size.
const SHAPES: [[[DeblockEdge; 65]; 2]; 2] = {
    let mut table = [[[DeblockEdge::NONE; 65]; 2]; 2];
    let mut entry = 0;
    while entry < 4 * 65 {
        let (chroma, sb_edge, size) = (entry / 130, entry / 65 % 2, entry % 65);
        let (neg, pos) = MAX_WIDTHS[chroma][sb_edge][size];
        if let Some(edge) = DeblockEdge::new(1, 1, neg as usize, pos as usize, false, false) {
            table[chroma][sb_edge][size] = edge;
        }
        entry += 1;
    }
    table
};

/// The § 7.17.2 decision of the transform edge between `prev` and `curr`
/// at plane sample position `at` across it, of strengths
/// `(qThr, sideThr)`: filtered when the current side is coded or starts its
/// prediction unit and either side's strengths are both nonzero.
#[allow(clippy::inline_always, reason = "measured deblock hot path")]
#[inline(always)]
fn decide(
    (curr, curr_strength): (CellFacts, (i32, i32)),
    (prev, prev_strength): (CellFacts, (i32, i32)),
    (at, pass): (usize, usize),
    limit: u8,
    shapes: &[DeblockEdge; 65],
) -> DeblockEdge {
    let coded = usize::from(curr.prediction[pass]) == at || curr.flags & CellFacts::SKIP == 0;
    let strong = |(q_thr, side): (i32, i32)| q_thr != 0 && side != 0;
    if !coded || !(strong(curr_strength) || strong(prev_strength)) {
        return DeblockEdge::NONE;
    }
    let size = curr.tx_size[pass]
        .min(prev.tx_size[pass])
        .min(limit)
        .min(64);
    let (q_thr, side) = combine_strengths(
        curr_strength.0,
        prev_strength.0,
        curr_strength.1,
        prev_strength.1,
    );
    let threshold = |value: i32| value.clamp(0, i32::from(u16::MAX)) as u16;
    shapes[usize::from(size)]
        .with_lossless(
            prev.flags & CellFacts::LOSSLESS != 0,
            curr.flags & CellFacts::LOSSLESS != 0,
        )
        .with_thresholds(threshold(q_thr), threshold(side))
}

/// The bitmask of the cells of `curr` whose transform base differs from
/// `prev`'s at the same index, 64 cells per word, with whether every cell
/// of both rows is covered.
fn transform_edges(curr: &[CellFacts], prev: &[CellFacts], words: &mut Vec<u64>) -> bool {
    words.clear();
    let mut covered = CellFacts::COVERED;
    for (curr, prev) in curr.chunks(64).zip(prev.chunks(64)) {
        let mut word = 0u64;
        for (bit, (curr, prev)) in curr.iter().zip(prev).enumerate() {
            word |= u64::from(curr.tx_base != prev.tx_base) << bit;
            covered &= curr.flags & prev.flags;
        }
        words.push(word);
    }
    covered != 0
}

impl PassEdges {
    /// Decides every edge of `plane_pass` from the records on its two sides.
    pub(super) fn build(
        &mut self,
        grid: &MiGrid<'_>,
        plane_pass: &PlanePass,
        geometry: &PassGeometry<'_>,
    ) -> Result<(), DeblockError> {
        match (plane_pass.plane, plane_pass.pass) {
            (0, 0) => self.build_specialized::<0, 0>(grid, plane_pass, geometry),
            (0, _) => self.build_specialized::<0, 1>(grid, plane_pass, geometry),
            (1, 0) => self.build_specialized::<1, 0>(grid, plane_pass, geometry),
            (1, _) => self.build_specialized::<1, 1>(grid, plane_pass, geometry),
            (2, 0) => self.build_specialized::<2, 0>(grid, plane_pass, geometry),
            _ => self.build_specialized::<2, 1>(grid, plane_pass, geometry),
        }
    }

    fn build_specialized<const PLANE: usize, const PASS: usize>(
        &mut self,
        grid: &MiGrid<'_>,
        plane_pass: &PlanePass,
        geometry: &PassGeometry<'_>,
    ) -> Result<(), DeblockError> {
        let (sub_x, sub_y) = (plane_pass.plane_sub_x, plane_pass.plane_sub_y);
        let row_step = plane_pass.row_step;
        let end = plane_pass.mi_row_range.1.min(geometry.mi_rows);
        let mut first = plane_pass
            .mi_row_range
            .0
            .div_ceil(row_step)
            .saturating_mul(row_step);
        if PASS == 1 && first == 0 {
            first = row_step;
        }
        self.first_row = first >> sub_y;
        self.rows = end.saturating_sub(first).div_ceil(row_step);
        self.cols = geometry.mi_cols.div_ceil(1 << sub_x);
        self.edges.clear();
        let len = self.rows * self.cols;
        self.edges
            .try_reserve(len)
            .map_err(|_| DeblockError::Allocation {
                plane: splot_recon::PlaneId::Y,
                context: "deblock edge decisions",
            })?;
        self.edges.resize(len, DeblockEdge::NONE);
        let strengths = StrengthCache::new(
            plane_pass.quant_delta,
            plane_pass.df_delta_q,
            plane_pass.bit_depth,
        );
        if !plane_pass.allow_df_sub_pu {
            return self.build_without_sub_pu::<PLANE, PASS>(
                grid, plane_pass, geometry, first, &strengths,
            );
        }
        for row_index in 0..self.rows {
            let r = first + row_index * row_step;
            let row_tile_edge = PASS == 1 && starts_tile(geometry.tile_starts, r);
            for col_index in usize::from(PASS == 0)..self.cols {
                let c = col_index << sub_x;
                let tile_edge = row_tile_edge || PASS == 0 && starts_tile(geometry.tile_starts, c);
                let edge = edge_at::<PLANE, PASS>(
                    grid,
                    plane_pass.edge_context(r, c, tile_edge),
                    geometry,
                    &strengths,
                )?;
                let slot = if PASS == 0 {
                    col_index * self.rows + row_index
                } else {
                    row_index * self.cols + col_index
                };
                self.edges[slot] = edge;
            }
        }
        Ok(())
    }

    /// [`Self::build_specialized`] for a frame without sub-PU edges, where an
    /// edge filters only across a transform edge and its decision reads only
    /// the [`CellFacts`] of its two sides, which the grid resolved once per
    /// cell. Each row finds its transform edges as a bitmask first, so the
    /// decisions cost only where edges are.
    fn build_without_sub_pu<const PLANE: usize, const PASS: usize>(
        &mut self,
        grid: &MiGrid<'_>,
        plane_pass: &PlanePass,
        geometry: &PassGeometry<'_>,
        first: usize,
        strengths: &StrengthCache,
    ) -> Result<(), DeblockError> {
        let sub_x = plane_pass.plane_sub_x;
        let (rows, cols) = (self.rows, self.cols);
        let reach = if PLANE == 0 { 16 } else { 8 };
        let shapes = &SHAPES[usize::from(PLANE != 0)];
        let mut last = (u16::MAX, (0, 0));
        let mut strength = |qindex: u16| {
            if qindex != last.0 {
                last = (qindex, strengths.get(qindex.into()));
            }
            last.1
        };
        let mut words = core::mem::take(&mut self.words);
        let result = (|| {
            for row_index in 0..rows {
                let cell_row = self.first_row + row_index;
                let r = first + row_index * plane_pass.row_step;
                let y_p = cell_row * MI_SIZE;
                let uncovered = || DeblockError::UncoveredMi { row: r, col: 0 };
                let row = |at: usize| {
                    grid.facts_row(at)
                        .and_then(|facts| facts.get(..cols))
                        .ok_or_else(uncovered)
                };
                let curr = row(cell_row)?;
                let (curr_cells, prev_cells) = if PASS == 0 {
                    (&curr[1..], &curr[..cols - 1])
                } else {
                    (curr, row(cell_row - 1)?)
                };
                if !transform_edges(curr_cells, prev_cells, &mut words) {
                    return Err(uncovered());
                }
                let tile_row = PASS == 1 && starts_tile(geometry.tile_starts, r);
                if PASS == 1 && geometry.disable_loopfilters_across_tiles && tile_row {
                    continue;
                }
                let sb_row = PASS == 1 && (r * MI_SIZE).is_multiple_of(SB_SIZE);
                let row_limit = PASS == 1 && y_p + reach > geometry.height;
                for (word_index, &word) in words.iter().enumerate() {
                    let mut word = word;
                    while word != 0 {
                        let bit = word.trailing_zeros() as usize;
                        word &= word - 1;
                        let index = word_index * 64 + bit;
                        let col = index + usize::from(PASS == 0);
                        let (curr, prev) = (curr_cells[index], prev_cells[index]);
                        let tile = PASS == 0 && starts_tile(geometry.tile_starts, col << sub_x);
                        if tile && geometry.disable_loopfilters_across_tiles {
                            continue;
                        }
                        let at = if PASS == 0 { col * MI_SIZE } else { y_p };
                        let near = row_limit || PASS == 0 && at + reach > geometry.width;
                        let limit = if near { reach as u8 } else { 64 };
                        let edge = decide(
                            (curr, strength(curr.qindex)),
                            (prev, strength(prev.qindex)),
                            (at, PASS),
                            limit,
                            &shapes[usize::from(sb_row || tile)],
                        );
                        let slot = if PASS == 0 {
                            col * rows + row_index
                        } else {
                            row_index * cols + col
                        };
                        self.edges[slot] = edge;
                    }
                }
            }
            Ok(())
        })();
        self.words = words;
        result
    }

    /// Filters every decided edge: whole columns or rows through the batched
    /// kernels where their samples 8 either side of the edge lie in the
    /// band, and the rest one by one.
    pub(super) fn apply<T: ReconSample, const PASS: usize>(
        &self,
        ctx: &mut PlaneCtx<'_, '_, T>,
        bit_depth: BitDepth,
    ) -> Result<(), DeblockError> {
        let (width, height, y_origin) = (ctx.width, ctx.height, ctx.y_origin);
        let lines = |count: usize, first: usize, limit: usize| {
            (0..count)
                .take_while(|&k| (first + k) * MI_SIZE + MI_SIZE <= limit)
                .count()
        };
        let (outer, inner) = if PASS == 0 {
            (self.cols, self.rows)
        } else {
            (self.rows, self.cols)
        };
        for line in 0..outer {
            let edges = &self.edges[line * inner..(line + 1) * inner];
            if edges.iter().all(|edge| edge.is_none()) {
                continue;
            }
            let (x_p, y_p) = if PASS == 0 {
                (line * MI_SIZE, self.first_row * MI_SIZE)
            } else {
                (0, (self.first_row + line) * MI_SIZE)
            };
            let full = if PASS == 0 {
                let fits = x_p >= GATHER_HALF && x_p + GATHER_HALF <= width;
                if fits {
                    lines(inner, self.first_row, height)
                } else {
                    0
                }
            } else {
                let fits = y_p >= y_origin + GATHER_HALF && y_p + GATHER_HALF <= height;
                if fits { lines(inner, 0, width) } else { 0 }
            };
            if full > 0 {
                let (samples, stride) = ctx.rows.contiguous_mut();
                let first = (y_p - y_origin) * stride + x_p;
                let kernel = if PASS == 0 {
                    deblock_vertical_edges::<T>
                } else {
                    deblock_horizontal_edges::<T>
                };
                kernel(samples, stride, first, &edges[..full], bit_depth)
                    .map_err(|_| DeblockError::SampleFilter)?;
            }
            for (k, &edge) in edges.iter().enumerate().skip(full) {
                if edge.is_none() {
                    continue;
                }
                let (x_p, y_p) = if PASS == 0 {
                    (x_p, y_p + k * MI_SIZE)
                } else {
                    (k * MI_SIZE, y_p)
                };
                LoneEdge { x_p, y_p, edge }.filter::<T, PASS>(ctx, bit_depth)?;
            }
        }
        Ok(())
    }
}

/// The edge at mode-info (`row`, `col`) of `ctx`, which is on screen.
#[allow(clippy::inline_always, reason = "measured deblock hot path")]
#[inline(always)]
pub(super) fn edge_at<const PLANE: usize, const PASS: usize>(
    grid: &MiGrid<'_>,
    ctx: EdgeContext,
    geometry: &PassGeometry<'_>,
    strengths: &StrengthCache,
) -> Result<DeblockEdge, DeblockError> {
    let EdgeContext {
        row,
        col,
        plane_sub_x,
        plane_sub_y,
        tile_edge,
        ..
    } = ctx;
    let (curr, prev) = edge_blocks::<PLANE, PASS>(grid, row, col, plane_sub_x, plane_sub_y)?;
    if geometry.disable_loopfilters_across_tiles && tile_edge {
        return Ok(DeblockEdge::NONE);
    }
    let sb_edge = PASS == 1 && (row * MI_SIZE).is_multiple_of(SB_SIZE) || PASS == 0 && tile_edge;
    let x_p = (col * MI_SIZE) >> plane_sub_x;
    let y_p = (row * MI_SIZE) >> plane_sub_y;
    let Some(decision) = edge_decision::<PLANE, PASS>(curr, prev, x_p, y_p, ctx, strengths) else {
        return Ok(DeblockEdge::NONE);
    };
    let mut filter_size = decision.filter_size;
    let (dx, dy) = if PASS == 0 { (1, 0) } else { (0, 1) };
    let reach = if PLANE == 0 { 16 } else { 8 };
    if x_p + dx * reach > geometry.width || y_p + dy * reach > geometry.height {
        filter_size = filter_size.min(reach);
    }
    let (max_width_neg, max_width_pos) =
        MAX_WIDTHS[usize::from(PLANE != 0)][usize::from(sb_edge)][filter_size.min(64)];
    if max_width_neg == 0 || max_width_pos == 0 {
        return Ok(DeblockEdge::NONE);
    }
    DeblockEdge::new(
        decision.q_thr,
        decision.side,
        max_width_neg.into(),
        max_width_pos.into(),
        decision.prev_lossless,
        decision.curr_lossless,
    )
    .ok_or(DeblockError::SampleFilter)
}

/// An edge at plane position (`x_p`, `y_p`) that the batched kernels cannot
/// reach, filtered on its own.
#[derive(Clone, Copy)]
pub(super) struct LoneEdge {
    pub(super) x_p: usize,
    pub(super) y_p: usize,
    pub(super) edge: DeblockEdge,
}

/// Samples a lone edge's window holds: its four lines of `2 * GATHER_HALF`.
const WINDOW: usize = MI_SIZE * 2 * GATHER_HALF;

impl LoneEdge {
    /// Chooses the filter width from the plane's own samples, then filters.
    pub(super) fn filter<T: ReconSample, const PASS: usize>(
        self,
        plane_ctx: &mut PlaneCtx<'_, '_, T>,
        bit_depth: BitDepth,
    ) -> Result<(), DeblockError> {
        if self.filter_window::<T, PASS>(plane_ctx, bit_depth)? {
            return Ok(());
        }
        self.filter_lines::<T, PASS>(plane_ctx, bit_depth)
    }

    /// Chooses and filters the edge line by line on clamped gathers.
    pub(super) fn filter_lines<T: ReconSample, const PASS: usize>(
        self,
        plane_ctx: &mut PlaneCtx<'_, '_, T>,
        bit_depth: BitDepth,
    ) -> Result<(), DeblockError> {
        let (dx, dy) = if PASS == 0 { (1, 0) } else { (0, 1) };
        let edge = self.edge;
        let (max_width_neg, max_width_pos) = (edge.max_width_neg(), edge.max_width_pos());
        let (q_thr, side) = (i32::from(edge.q_thr()), i32::from(edge.side()));
        let width = choose_filter_width(
            plane_ctx,
            self.x_p,
            self.y_p,
            dx,
            dy,
            q_thr,
            side,
            max_width_neg,
            max_width_pos,
        )?;
        if width == 0 {
            return Ok(());
        }
        let eff_neg = width.min(max_width_neg);
        let eff_pos = width.min(max_width_pos);
        let sample_params = DeblockSampleFilter {
            boundary: GATHER_HALF,
            q_thr,
            max_width_neg: eff_neg,
            max_width_pos: eff_pos,
            q_thresh_mult: Q_THRESH_MULTS[eff_neg.max(eff_pos) - 1],
            w_mult_neg: W_MULT[eff_neg - 1],
            w_mult_pos: W_MULT[eff_pos - 1],
            prev_lossless: edge.prev_lossless(),
            curr_lossless: edge.curr_lossless(),
            bit_depth,
        };
        apply_edge_samples(
            plane_ctx,
            PerpLine::new(self.x_p, self.y_p, dx, dy),
            MI_SIZE,
            sample_params,
        )
    }

    /// Filters the edge with the batched kernel on a copy of its four
    /// lines whose samples past the plane or band repeat its last ones, as
    /// the per-line path gathers them, then writes the copy back. Returns
    /// `false`, having written nothing, when the edge has fewer than four
    /// lines in the plane or a line outside the band. A changed sample
    /// outside the plane or band is the error the per-line path gives.
    fn filter_window<T: ReconSample, const PASS: usize>(
        self,
        ctx: &mut PlaneCtx<'_, '_, T>,
        bit_depth: BitDepth,
    ) -> Result<bool, DeblockError> {
        let (x, y) = (self.x_p, self.y_p);
        let band_end = ctx.y_origin + ctx.band_rows;
        let fits = if PASS == 0 {
            y >= ctx.y_origin && y + MI_SIZE <= ctx.height.min(band_end)
        } else {
            x + MI_SIZE <= ctx.width && ctx.band_rows > 0
        };
        if !fits || ctx.width == 0 {
            return Ok(false);
        }
        let mut window = [T::default(); WINDOW];
        let line = 2 * GATHER_HALF;
        let across = |i: usize, base: usize, low: usize, high: usize| {
            (base + i).saturating_sub(GATHER_HALF).clamp(low, high - 1)
        };
        if PASS == 0 {
            for (lane, samples) in window.chunks_exact_mut(line).enumerate() {
                let row = ctx
                    .rows
                    .row(y + lane - ctx.y_origin)
                    .ok_or(DeblockError::Workspace)?;
                for (i, sample) in samples.iter_mut().enumerate() {
                    *sample = *row
                        .get(across(i, x, 0, ctx.width))
                        .ok_or(DeblockError::Workspace)?;
                }
            }
        } else {
            for (i, samples) in window.chunks_exact_mut(MI_SIZE).enumerate() {
                let row = across(i, y, ctx.y_origin, band_end) - ctx.y_origin;
                let row = ctx.rows.row(row).ok_or(DeblockError::Workspace)?;
                samples.copy_from_slice(row.get(x..x + MI_SIZE).ok_or(DeblockError::Workspace)?);
            }
        }
        let original = window;
        let filtered = if PASS == 0 {
            deblock_vertical_edges(&mut window, line, GATHER_HALF, &[self.edge], bit_depth)
        } else {
            deblock_horizontal_edges(
                &mut window,
                MI_SIZE,
                GATHER_HALF * MI_SIZE,
                &[self.edge],
                bit_depth,
            )
        };
        filtered.map_err(|_| DeblockError::SampleFilter)?;
        let (low, high) = if PASS == 0 {
            (0, ctx.width)
        } else {
            (ctx.y_origin, band_end.min(ctx.height))
        };
        let base = if PASS == 0 { x } else { y };
        let first = (low + GATHER_HALF).saturating_sub(base).min(line);
        let end = (high + GATHER_HALF).saturating_sub(base).clamp(first, line);
        let outside = |i: usize| i < first || i >= end;
        let across_index = |k: usize| if PASS == 0 { k % line } else { k / MI_SIZE };
        let changed_outside = window
            .iter()
            .zip(&original)
            .enumerate()
            .any(|(k, (new, old))| outside(across_index(k)) && new.to_u16() != old.to_u16());
        if changed_outside {
            return Err(DeblockError::Workspace);
        }
        if PASS == 0 {
            for (lane, samples) in window.chunks_exact(line).enumerate() {
                let row = ctx
                    .rows
                    .row_mut(y + lane - ctx.y_origin)
                    .ok_or(DeblockError::Workspace)?;
                let start = base + first - GATHER_HALF;
                row.get_mut(start..start + end - first)
                    .ok_or(DeblockError::Workspace)?
                    .copy_from_slice(&samples[first..end]);
            }
        } else {
            for (i, samples) in window
                .chunks_exact(MI_SIZE)
                .enumerate()
                .take(end)
                .skip(first)
            {
                let row = ctx
                    .rows
                    .row_mut(base + i - GATHER_HALF - ctx.y_origin)
                    .ok_or(DeblockError::Workspace)?;
                row.get_mut(x..x + MI_SIZE)
                    .ok_or(DeblockError::Workspace)?
                    .copy_from_slice(samples);
            }
        }
        Ok(true)
    }
}
