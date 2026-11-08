// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! One plane pass's § 7.17.2 edge decisions, laid out for the batched
//! kernels: every edge segment the pass owes gets its [`DeblockEdge`], and the
//! kernels then filter whole columns (vertical edges) and rows (horizontal
//! edges) of them.

use splot_core::tables::conversion::{Q_THRESH_MULTS, W_MULT};
use splot_recon::{
    BitDepth, DeblockEdge, DeblockSampleFilter, ReconSample, deblock_filter_max_width,
    deblock_horizontal_edges, deblock_vertical_edge_band, deblock_vertical_edges,
};

use fearless_simd::{Simd, mask16x16, prelude::*, u8x32, u16x16};

use super::facts::{COVERED, EDGE_H, EDGE_V, LOSSLESS, PlaneFacts, SKIP, padded_stride};
use super::grid::MiGrid;
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
    /// Edges per row: `cols` padded so that a row's last lane group stores
    /// inside it.
    stride: usize,
    first_row: usize,
    /// The vertical pass's per-column lane controls, then its per-column
    /// filtered-edge marks.
    controls: Vec<u16>,
    marks: Vec<u16>,
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
const SB_EDGE: u16 = 1 << 8;
const NO_FILTER: u16 = 1 << 9;

/// Lanes one decision group holds.
const GROUP: usize = 16;

const LANE_INDEX: [u16; GROUP] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15];

/// One lane group's cells on one side of an edge: their [`PlaneFacts`].
#[derive(Clone, Copy)]
struct Side<S: Simd> {
    keys: u16x16<S>,
    info: u16x16<S>,
    qindex: u16x16<S>,
}

/// The § 7.17.5 `qThr` and `side` of the `qindex` of each lane, remembering
/// the last `qindex` met: a frame without delta-q costs two compares per
/// lane group. Each is kept to 15 bits, so that the 16-bit sum of two sides
/// cannot wrap; [`DeblockEdge::from_words`] caps both far below that.
struct Strengths<'a> {
    cache: &'a StrengthCache,
    last: (u16, [u16; 2]),
}

impl Strengths<'_> {
    /// `qThr` and `side` of the current, then the previous, side's lanes.
    #[allow(clippy::inline_always, reason = "measured deblock hot path")]
    #[inline(always)]
    fn of<S: Simd>(&mut self, simd: S, curr: u16x16<S>, prev: u16x16<S>) -> [u16x16<S>; 4] {
        let last = u16x16::splat(simd, self.last.0);
        if (curr.simd_eq(last) & prev.simd_eq(last)).all_true() {
            let [q_thr, side] = self.last.1.map(|value| u16x16::splat(simd, value));
            return [q_thr, side, q_thr, side];
        }
        self.gather(simd, [curr, prev])
    }

    #[cold]
    #[inline(never)]
    fn gather<S: Simd>(&mut self, simd: S, sides: [u16x16<S>; 2]) -> [u16x16<S>; 4] {
        let mut words = [[0; GROUP]; 4];
        for (side, qindex) in sides.iter().enumerate() {
            for (lane, qindex) in qindex.to_array().into_iter().enumerate() {
                if qindex != self.last.0 {
                    let (q_thr, side) = self.cache.get(u32::from(qindex));
                    let word = |value: i32| value.clamp(0, 0x7fff) as u16;
                    self.last = (qindex, [word(q_thr), word(side)]);
                }
                words[2 * side][lane] = self.last.1[0];
                words[2 * side + 1][lane] = self.last.1[1];
            }
        }
        words.map(|words| u16x16::from_slice(simd, &words))
    }
}

/// The § 7.17.2 decisions of sixteen edges between `prev` and `curr` cells,
/// as `qThr`, `sideThr`, `maxWidthPos | maxWidthNeg << 8` and the lossless
/// flags of each lane, zero where it does not filter, plus whether a `valid`
/// lane has an uncovered side. An edge filters when it is a transform edge,
/// its current side is coded or starts its prediction unit (`isBlockEdge ||
/// !skip`), and either side's strengths are both nonzero; its size is
/// § 7.17.4 `filterSize` clipped by `control`, its widths § 7.17.3, and its
/// thresholds the § 7.17.2 combination of both sides'.
#[allow(clippy::inline_always, reason = "measured deblock hot path")]
#[inline(always)]
fn decide_lanes<S: Simd, const PLANE: usize, const PASS: usize>(
    simd: S,
    (curr, prev): (Side<S>, Side<S>),
    [curr_q, curr_side, prev_q, prev_side]: [u16x16<S>; 4],
    control: u16x16<S>,
    valid: mask16x16<S>,
) -> ([u16x16<S>; 4], bool) {
    let splat = |value: u16| u16x16::splat(simd, value);
    let zero = splat(0);
    let set = |value: u16x16<S>, bit: u16| (value & bit).simd_eq(splat(bit));
    let uncovered = !(set(curr.info, COVERED) & set(prev.info, COVERED)) & valid;
    let block_edge = if PASS == 0 { EDGE_V } else { EDGE_H };
    let skipped = (curr.info & (block_edge | SKIP)).simd_eq(splat(SKIP));
    let strong = |q: u16x16<S>, side: u16x16<S>| !(q.simd_eq(zero) | side.simd_eq(zero));
    let apply = !(curr.keys.simd_eq(prev.keys) | skipped | set(control, NO_FILTER))
        & (strong(curr_q, curr_side) | strong(prev_q, prev_side))
        & valid;
    let code = |info: u16x16<S>| (info >> (3 * PASS as u32)) & 7;
    let size = code(curr.info).min(code(prev.info)).min(control & 0xff);
    let table = u8x32::from_slice(simd, &WIDTHS[usize::from(PLANE != 0)]);
    let index = size * 0x0101 + ((control & SB_EDGE) << 3);
    let widths = simd.swizzle_dyn_within_blocks_u8x32(table, index.bitcast::<u8x32<S>>());
    let combine = |curr: u16x16<S>, prev: u16x16<S>| {
        let sum = curr + prev;
        simd.select_u16x16(curr.min(prev).simd_eq(zero), sum, (sum + 1) >> 1)
    };
    let lossless = |info: u16x16<S>| (info & LOSSLESS) >> 7;
    let flags = lossless(prev.info) | lossless(curr.info) << 1;
    let keep = |value: u16x16<S>| simd.select_u16x16(apply, value, zero);
    (
        [
            keep(combine(curr_q, prev_q)),
            keep(combine(curr_side, prev_side)),
            keep(widths.bitcast::<u16x16<S>>()),
            keep(flags),
        ],
        uncovered.any_true(),
    )
}

/// § 7.17.3 `maxWidthPos` then `maxWidthNeg` on a superblock edge, by luma
/// or chroma and [`size_code`] filter size, as a lookup on 16-byte lanes:
/// bytes `0..8` by code, bytes `8..16` the superblock-edge negative widths.
const WIDTHS: [[u8; 32]; 2] = {
    let mut tables = [[0; 32]; 2];
    let mut entry = 0;
    while entry < 2 * 16 {
        let (chroma, byte): (usize, usize) = (entry / 16, entry % 16);
        let size = 4 << (byte % 8).saturating_sub(1);
        let (neg, pos) = deblock_filter_max_width(size, chroma == 1, true);
        let width = if byte < 8 { pos } else { neg };
        tables[chroma][byte] = width as u8;
        tables[chroma][byte + 16] = width as u8;
        entry += 1;
    }
    tables
};

/// The filter size code of a transform or clipped dimension: `log2(size) - 1`
/// for the § 7.17.4 sizes 4 to 64, which [`WIDTHS`] maps.
const fn size_code(size: u32) -> u32 {
    match size.checked_ilog2() {
        Some(log) => log.saturating_sub(1),
        None => 0,
    }
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
        self.stride = padded_stride(self.cols);
        let len = self.rows * self.stride;
        if self.edges.len() < len {
            self.edges
                .try_reserve(len - self.edges.len())
                .map_err(|_| DeblockError::Allocation {
                    plane: splot_recon::PlaneId::Y,
                    context: "deblock edge decisions",
                })?;
            self.edges.resize(len, DeblockEdge::NONE);
        }
        let lines = if PASS == 0 { self.cols } else { self.rows };
        self.live.clear();
        self.live.resize(lines, false);
        let strengths = StrengthCache::new(
            plane_pass.quant_delta,
            plane_pass.df_delta_q,
            plane_pass.bit_depth,
        );
        if !plane_pass.allow_df_sub_pu {
            let facts = grid.facts.ok_or(DeblockError::Workspace)?;
            return self
                .build_from_facts::<PLANE, PASS>(facts, plane_pass, geometry, first, &strengths);
        }
        for row_index in 0..self.rows {
            let r = first + row_index * row_step;
            let row_tile_edge = PASS == 1 && starts_tile(geometry.tile_starts, r);
            self.edges[row_index * self.stride] = DeblockEdge::NONE;
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
                self.edges[row_index * self.stride + col_index] = edge;
                self.live[line] |= !edge.is_none();
            }
        }
        Ok(())
    }

    /// [`Self::build_specialized`] for a frame without sub-PU edges, where an
    /// edge filters only across a transform edge and its decision reads only
    /// the [`PlaneFacts`] of its two sides: sixteen edges at a time.
    fn build_from_facts<const PLANE: usize, const PASS: usize>(
        &mut self,
        facts: &PlaneFacts,
        plane_pass: &PlanePass,
        geometry: &PassGeometry<'_>,
        first: usize,
        cache: &StrengthCache,
    ) -> Result<(), DeblockError> {
        let sub_x = plane_pass.plane_sub_x;
        let mut strengths = Strengths {
            cache,
            last: (u16::MAX, [0; 2]),
        };
        let (rows, cols, stride) = (self.rows, self.cols, self.stride);
        let reach = if PLANE == 0 { 16 } else { 8 };
        let disable = geometry.disable_loopfilters_across_tiles;
        let control = |near: bool, sb_edge: bool, tile: bool| {
            size_code(if near { reach as u32 } else { 64 }) as u16
                | if sb_edge || tile { SB_EDGE } else { 0 }
                | if tile && disable { NO_FILTER } else { 0 }
        };
        if PASS == 0 {
            self.controls.clear();
            self.controls.extend((0..stride).map(|col| {
                let tile = starts_tile(geometry.tile_starts, col << sub_x);
                control(col * MI_SIZE + reach > geometry.width, false, tile)
            }));
            self.marks.clear();
            self.marks.resize(stride, 0);
        }
        let offset = usize::from(PASS == 0);
        let span = cols.saturating_sub(offset).div_ceil(GROUP) * GROUP;
        baseline_simd!(simd => {
            let lane = u16x16::from_slice(simd, &LANE_INDEX);
            for row_index in 0..rows {
                let cell_row = self.first_row + row_index;
                let r = first + row_index * plane_pass.row_step;
                let uncovered = || DeblockError::UncoveredMi { row: r, col: 0 };
                let curr = facts.row(cell_row, offset, span).ok_or_else(uncovered)?;
                let prev = facts.row(cell_row - PASS, 0, span).ok_or_else(uncovered)?;
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
                let row_control = u16x16::splat(simd, row_control);
                let out = row_index * stride;
                self.edges[out] = DeblockEdge::NONE;
                let mut row_marks = u16x16::splat(simd, 0);
                let controls = if PASS == 0 {
                    self.controls.get(offset..offset + span)
                } else {
                    Some(&[][..])
                };
                let marks = if PASS == 0 {
                    self.marks.get_mut(offset..offset + span)
                } else {
                    Some(&mut [][..])
                };
                let (Some(controls), Some(marks), Some(edges)) = (
                    controls,
                    marks,
                    self.edges.get_mut(out + offset..out + offset + span),
                ) else {
                    return Err(DeblockError::Workspace);
                };
                for at in (0..span).step_by(GROUP) {
                    let lanes = at..at + GROUP;
                    let side = |[keys, info, qindex]: [&[u16]; 3]| Side {
                        keys: u16x16::from_slice(simd, &keys[lanes.clone()]),
                        info: u16x16::from_slice(simd, &info[lanes.clone()]),
                        qindex: u16x16::from_slice(simd, &qindex[lanes.clone()]),
                    };
                    let sides = (side(curr), side(prev));
                    let left = (cols - offset - at).min(GROUP) as u16;
                    let valid = lane.simd_lt(u16x16::splat(simd, left));
                    let control = if PASS == 0 {
                        u16x16::from_slice(simd, &controls[lanes.clone()])
                    } else {
                        row_control
                    };
                    if sides.0.keys.simd_eq(sides.1.keys).all_true() {
                        let covered = |info: u16x16<_>| (info & COVERED).simd_eq(u16x16::splat(simd, COVERED));
                        if (!(covered(sides.0.info) & covered(sides.1.info)) & valid).any_true() {
                            return Err(uncovered());
                        }
                        edges[lanes].fill(DeblockEdge::NONE);
                        continue;
                    }
                    let strengths = strengths.of(simd, sides.0.qindex, sides.1.qindex);
                    let (words, uncovered_lane) =
                        decide_lanes::<_, PLANE, PASS>(simd, sides, strengths, control, valid);
                    if uncovered_lane {
                        return Err(uncovered());
                    }
                    edges[lanes.clone()].copy_from_slice(&DeblockEdge::from_words(simd, words));
                    if PASS == 0 {
                        let marks = &mut marks[lanes];
                        (u16x16::from_slice(simd, marks) | words[0]).store_slice(marks);
                    } else {
                        row_marks |= words[0];
                    }
                }
                if PASS == 1 {
                    self.live[row_index] = !row_marks.simd_eq(u16x16::splat(simd, 0)).all_true();
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

    /// Filters every decided edge in § 7.17.1 order: whole columns or rows
    /// through the batched kernels where their samples 8 either side of the
    /// edge lie in the band, and the rest in runs of [`LONE_RUN`] through
    /// padded copies of their samples, or one by one.
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
        let band = if PASS == 0 {
            let columns = (GATHER_HALF / MI_SIZE).min(outer)
                ..(width.saturating_sub(GATHER_HALF) / MI_SIZE + 1).min(outer);
            let full = lines(inner, self.first_row, height);
            (if full == 0 { 0..0 } else { columns }, full)
        } else {
            (0..0, 0)
        };
        for line in 0..outer {
            if PASS == 0 && line == band.0.start && !band.0.is_empty() {
                let columns = band.0.clone();
                let (samples, stride) = ctx.rows.contiguous_mut();
                let first =
                    (self.first_row * MI_SIZE - y_origin) * stride + columns.start * MI_SIZE;
                let edges = self
                    .edges
                    .get(columns.start..)
                    .ok_or(DeblockError::Workspace)?;
                let live = self.live.get(columns).ok_or(DeblockError::Workspace)?;
                deblock_vertical_edge_band::<T>(
                    samples,
                    stride,
                    first,
                    edges,
                    (band.1, self.stride),
                    live,
                    bit_depth,
                )
                .map_err(|_| DeblockError::SampleFilter)?;
            }
            if !self.live.get(line).copied().unwrap_or(true) {
                continue;
            }
            let edge_at = |k: usize| {
                let index = if PASS == 0 {
                    k * self.stride + line
                } else {
                    line * self.stride + k
                };
                self.edges.get(index).copied().unwrap_or_default()
            };
            let (x_p, y_p) = if PASS == 0 {
                (line * MI_SIZE, self.first_row * MI_SIZE)
            } else {
                (0, (self.first_row + line) * MI_SIZE)
            };
            let full = if PASS == 0 {
                if band.0.contains(&line) { band.1 } else { 0 }
            } else {
                let fits = y_p >= y_origin + GATHER_HALF && y_p + GATHER_HALF <= height;
                let full = if fits { lines(inner, 0, width) } else { 0 };
                if full > 0 {
                    let (samples, stride) = ctx.rows.contiguous_mut();
                    let first = (y_p - y_origin) * stride + x_p;
                    let row = self
                        .edges
                        .get(line * self.stride..line * self.stride + full)
                        .ok_or(DeblockError::Workspace)?;
                    deblock_horizontal_edges::<T>(samples, stride, first, row, bit_depth)
                        .map_err(|_| DeblockError::SampleFilter)?;
                }
                full
            };
            let at = |k: usize| {
                if PASS == 0 {
                    (x_p, y_p + k * MI_SIZE)
                } else {
                    (k * MI_SIZE, y_p)
                }
            };
            for start in (full..inner).step_by(LONE_RUN) {
                let mut run = [DeblockEdge::NONE; LONE_RUN];
                let count = (inner - start).min(LONE_RUN);
                for (k, edge) in run.iter_mut().take(count).enumerate() {
                    *edge = edge_at(start + k);
                }
                let run = &run[..count];
                if run.iter().all(|edge| edge.is_none()) {
                    continue;
                }
                let (x_p, y_p) = at(start);
                let lone = LoneRun {
                    x_p,
                    y_p,
                    edges: run,
                };
                if count > 1 && lone.filter::<T, PASS>(ctx, bit_depth)? {
                    continue;
                }
                for (k, &edge) in run.iter().enumerate().filter(|(_, edge)| !edge.is_none()) {
                    let (x_p, y_p) = at(start + k);
                    LoneEdge { x_p, y_p, edge }.filter::<T, PASS>(ctx, bit_depth)?;
                }
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

/// Edges one [`LoneRun`] window holds.
pub(super) const LONE_RUN: usize = 8;

/// Samples a [`LoneRun`] window holds: four lines of `2 * GATHER_HALF` per
/// edge.
const WINDOW: usize = LONE_RUN * MI_SIZE * 2 * GATHER_HALF;

/// Up to [`LONE_RUN`] edges along one line of edge segments from plane
/// position (`x_p`, `y_p`): side by side for horizontal edges, stacked for
/// vertical ones.
#[derive(Clone, Copy)]
pub(super) struct LoneRun<'a> {
    pub(super) x_p: usize,
    pub(super) y_p: usize,
    pub(super) edges: &'a [DeblockEdge],
}

impl LoneEdge {
    /// Chooses the filter width from the plane's own samples, then filters.
    pub(super) fn filter<T: ReconSample, const PASS: usize>(
        self,
        plane_ctx: &mut PlaneCtx<'_, '_, T>,
        bit_depth: BitDepth,
    ) -> Result<(), DeblockError> {
        let run = LoneRun {
            x_p: self.x_p,
            y_p: self.y_p,
            edges: &[self.edge],
        };
        if run.filter::<T, PASS>(plane_ctx, bit_depth)? {
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
}

impl LoneRun<'_> {
    /// Filters the run with the batched kernel on a copy of its lines whose
    /// samples past the plane or band repeat its last ones, as the per-line
    /// path gathers them, then writes the copy back. Returns `false`, having
    /// written nothing, when the run has a line outside the plane or band,
    /// or more than [`LONE_RUN`] edges. A changed sample outside the plane or
    /// band is the error the per-line path gives.
    pub(super) fn filter<T: ReconSample, const PASS: usize>(
        self,
        ctx: &mut PlaneCtx<'_, '_, T>,
        bit_depth: BitDepth,
    ) -> Result<bool, DeblockError> {
        let (x, y, count) = (self.x_p, self.y_p, self.edges.len());
        let band_end = ctx.y_origin + ctx.band_rows;
        let fits = if PASS == 0 {
            y >= ctx.y_origin && y + count * MI_SIZE <= ctx.height.min(band_end)
        } else {
            x + count * MI_SIZE <= ctx.width && ctx.band_rows > 0
        };
        if !fits || ctx.width == 0 || count == 0 || count > LONE_RUN {
            return Ok(false);
        }
        let mut storage = [T::default(); WINDOW];
        let line = 2 * GATHER_HALF;
        let lines = count * MI_SIZE;
        let window = &mut storage[..lines * line];
        let (low, high) = if PASS == 0 {
            (0, ctx.width)
        } else {
            (ctx.y_origin, band_end.min(ctx.height))
        };
        let base = if PASS == 0 { x } else { y };
        let first = (low + GATHER_HALF).saturating_sub(base).min(line);
        let end = (high + GATHER_HALF).saturating_sub(base).clamp(first, line);
        if first == end {
            return Ok(false);
        }
        if PASS == 0 {
            let start = base + first - GATHER_HALF;
            for (lane, samples) in window.chunks_exact_mut(line).enumerate() {
                let row = ctx
                    .rows
                    .row(y + lane - ctx.y_origin)
                    .and_then(|row| row.get(start..start + end - first))
                    .ok_or(DeblockError::Workspace)?;
                let (head, rest) = samples.split_at_mut(first);
                let (middle, tail) = rest.split_at_mut(end - first);
                middle.copy_from_slice(row); // splot-copy-ok: stage a lone run's window
                head.fill(middle[0]);
                tail.fill(middle[middle.len() - 1]);
            }
        } else {
            for (i, samples) in window.chunks_exact_mut(lines).enumerate() {
                let row = base + i.clamp(first, end - 1) - GATHER_HALF - ctx.y_origin;
                let row = ctx.rows.row(row).ok_or(DeblockError::Workspace)?;
                samples.copy_from_slice(row.get(x..x + lines).ok_or(DeblockError::Workspace)?);
            }
        }
        let mut original = [T::default(); WINDOW];
        let original = &mut original[..lines * line];
        original.copy_from_slice(window); // splot-copy-ok: keep the gathered window to check
        let filtered = if PASS == 0 {
            deblock_vertical_edges(window, line, GATHER_HALF, self.edges, bit_depth)
        } else {
            deblock_horizontal_edges(window, lines, GATHER_HALF * lines, self.edges, bit_depth)
        };
        filtered.map_err(|_| DeblockError::SampleFilter)?;
        let changed = |range: core::ops::Range<usize>| {
            let (new, old) = (window.get(range.clone()), original.get(range));
            new.zip(old).is_some_and(|(new, old)| {
                new.iter()
                    .zip(old)
                    .any(|(new, old)| new.to_u16() != old.to_u16())
            })
        };
        let changed_outside = if PASS == 0 {
            (0..lines).any(|lane| {
                changed(lane * line..lane * line + first)
                    || changed(lane * line + end..(lane + 1) * line)
            })
        } else {
            changed(0..first * lines) || changed(end * lines..line * lines)
        };
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
            for (i, samples) in window.chunks_exact(lines).enumerate().take(end).skip(first) {
                let row = ctx
                    .rows
                    .row_mut(base + i - GATHER_HALF - ctx.y_origin)
                    .ok_or(DeblockError::Workspace)?;
                row.get_mut(x..x + lines)
                    .ok_or(DeblockError::Workspace)?
                    .copy_from_slice(samples);
            }
        }
        Ok(true)
    }
}
