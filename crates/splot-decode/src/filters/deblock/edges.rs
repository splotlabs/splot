// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! One plane pass's § 7.17.2 edge decisions, laid out for the batched
//! kernels: every edge segment the pass owes gets its [`DeblockEdge`], and the
//! kernels then filter whole columns (vertical edges) and rows (horizontal
//! edges) of them.

use core::num::NonZeroUsize;

use splot_core::tables::conversion::{Q_THRESH_MULTS, W_MULT};
use splot_recon::{
    BitDepth, DeblockEdge, DeblockSampleFilter, ReconSample, deblock_horizontal_edges,
    deblock_vertical_edge_column, deblock_vertical_edges,
};

use fearless_simd::{Simd, prelude::*, u32x8};

use super::grid::{EDGE_H, EDGE_V, LOSSLESS, MiGrid, QINDEX_SHIFT, SKIP, UNCOVERED};
use super::{
    DeblockError, EdgeContext, GATHER_HALF, MAX_WIDTHS, MI_SIZE, PerpLine, PlaneCtx, PlanePass,
    SB_SIZE, StrengthCache, apply_edge_samples, choose_filter_width, edge_blocks, edge_decision,
    starts_tile,
};

/// The edges of one plane pass over the mode-info rows it owes, row-major by
/// plane 4x4 cell; a vertical-pass column is read at a stride of one row.
#[derive(Default)]
pub(crate) struct PassEdges {
    edges: Vec<DeblockEdge>,
    rows: usize,
    cols: usize,
    first_row: usize,
    /// The vertical pass's per-column lane controls, then its per-column
    /// filtered-edge marks.
    controls: Vec<u32>,
    marks: Vec<u32>,
    /// Whether each column (vertical pass) or row (horizontal pass) holds
    /// an edge to filter.
    live: Vec<bool>,
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

/// Lane control bits past the 8-bit filter size limit: the edge is a
/// § 7.17.2 `sbEdge`, or one no filtering crosses.
const SB_EDGE: u32 = 1 << 8;
const NO_FILTER: u32 = 1 << 9;

/// One lane group's cells across an edge: transform bases, info words and
/// the § 7.17.5 strengths as `qThr | side << 16`.
#[derive(Clone, Copy)]
struct Lanes<S: Simd> {
    bases: u32x8<S>,
    info: u32x8<S>,
    strengths: u32x8<S>,
}

/// The packed § 7.17.5 strengths of the `qindex` of each lane's info word,
/// remembering the last `qindex` met: a frame without delta-q costs one
/// compare per lane group.
struct Strengths<'a> {
    cache: &'a StrengthCache,
    last: (u32, u32),
}

impl Strengths<'_> {
    #[allow(clippy::inline_always, reason = "measured deblock hot path")]
    #[inline(always)]
    fn of<S: Simd>(&mut self, simd: S, info: u32x8<S>) -> u32x8<S> {
        let qindex = info >> QINDEX_SHIFT;
        if qindex.simd_eq(u32x8::splat(simd, self.last.0)).all_true() {
            return u32x8::splat(simd, self.last.1);
        }
        self.gather(simd, qindex)
    }

    #[inline(never)]
    fn gather<S: Simd>(&mut self, simd: S, qindex: u32x8<S>) -> u32x8<S> {
        let words = qindex.to_array().map(|qindex| {
            if qindex != self.last.0 {
                let (q_thr, side) = self.cache.get(qindex);
                let word = q_thr.clamp(0, 0xffff) as u32 | (side.clamp(0, 0xffff) as u32) << 16;
                self.last = (qindex, word);
            }
            self.last.1
        });
        u32x8::from_slice(simd, &words)
    }
}

/// The eight cells from `at`.
#[allow(clippy::inline_always, reason = "measured deblock hot path")]
#[inline(always)]
fn load<S: Simd>(simd: S, cells: &[u32], at: usize) -> u32x8<S> {
    match cells.get(at..at + 8) {
        Some(cells) => u32x8::from_slice(simd, cells),
        None => pad(simd, cells, at),
    }
}

/// The cells from `at` padded with zero past the row: a zero base on both
/// sides of an edge is no transform edge.
#[cold]
fn pad<S: Simd>(simd: S, cells: &[u32], at: usize) -> u32x8<S> {
    let mut lanes = [0; 8];
    for (lane, &cell) in lanes.iter_mut().zip(cells.get(at..).unwrap_or_default()) {
        *lane = cell;
    }
    u32x8::from_slice(simd, &lanes)
}

/// The § 7.17.2 decisions of eight edges between `prev` and `curr` cells,
/// as the low and high words [`DeblockEdge::from_lanes`] packs, plus the
/// lanes where either side is uncovered. An edge filters when it is a
/// transform edge, its current side is coded or starts its prediction unit
/// (`isBlockEdge || !skip`), and either side's strengths are both nonzero;
/// its size is § 7.17.4 `filterSize` clipped by `control`, its widths
/// § 7.17.3, and its thresholds the § 7.17.2 combination of both sides'.
#[allow(clippy::inline_always, reason = "measured deblock hot path")]
#[inline(always)]
fn decide_lanes<S: Simd, const PLANE: usize, const PASS: usize>(
    simd: S,
    curr: Lanes<S>,
    prev: Lanes<S>,
    control: u32x8<S>,
) -> (u32x8<S>, u32x8<S>, bool) {
    let splat = |value: u32| u32x8::splat(simd, value);
    let chroma = PLANE != 0;
    let zero = splat(0);
    let set = |value: u32x8<S>, bit: u32| (value & bit).simd_eq(splat(bit));
    let uncovered = curr.bases.simd_eq(splat(UNCOVERED)) | prev.bases.simd_eq(splat(UNCOVERED));
    let block_edge = if PASS == 0 { EDGE_V } else { EDGE_H };
    let skipped = (curr.info & (block_edge | SKIP)).simd_eq(splat(SKIP));
    let strong = |strengths: u32x8<S>| {
        !((strengths & 0xffff).simd_eq(zero) | (strengths >> 16).simd_eq(zero))
    };
    let apply = !(curr.bases.simd_eq(prev.bases) | skipped | set(control, NO_FILTER))
        & (strong(curr.strengths) | strong(prev.strengths));
    let size = ((curr.info >> (8 * PASS as u32)) & 0xff)
        .min((prev.info >> (8 * PASS as u32)) & 0xff)
        .min(control & 0xff);
    let pick = |mask, yes: u32, no: u32x8<S>| simd.select_u32x8(mask, splat(yes), no);
    let wide = pick(
        size.simd_eq(splat(16)),
        if chroma { 4 } else { 6 },
        splat(if chroma { 4 } else { 8 }),
    );
    let pos = pick(
        size.simd_le(splat(4)),
        1,
        pick(size.simd_eq(splat(8)), 3, wide),
    );
    let cap = pos.min(splat(if chroma { 2 } else { 6 }));
    let neg = simd.select_u32x8(set(control, SB_EDGE), cap, pos);
    let combine = |curr: u32x8<S>, prev: u32x8<S>| {
        let both = !(curr.simd_eq(zero) | prev.simd_eq(zero));
        simd.select_u32x8(both, (curr + prev + 1) >> 1, curr.max(prev))
    };
    let q_thr = combine(curr.strengths & 0xffff, prev.strengths & 0xffff);
    let side = combine(curr.strengths >> 16, prev.strengths >> 16);
    let lossless = |info: u32x8<S>| (info & LOSSLESS) >> 17;
    let low = q_thr.min(splat(0xffff)) | side.min(splat(0xffff)) << 16;
    let high = pos | neg << 8 | lossless(prev.info) << 16 | lossless(curr.info) << 17;
    (
        simd.select_u32x8(apply, low, zero),
        simd.select_u32x8(apply, high, zero),
        uncovered.any_true(),
    )
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
        let lines = if PASS == 0 { self.cols } else { self.rows };
        self.live.clear();
        self.live.resize(lines, false);
        let strengths = StrengthCache::new(
            plane_pass.quant_delta,
            plane_pass.df_delta_q,
            plane_pass.bit_depth,
        );
        if !plane_pass.allow_df_sub_pu {
            return self
                .build_from_cells::<PLANE, PASS>(grid, plane_pass, geometry, first, &strengths);
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
                let line = if PASS == 0 { col_index } else { row_index };
                self.edges[row_index * self.cols + col_index] = edge;
                self.live[line] |= !edge.is_none();
            }
        }
        Ok(())
    }

    /// [`Self::build_specialized`] for a frame without sub-PU edges, where an
    /// edge filters only across a transform edge and its decision reads only
    /// the grid's per-cell arrays of its two sides: eight edges at a time.
    fn build_from_cells<const PLANE: usize, const PASS: usize>(
        &mut self,
        grid: &MiGrid<'_>,
        plane_pass: &PlanePass,
        geometry: &PassGeometry<'_>,
        first: usize,
        cache: &StrengthCache,
    ) -> Result<(), DeblockError> {
        let sub_x = plane_pass.plane_sub_x;
        let mut strengths = Strengths {
            cache,
            last: (u32::MAX, 0),
        };
        let (rows, cols) = (self.rows, self.cols);
        let reach = if PLANE == 0 { 16 } else { 8 };
        let disable = geometry.disable_loopfilters_across_tiles;
        let control = |near: bool, sb_edge: bool, tile: bool| {
            (if near { reach as u32 } else { 64 })
                | if sb_edge || tile { SB_EDGE } else { 0 }
                | if tile && disable { NO_FILTER } else { 0 }
        };
        if PASS == 0 {
            self.controls.clear();
            self.controls.extend((0..cols).map(|col| {
                let tile = starts_tile(geometry.tile_starts, col << sub_x);
                control(col * MI_SIZE + reach > geometry.width, false, tile)
            }));
            self.marks.clear();
            self.marks.resize(cols, 0);
        }
        let offset = usize::from(PASS == 0);
        baseline_simd!(simd => {
            for row_index in 0..rows {
                let cell_row = self.first_row + row_index;
                let r = first + row_index * plane_pass.row_step;
                let uncovered = || DeblockError::UncoveredMi { row: r, col: 0 };
                let row = |at: usize| grid.cell_row(at).ok_or_else(uncovered);
                let (curr, prev) = if PASS == 0 {
                    let cells = row(cell_row)?;
                    let head = cols.saturating_sub(1);
                    (
                        cells.map(|cells| cells.get(1..).unwrap_or_default()),
                        cells.map(|cells| cells.get(..head).unwrap_or_default()),
                    )
                } else {
                    (row(cell_row)?, row(cell_row - 1)?)
                };
                let row_control = if PASS == 1 {
                    let tile = starts_tile(geometry.tile_starts, r);
                    control(
                        cell_row * MI_SIZE + reach > geometry.height,
                        (r * MI_SIZE).is_multiple_of(SB_SIZE),
                        false,
                    ) | if tile && disable { NO_FILTER } else { 0 }
                } else {
                    0
                };
                let row_control = u32x8::splat(simd, row_control);
                let lanes = curr[0].len();
                let out = row_index * cols + offset;
                let mut row_marks = u32x8::splat(simd, 0);
                for at in (0..lanes).step_by(8) {
                    let control = if PASS == 0 {
                        load(simd, self.controls.get(offset..).unwrap_or_default(), at)
                    } else {
                        row_control
                    };
                    let (curr_bases, prev_bases) = (load(simd, curr[0], at), load(simd, prev[0], at));
                    if curr_bases.simd_eq(prev_bases).all_true() {
                        if curr_bases.simd_eq(u32x8::splat(simd, UNCOVERED)).any_true() {
                            return Err(uncovered());
                        }
                        continue;
                    }
                    let (curr_info, prev_info) = (load(simd, curr[1], at), load(simd, prev[1], at));
                    let curr = Lanes {
                        bases: curr_bases,
                        info: curr_info,
                        strengths: strengths.of(simd, curr_info),
                    };
                    let prev = Lanes {
                        bases: prev_bases,
                        info: prev_info,
                        strengths: strengths.of(simd, prev_info),
                    };
                    let (low, high, uncovered_lane) =
                        decide_lanes::<_, PLANE, PASS>(simd, curr, prev, control);
                    if uncovered_lane {
                        return Err(uncovered());
                    }
                    let edges = DeblockEdge::from_lanes(simd, low, high);
                    let count = (lanes - at).min(8);
                    let out = self
                        .edges
                        .get_mut(out + at..out + at + count)
                        .ok_or(DeblockError::Workspace)?;
                    if let Ok(out) = <&mut [DeblockEdge; 8]>::try_from(&mut *out) {
                        *out = edges;
                    } else {
                        for (out, &edge) in out.iter_mut().zip(&edges) {
                            *out = edge;
                        }
                    }
                    if PASS == 0 {
                        let marks = self.marks.get_mut(offset..).unwrap_or_default();
                        let merged = load(simd, marks, at) | low;
                        match marks.get_mut(at..at + 8) {
                            Some(marks) => merged.store_slice(marks),
                            None => {
                                for (mark, value) in marks.iter_mut().skip(at).zip(merged.to_array()) {
                                    *mark = value;
                                }
                            }
                        }
                    } else {
                        row_marks |= low;
                    }
                }
                if PASS == 1 {
                    self.live[row_index] = !row_marks.simd_eq(u32x8::splat(simd, 0)).all_true();
                }
            }
            Ok::<(), DeblockError>(())
        })?;
        if PASS == 0 {
            for (live, &mark) in self.live.iter_mut().zip(&self.marks) {
                *live = mark != 0;
            }
        }
        Ok(())
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
            count.min((limit / MI_SIZE).saturating_sub(first))
        };
        let (outer, inner) = if PASS == 0 {
            (self.cols, self.rows)
        } else {
            (self.rows, self.cols)
        };
        let cols = NonZeroUsize::new(self.cols).ok_or(DeblockError::Workspace)?;
        for line in 0..outer {
            if !self.live.get(line).copied().unwrap_or(true) {
                continue;
            }
            let edge_at = |k: usize| {
                let index = if PASS == 0 {
                    k * self.cols + line
                } else {
                    line * self.cols + k
                };
                self.edges.get(index).copied().unwrap_or_default()
            };
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
                let filtered = if PASS == 0 {
                    let column = self
                        .edges
                        .get(line..line + (full - 1) * self.cols + 1)
                        .ok_or(DeblockError::Workspace)?;
                    deblock_vertical_edge_column::<T>(
                        samples, stride, first, column, cols, bit_depth,
                    )
                } else {
                    let row = self
                        .edges
                        .get(line * self.cols..line * self.cols + full)
                        .ok_or(DeblockError::Workspace)?;
                    deblock_horizontal_edges::<T>(samples, stride, first, row, bit_depth)
                };
                filtered.map_err(|_| DeblockError::SampleFilter)?;
            }
            for k in full..inner {
                let edge = edge_at(k);
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
