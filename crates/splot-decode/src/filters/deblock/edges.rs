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
    SB_SIZE, StrengthCache, apply_edge_samples, choose_filter_width, edge_blocks, edge_decision,
    starts_tile,
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
    /// The previous and current cell rows of a pass without sub-PU edges.
    sides: [SideRow; 2],
    /// One row of vertical-pass edges before they go to their columns.
    line: Vec<DeblockEdge>,
}

/// One cell row's [`CellFacts`] across one pass, with the cells'
/// strengths, laid out field by field.
#[derive(Default)]
struct SideRow {
    tx_base: Vec<u32>,
    /// The prediction unit's origin across the pass, in plane samples.
    origin: Vec<u16>,
    /// The transform size across the pass.
    size: Vec<u8>,
    flags: Vec<u8>,
    q_thr: Vec<u16>,
    side: Vec<u16>,
}

/// One cell of a [`SideRow`].
#[derive(Clone, Copy)]
struct Side {
    tx_base: u32,
    origin: u16,
    size: u8,
    flags: u8,
    q_thr: u16,
    side: u16,
}

impl SideRow {
    /// Lays `facts` out across pass `PASS`, failing on an uncovered cell.
    fn load<const PASS: usize>(
        &mut self,
        facts: &[CellFacts],
        strength: &mut impl FnMut(u16) -> (i32, i32),
    ) -> Result<(), DeblockError> {
        self.tx_base.clear();
        self.origin.clear();
        self.size.clear();
        self.flags.clear();
        self.q_thr.clear();
        self.side.clear();
        let mut covered = CellFacts::COVERED;
        for facts in facts {
            let (q_thr, side) = strength(facts.qindex);
            self.tx_base.push(facts.tx_base);
            self.origin.push(facts.prediction[PASS]);
            self.size.push(facts.tx_size[PASS]);
            self.flags.push(facts.flags);
            self.q_thr.push(q_thr.clamp(0, i32::from(u16::MAX)) as u16);
            self.side.push(side.clamp(0, i32::from(u16::MAX)) as u16);
            covered &= facts.flags;
        }
        if covered == 0 {
            let col = facts
                .iter()
                .position(|facts| facts.flags & CellFacts::COVERED == 0)
                .unwrap_or(0);
            return Err(DeblockError::UncoveredMi { row: 0, col });
        }
        Ok(())
    }

    fn side(&self, col: usize) -> Side {
        Side {
            tx_base: self.tx_base[col],
            origin: self.origin[col],
            size: self.size[col],
            flags: self.flags[col],
            q_thr: self.q_thr[col],
            side: self.side[col],
        }
    }
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

/// The § 7.17.2 decision of the edge between `prev` and `curr` at plane
/// sample position `at` across it: filtered when it is a transform edge, the
/// current side is coded or starts its prediction unit, and either side's
/// strengths are both nonzero.
#[allow(clippy::inline_always, reason = "measured deblock hot path")]
#[allow(clippy::needless_bitwise_bool, reason = "branch-free edge decisions")]
#[inline(always)]
fn decide(curr: Side, prev: Side, at: usize, limit: u8, shapes: &[DeblockEdge; 65]) -> DeblockEdge {
    let coded = (usize::from(curr.origin) == at) | (curr.flags & CellFacts::SKIP == 0);
    let strong = |side: Side| (side.q_thr != 0) & (side.side != 0);
    let apply = (curr.tx_base != prev.tx_base) & coded & (strong(curr) | strong(prev));
    let size = curr.size.min(prev.size).min(limit).min(64);
    let mean = |a: u16, b: u16| {
        if (a != 0) & (b != 0) {
            ((u32::from(a) + u32::from(b) + 1) >> 1) as u16
        } else {
            a.max(b)
        }
    };
    let edge = shapes[usize::from(size)]
        .with_lossless(
            prev.flags & CellFacts::LOSSLESS != 0,
            curr.flags & CellFacts::LOSSLESS != 0,
        )
        .with_thresholds(mean(curr.q_thr, prev.q_thr), mean(curr.side, prev.side));
    core::hint::select_unpredictable(apply, edge, DeblockEdge::NONE)
}

/// Decides every edge of one cell row into `out`: between horizontal
/// neighbours of `curr` (vertical pass, `prev` absent) from column 1 on, or
/// between `curr` and the row `prev` above it, at plane row `y_p`.
fn decide_row(
    curr: &SideRow,
    prev: Option<(&SideRow, usize)>,
    limit: u8,
    shapes: &[DeblockEdge; 65],
    out: &mut [DeblockEdge],
) {
    match prev {
        Some((prev, y_p)) => {
            for (col, out) in out.iter_mut().enumerate() {
                *out = decide(curr.side(col), prev.side(col), y_p, limit, shapes);
            }
        }
        None => {
            for (col, out) in out.iter_mut().enumerate().skip(1) {
                let at = col * MI_SIZE;
                *out = decide(curr.side(col), curr.side(col - 1), at, limit, shapes);
            }
        }
    }
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
    /// edge's decision reads only the [`CellFacts`] of its two sides, which
    /// the grid resolved once per cell. Each row of cells is first laid out
    /// as one [`SideRow`], and every edge of the row then decides without a
    /// branch; the few edges at a tile start or near the plane's far edge,
    /// whose widths their position changes, decide again on their own.
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
        let mut sides = core::mem::take(&mut self.sides);
        let mut line = core::mem::take(&mut self.line);
        let mut last = (u16::MAX, (0, 0));
        let mut strength = |qindex: u16| {
            if qindex != last.0 {
                last = (qindex, strengths.get(qindex.into()));
            }
            last.1
        };
        let mut load = |side: &mut SideRow, row: usize| {
            let facts = grid
                .facts_row(row)
                .filter(|facts| facts.len() >= cols)
                .ok_or(DeblockError::UncoveredMi { row, col: 0 })?;
            side.load::<PASS>(&facts[..cols], &mut strength)
        };
        let result = (|| {
            if PASS == 1 {
                load(&mut sides[0], self.first_row - 1)?;
            }
            for row_index in 0..rows {
                let cell_row = self.first_row + row_index;
                let r = first + row_index * plane_pass.row_step;
                let y_p = cell_row * MI_SIZE;
                let (prev, curr) = sides.split_at_mut(1);
                let (prev, curr) = (&mut prev[0], &mut curr[0]);
                load(curr, cell_row)?;
                let tile_row = PASS == 1 && starts_tile(geometry.tile_starts, r);
                let sb_row = PASS == 1 && (r * MI_SIZE).is_multiple_of(SB_SIZE);
                let limit = if PASS == 1 && y_p + reach > geometry.height {
                    reach as u8
                } else {
                    64
                };
                let shapes = &shapes[usize::from(sb_row)];
                if PASS == 1 {
                    let out = &mut self.edges[row_index * cols..(row_index + 1) * cols];
                    decide_row(curr, Some((&*prev, y_p)), limit, shapes, out);
                    if geometry.disable_loopfilters_across_tiles && tile_row {
                        out.fill(DeblockEdge::NONE);
                    }
                } else {
                    line.clear();
                    line.resize(cols, DeblockEdge::NONE);
                    decide_row(curr, None, limit, shapes, &mut line);
                    let tail = geometry
                        .width
                        .checked_sub(reach)
                        .map_or(0, |width| width / MI_SIZE + 1)
                        .max(1)
                        .min(cols);
                    let tiles = geometry.tile_starts.unwrap_or_default().iter();
                    let tile_cols = tiles.map(|&col| col as usize >> sub_x);
                    for col in tile_cols.chain(tail..cols) {
                        if col == 0 || col >= cols {
                            continue;
                        }
                        let tile = starts_tile(geometry.tile_starts, col << sub_x);
                        let near = col * MI_SIZE + reach > geometry.width;
                        let limit = if near { reach as u8 } else { 64 };
                        let shapes = &SHAPES[usize::from(PLANE != 0)][usize::from(tile)];
                        let at = col * MI_SIZE;
                        let edge = decide(curr.side(col), curr.side(col - 1), at, limit, shapes);
                        line[col] = if tile && geometry.disable_loopfilters_across_tiles {
                            DeblockEdge::NONE
                        } else {
                            edge
                        };
                    }
                    for (col, &edge) in line.iter().enumerate() {
                        self.edges[col * rows + row_index] = edge;
                    }
                }
                core::mem::swap(prev, curr);
            }
            Ok(())
        })();
        self.sides = sides;
        self.line = line;
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
