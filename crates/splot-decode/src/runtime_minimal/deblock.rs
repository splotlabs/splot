// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 § 7.17 deblocking-filter orchestration for the general intra decode path.
//!
//! This is the scheduler over the `splot-recon` per-edge deblocking primitives
//! ([`deblock_filter_choice`], [`deblock_sample_filter`], [`deblock_filter_max_width`],
//! [`deblock_adaptive_filter_strength`], [`deblock_side_threshold_index`]): it
//! derives the per-(plane, pass) filter LEVEL (§ 7.17.6), the (qThr, side)
//! strengths (§ 7.17.5), iterates the § 7.17.1 / § 7.17.2 plane × pass × MI edge
//! loop over the decoded block grid, gathers each perpendicular sample line from
//! the [`CurrentFrameWorkspace`], and applies the filter IN PLACE after the block
//! walk and before `workspace.freeze()`.
//!
//! Verified subset: the general intra frontier admits intra key frames whose
//! `df_delta_q` is all zero, `allow_df_sub_pu` is `0` (always for a key frame, so
//! `isSubPuEdge == 0`), segmentation disabled (so `LosslessArray` is all-false and
//! `SegmentIds`/`ChromaSegmentIds` are `0`), and the single 4:2:0 tile. The chroma
//! per-plane base offsets resolve to zero (the frontier gate requires it). This
//! module is bit-exact vs avmdec for `syn-2sb-deblock-intra-128x64-q100.ivf`.
//!
//! Feature tracking: `DECODE-GENERAL-INTRA-DEBLOCK`.

use splot_core::tables::conversion::{
    Q_FIRST, Q_THRESH_MULTS, SIDE_THRESHOLDS, TX_HEIGHT, TX_WIDTH, W_MULT,
};
use splot_recon::{
    BitDepth, CurrentFrameWorkspace, DeblockFilterChoice, DeblockSampleFilter, PlaneId,
    ReconSample, deblock_adaptive_filter_strength, deblock_filter_choice, deblock_filter_max_width,
    deblock_sample_filter, deblock_side_threshold_index,
};

/// AV2 § 3 `MI_SIZE`: the side of one mode-info unit in luma samples.
const MI_SIZE: usize = 4;

/// One decoded leaf block's deblocking-relevant geometry, recorded during the
/// § 5.20.3.1 partition walk. `r` / `c` are the luma MI position of the block's
/// top-left; `n4w` / `n4h` are its width / height in luma 4x4 units; `luma_tx` /
/// `chroma_tx` are the § 9.2 `TX_SIZES_ALL` indices of the single (TX_MODE_LARGEST)
/// luma / 4:2:0-chroma transform spanning the block.
#[derive(Clone, Copy, Debug)]
pub(crate) struct DeblockBlock {
    /// Luma MI row of the block's top-left (`MiRowBase`).
    pub(crate) r: usize,
    /// Luma MI col of the block's top-left (`MiColBase`).
    pub(crate) c: usize,
    /// Block width in luma 4x4 units (`Num_4x4_Blocks_Wide[MiSize]`).
    pub(crate) n4w: usize,
    /// Block height in luma 4x4 units (`Num_4x4_Blocks_High[MiSize]`).
    pub(crate) n4h: usize,
    /// § 9.2 `TX_SIZES_ALL` index of the luma transform (DeblockingTxSizes[0]).
    pub(crate) luma_tx: usize,
    /// § 9.2 `TX_SIZES_ALL` index of the 4:2:0 chroma transform
    /// (DeblockingTxSizes[1]/[2]), or `None` for a luma-only block.
    pub(crate) chroma_tx: Option<usize>,
}

/// Per-plane deblocking metadata for one MI position, resolved from the covering
/// decoded block. The luma plane reads the luma grid (full resolution); a chroma
/// plane reads the same grid but with the § 7.17.2 chroma "bottom-right mode info"
/// and subsampling adjustments applied by the caller.
#[derive(Clone, Copy)]
struct MiBlockInfo {
    /// `MiRowBase` (luma MI) of the covering block.
    base_row: usize,
    /// `MiColBase` (luma MI) of the covering block.
    base_col: usize,
    /// Luma transform size index.
    luma_tx: usize,
    /// Chroma transform size index, if the block has chroma.
    chroma_tx: Option<usize>,
}

/// AV2 § 7.17.1 / § 7.17.2 deblocking-filter orchestration over the decoded
/// general intra block grid, applied in place to `workspace`.
///
/// `blocks` are the decoded leaf blocks (their union tiles the frame); `mi_rows`
/// / `mi_cols` are the frame MI dimensions; `apply` is the parsed
/// `apply_deblocking_filter[0..4]` gate; `base_q_idx` is the frame quantizer
/// (segmentation disabled, no delta-Q, `df_delta_q` all zero → every plane's
/// § 7.17.6 `lvl` equals `q_clamped(base_q_idx, 0)`); `bit_depth` is the active
/// decoded bit depth.
///
/// Returns `Err` only on an internal inconsistency (a block grid that does not
/// cover the frame, or a workspace read/write out of bounds); for the verified
/// subset it is total.
pub(crate) fn deblock_general_intra_frame<T: ReconSample>(
    workspace: &mut CurrentFrameWorkspace<T>,
    blocks: &[DeblockBlock],
    mi_rows: usize,
    mi_cols: usize,
    apply: [bool; 4],
    base_q_idx: u32,
    bit_depth: BitDepth,
) -> Result<(), DeblockError> {
    // Nothing to do if no pass is enabled.
    if apply == [false; 4] {
        return Ok(());
    }

    // Build the luma-MI-indexed covering-block grid. Every MI position the loop
    // visits must be covered by exactly one decoded block; an uncovered position
    // is an internal inconsistency.
    let grid = build_mi_grid(blocks, mi_rows, mi_cols)?;

    // 4:2:0 subsampling (the general intra frontier is 4:2:0 only).
    let sub_x = 1usize;
    let sub_y = 1usize;
    // The frontier admits 4:2:0 with chroma planes, so NumPlanes == 3.
    let num_planes = 3usize;

    for plane in 0..num_planes {
        for pass in 0..2usize {
            let apply_index = if plane == 0 { pass } else { plane + 1 };
            if !apply[apply_index] {
                continue;
            }
            let (plane_sub_x, plane_sub_y) = if plane == 0 { (0, 0) } else { (sub_x, sub_y) };
            let row_step = if plane == 0 { 1 } else { 1 << sub_y };
            let col_step = if plane == 0 { 1 } else { 1 << sub_x };

            // § 7.17.6 filter LEVEL for this (plane, pass): segmentation disabled
            // and `df_delta_q` all zero make `lvl = q_clamped(base_q_idx, 0)` for
            // every plane (the chroma base offsets resolve to zero in the admitted
            // subset). `q_clamped(qindex, 0)` is `0` iff `qindex == 0`, else
            // `Clip3(1, MaxQ, qindex)`; `base_q_idx` is already in `0..=MaxQ`.
            let lvl = q_clamped_zero_delta(base_q_idx);
            // § 7.17.5 strengths from the level (same for curr and prev: every
            // block shares the frame quantizer and the segment is 0).
            let (q_thr, side) = adaptive_strength(lvl, bit_depth);

            let plane_id = plane_index_to_id(plane);
            let mut r = 0usize;
            while r < mi_rows {
                let mut c = 0usize;
                while c < mi_cols {
                    deblock_filter_edge(
                        workspace,
                        &grid,
                        EdgeContext {
                            plane,
                            plane_id,
                            pass,
                            row: r,
                            col: c,
                            mi_rows,
                            mi_cols,
                            plane_sub_x,
                            plane_sub_y,
                            q_thr,
                            side,
                            bit_depth,
                        },
                    )?;
                    c += col_step;
                }
                r += row_step;
            }
        }
    }

    Ok(())
}

/// Inputs to the § 7.17.2 edge deblocking filter process for one MI edge.
struct EdgeContext {
    plane: usize,
    plane_id: PlaneId,
    pass: usize,
    row: usize,
    col: usize,
    mi_rows: usize,
    mi_cols: usize,
    plane_sub_x: usize,
    plane_sub_y: usize,
    q_thr: i32,
    side: i32,
    bit_depth: BitDepth,
}

/// AV2 § 7.17.2 edge deblocking filter process for one (plane, pass, row, col).
fn deblock_filter_edge<T: ReconSample>(
    workspace: &mut CurrentFrameWorkspace<T>,
    grid: &MiGrid,
    ctx: EdgeContext,
) -> Result<(), DeblockError> {
    let EdgeContext {
        plane,
        plane_id,
        pass,
        row,
        col,
        mi_rows,
        mi_cols,
        plane_sub_x,
        plane_sub_y,
        q_thr: curr_q,
        side: curr_side,
        bit_depth,
    } = ctx;

    // § 7.17.2: (dx, dy) is the filter direction (perpendicular to the edge).
    let (dx, dy) = if pass == 0 { (1usize, 0usize) } else { (0, 1) };

    let x = col * MI_SIZE;
    let y = row * MI_SIZE;

    // sbEdge: a horizontal edge on the 64x64 grid, or a vertical tile edge. The
    // single full-frame tile has only one tile, so the tile-edge terms reduce to
    // the frame edge (x == 0 / y == 0), which `onScreen` already drops. So sbEdge
    // is the horizontal 64-grid edge only.
    let sb_edge = pass == 1 && y.is_multiple_of(64);

    // onScreen: drop the leading frame edge (no previous samples there).
    let on_screen = !((pass == 0 && x == 0) || (pass == 1 && y == 0));
    if !on_screen {
        return Ok(());
    }

    let x_p = x >> plane_sub_x;
    let y_p = y >> plane_sub_y;

    // prevRow / prevCol: the MI block on the other side of the boundary.
    let prev_row = row - (dy << plane_sub_y);
    let prev_col = col - (dx << plane_sub_x);

    // Resolve the covering blocks (luma-MI indexed). The grid is built so every
    // visited MI is covered.
    let curr = grid
        .get(row, col)
        .ok_or(DeblockError::UncoveredMi { row, col })?;
    let prev = grid
        .get(prev_row, prev_col)
        .ok_or(DeblockError::UncoveredMi {
            row: prev_row,
            col: prev_col,
        })?;

    // baseRow / baseCol / baseY / baseX of the current block.
    let base_row = curr.base_row;
    let base_col = curr.base_col;
    let base_y = (base_row * MI_SIZE) >> plane_sub_y;
    let base_x = (base_col * MI_SIZE) >> plane_sub_x;

    // txSz / prevTxSz from DeblockingTxSizes[plane].
    let tx_sz = plane_tx(plane, curr).ok_or(DeblockError::MissingTx { plane })?;
    let prev_tx_sz = plane_tx(plane, prev).ok_or(DeblockError::MissingTx { plane })?;

    // TxColBase / TxRowBase: for TX_MODE_LARGEST a block holds a single transform,
    // so the transform base is the block base (in plane MI units).
    let tx_col_base = base_col >> plane_sub_x;
    let tx_row_base = base_row >> plane_sub_y;
    let prev_tx_col_base = prev.base_col >> plane_sub_x;
    let prev_tx_row_base = prev.base_row >> plane_sub_y;

    // § 7.17.2 chroma adjustment: chroma info is held in the bottom-right mode
    // info. This only shifts `row` / `col` used for the `Skips` lookup; the
    // verified subset has `skip == 0` everywhere (intra residual blocks; chroma
    // `skip` is forced to 0 for INTRA_REGION / FrameIsIntra). isSubPuEdge == 0
    // (`allow_df_sub_pu == 0` for the intra key frame).
    let skip = false;
    let is_sub_pu_edge = false;

    let x_r = x_p - base_x;
    let y_r = y_p - base_y;
    let is_block_edge = (pass == 0 && x_r == 0) || (pass == 1 && y_r == 0);
    let is_tx_edge = tx_col_base != prev_tx_col_base || tx_row_base != prev_tx_row_base;

    // § 7.17.2 applyFilter cascade (with isSubPuEdge == 0). The spec's three
    // early-return-false rungs combine to: filter iff the edge crosses a tx (or
    // sub-PU) boundary AND at least one side has nonzero strength AND it is a
    // block edge / non-skip / sub-PU edge.
    let curr_strong = curr_q != 0 && curr_side != 0;
    // prev strengths equal curr strengths in the admitted subset (same lvl).
    let prev_strong = curr_strong;
    let apply_filter = (is_tx_edge || is_sub_pu_edge)
        && (curr_strong || prev_strong)
        && (is_block_edge || !skip || is_sub_pu_edge);
    if !apply_filter {
        return Ok(());
    }

    // § 7.17.4 filter size: Min over the two transform dims in the pass direction.
    let filter_size = if pass == 0 {
        TX_WIDTH[tx_sz].min(TX_WIDTH[prev_tx_sz])
    } else {
        TX_HEIGHT[tx_sz].min(TX_HEIGHT[prev_tx_sz])
    };
    let mut filter_size = usize::try_from(filter_size).unwrap_or(0);

    // Clip the filter size at the screen edge.
    let plane_width = (mi_cols * MI_SIZE) >> plane_sub_x;
    let plane_height = (mi_rows * MI_SIZE) >> plane_sub_y;
    if plane == 0 {
        if x_p + dx * 16 > plane_width || y_p + dy * 16 > plane_height {
            filter_size = filter_size.min(16);
        }
    } else if x_p + dx * 8 > plane_width || y_p + dy * 8 > plane_height {
        filter_size = filter_size.min(8);
    }

    // qThr / side combine (curr == prev in the admitted subset).
    let (mut q_thr, mut side) = combine_strengths(curr_q, curr_q, curr_side, curr_side);
    if is_sub_pu_edge && !is_tx_edge {
        q_thr >>= 3;
        side >>= 3;
    }

    // prevLossless && currLossless terminate (segmentation disabled → all false).
    // (No lossless segment in the admitted subset.)

    // § 7.17.3 per-side maximum widths.
    let (max_width_neg, max_width_pos) = deblock_filter_max_width(filter_size, plane != 0, sb_edge);
    if max_width_neg == 0 || max_width_pos == 0 {
        return Ok(());
    }

    // § 7.17.7.2 filter choice over the s / t perpendicular lines.
    let width = choose_filter_width(
        workspace,
        plane_id,
        x_p,
        y_p,
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

    // § 7.17.7.1 sample filtering for each of the MI_SIZE edge positions.
    let eff_neg = width.min(max_width_neg);
    let eff_pos = width.min(max_width_pos);
    #[cfg(debug_assertions)]
    if std::env::var_os("SPLOT_DEBLOCK_TRACE").is_some() {
        eprintln!(
            "DEBLOCK plane={} pass={} edge(xP={},yP={}) lvl_strengths(qThr={},side={}) filterSize={} maxW(neg={},pos={}) width={} sbEdge={}",
            plane,
            pass,
            x_p,
            y_p,
            q_thr,
            side,
            filter_size,
            max_width_neg,
            max_width_pos,
            width,
            sb_edge
        );
    }
    let q_thresh_mult = Q_THRESH_MULTS[eff_neg.max(eff_pos) - 1];
    let w_mult_neg = W_MULT[eff_neg - 1];
    let w_mult_pos = W_MULT[eff_pos - 1];
    for i in 0..MI_SIZE {
        let px = x_p + dy * i;
        let py = y_p + dx * i;
        apply_sample_filter(
            workspace,
            plane_id,
            PerpLine::new(px, py, dx, dy),
            DeblockSampleFilter {
                boundary: 0, // set inside apply_sample_filter
                q_thr,
                max_width_neg: eff_neg,
                max_width_pos: eff_pos,
                q_thresh_mult,
                w_mult_neg,
                w_mult_pos,
                prev_lossless: false,
                curr_lossless: false,
                bit_depth,
            },
        )?;
    }

    Ok(())
}

/// Gathers the perpendicular `s` / `t` sample lines at the two ends of the
/// MI_SIZE-long edge and calls § 7.17.7.2 [`deblock_filter_choice`].
#[allow(clippy::too_many_arguments)]
fn choose_filter_width<T: ReconSample>(
    workspace: &CurrentFrameWorkspace<T>,
    plane_id: PlaneId,
    x_p: usize,
    y_p: usize,
    dx: usize,
    dy: usize,
    q_thr: i32,
    side: i32,
    max_width_neg: usize,
    max_width_pos: usize,
) -> Result<usize, DeblockError> {
    if q_thr == 0 || side == 0 {
        return Ok(0);
    }
    // The choice cascade reads from -maxSamplesNeg to maxSamplesPos - 1 (with
    // maxSamples = Clip3(3, 8, maxWidth + 1)); the positive span also reads s[3]
    // when maxWidthPos > 1. Gather a generous window of 8 + 8 around the boundary.
    let boundary = GATHER_HALF;

    // s: line perpendicular to the edge through (x_p, y_p).
    let s = gather_line(workspace, plane_id, PerpLine::new(x_p, y_p, dx, dy))?;
    // t: line through the far end of the MI_SIZE-long edge. The edge runs along
    // (dy, dx); its last position is offset (count - 1) along the boundary.
    let end = MI_SIZE - 1;
    let t_x = x_p + dy * end;
    let t_y = y_p + dx * end;
    let t = gather_line(workspace, plane_id, PerpLine::new(t_x, t_y, dx, dy))?;

    let width = deblock_filter_choice(
        &s,
        &t,
        &DeblockFilterChoice {
            boundary,
            q_thr,
            side_thr: side,
            max_width_pos,
            max_width_neg,
            q_first: Q_FIRST,
        },
    )
    .map_err(|_| DeblockError::FilterChoice)?;
    Ok(width)
}

/// A perpendicular sample line through frame position `(x, y)` advancing by
/// `(dx, dy)` (the § 7.17 filter direction). The gathered line indexes samples
/// at signed offsets from `(x, y)` along `(dx, dy)`.
#[derive(Clone, Copy)]
struct PerpLine {
    x: usize,
    y: usize,
    dx: usize,
    dy: usize,
}

impl PerpLine {
    const fn new(x: usize, y: usize, dx: usize, dy: usize) -> Self {
        Self { x, y, dx, dy }
    }

    /// Frame position offset by `offset` steps along `(dx, dy)`, without clamping
    /// (the caller has verified the position is in-frame).
    fn offset(self, offset: isize) -> Result<(usize, usize), DeblockError> {
        let fx = (self.x as isize + offset * self.dx as isize)
            .try_into()
            .map_err(|_| DeblockError::Workspace)?;
        let fy = (self.y as isize + offset * self.dy as isize)
            .try_into()
            .map_err(|_| DeblockError::Workspace)?;
        Ok((fx, fy))
    }
}

/// Half-window (each side) gathered around the boundary. The § 7.17.7.1 sample
/// filter and § 7.17.7.2 choice cascade never access past `MAX_DBL_FLT_LEN == 8`
/// samples on either side, so a window of 8 + 8 (boundary at index 8) covers
/// every § 7.17.3 width.
const GATHER_HALF: usize = 8;

/// Gathers the perpendicular sample line through `(x, y)`, applies the § 7.17.7.1
/// [`deblock_sample_filter`] (with `boundary` repointed at the gathered `q0`), and
/// writes the modified samples back into the workspace.
fn apply_sample_filter<T: ReconSample>(
    workspace: &mut CurrentFrameWorkspace<T>,
    plane_id: PlaneId,
    perp: PerpLine,
    params: DeblockSampleFilter,
) -> Result<(), DeblockError> {
    let boundary = GATHER_HALF;
    let mut line = gather_line(workspace, plane_id, perp)?;
    let before = line.clone();

    deblock_sample_filter(&mut line, &DeblockSampleFilter { boundary, ..params })
        .map_err(|_| DeblockError::SampleFilter)?;

    // Write back only the samples that changed (their original frame positions
    // are `(x + (idx - boundary) * dx, y + (idx - boundary) * dy)`).
    for (idx, (&new, &old)) in line.iter().zip(before.iter()).enumerate() {
        if new.to_u16() == old.to_u16() {
            continue;
        }
        let (fx, fy) = perp.offset(idx as isize - boundary as isize)?;
        workspace
            .set_reconstructed_sample(plane_id, fx, fy, new)
            .map_err(|_| DeblockError::Workspace)?;
    }
    Ok(())
}

/// Gathers `2 * GATHER_HALF` samples along the perpendicular direction `(dx, dy)`
/// centred so index `GATHER_HALF` is the sample at `(x, y)` (the spec `q0` /
/// boundary). Off-frame positions are clamped to the frame edge (the deblocking
/// edge selection guarantees the filtered samples are in-frame; the wider gather
/// window can reach off-frame for the choice cascade's deep reads, which the edge
/// geometry never actually filters past).
fn gather_line<T: ReconSample>(
    workspace: &CurrentFrameWorkspace<T>,
    plane_id: PlaneId,
    perp: PerpLine,
) -> Result<Vec<T>, DeblockError> {
    let total = 2 * GATHER_HALF;
    let plane = workspace
        .plane(plane_id)
        .map_err(|_| DeblockError::Workspace)?;
    let max_x = plane.storage_size().width().saturating_sub(1) as isize;
    let max_y = plane.storage_size().height().saturating_sub(1) as isize;
    let mut line = Vec::new();
    line.try_reserve_exact(total)
        .map_err(|_| DeblockError::Workspace)?;
    for idx in 0..total {
        let offset = idx as isize - GATHER_HALF as isize;
        let sx = (perp.x as isize + offset * perp.dx as isize).clamp(0, max_x) as usize;
        let sy = (perp.y as isize + offset * perp.dy as isize).clamp(0, max_y) as usize;
        let sample = workspace
            .reconstructed_sample(plane_id, sx, sy)
            .map_err(|_| DeblockError::Workspace)?;
        line.push(sample);
    }
    Ok(line)
}

/// AV2 § 7.17.6 `q_clamped(qindex, 0)` for the admitted zero-delta subset:
/// returns `0` when `qindex == 0`, else `qindex` (already in `0..=MaxQ`).
const fn q_clamped_zero_delta(qindex: u32) -> u32 {
    if qindex == 0 { 0 } else { qindex }
}

/// AV2 § 7.17.5 adaptive filter strength `(qThr, side)` for filter level `lvl`,
/// composing [`deblock_side_threshold_index`] over the § 9.2 `Side_Thresholds`
/// table with [`deblock_adaptive_filter_strength`].
fn adaptive_strength(lvl: u32, bit_depth: BitDepth) -> (i32, i32) {
    let q_ind = deblock_side_threshold_index(lvl, bit_depth);
    let side_threshold = SIDE_THRESHOLDS[q_ind];
    deblock_adaptive_filter_strength(lvl, side_threshold, bit_depth)
}

/// AV2 § 7.17.2 `qThr` / `side` combine: the average when both sides are nonzero,
/// else the max.
fn combine_strengths(curr_q: i32, prev_q: i32, curr_side: i32, prev_side: i32) -> (i32, i32) {
    let q_thr = if curr_q != 0 && prev_q != 0 {
        (curr_q + prev_q + 1) >> 1
    } else {
        curr_q.max(prev_q)
    };
    let side = if curr_side != 0 && prev_side != 0 {
        (curr_side + prev_side + 1) >> 1
    } else {
        curr_side.max(prev_side)
    };
    (q_thr, side)
}

/// `DeblockingTxSizes[plane]` for the covering block: the luma transform for
/// `plane == 0`, the chroma transform otherwise.
fn plane_tx(plane: usize, info: MiBlockInfo) -> Option<usize> {
    if plane == 0 {
        Some(info.luma_tx)
    } else {
        info.chroma_tx
    }
}

fn plane_index_to_id(plane: usize) -> PlaneId {
    match plane {
        0 => PlaneId::Y,
        1 => PlaneId::U,
        _ => PlaneId::V,
    }
}

/// Luma-MI-indexed covering-block grid.
struct MiGrid {
    mi_cols: usize,
    cells: Vec<Option<MiBlockInfo>>,
}

impl MiGrid {
    fn get(&self, row: usize, col: usize) -> Option<MiBlockInfo> {
        self.cells.get(row * self.mi_cols + col).copied().flatten()
    }
}

/// Builds the luma-MI covering-block grid from the decoded leaf blocks. Every MI
/// position the deblocking loop visits must be covered; an MI left uncovered is an
/// internal inconsistency that surfaces as [`DeblockError::UncoveredMi`] later.
fn build_mi_grid(
    blocks: &[DeblockBlock],
    mi_rows: usize,
    mi_cols: usize,
) -> Result<MiGrid, DeblockError> {
    let count = mi_rows
        .checked_mul(mi_cols)
        .ok_or(DeblockError::Workspace)?;
    let mut cells = Vec::new();
    cells
        .try_reserve_exact(count)
        .map_err(|_| DeblockError::Workspace)?;
    cells.resize(count, None);

    for block in blocks {
        let info = MiBlockInfo {
            base_row: block.r,
            base_col: block.c,
            luma_tx: block.luma_tx,
            chroma_tx: block.chroma_tx,
        };
        for rr in block.r..block.r + block.n4h {
            for cc in block.c..block.c + block.n4w {
                if rr < mi_rows && cc < mi_cols {
                    cells[rr * mi_cols + cc] = Some(info);
                }
            }
        }
    }
    Ok(MiGrid { mi_cols, cells })
}

/// Errors from the deblocking-filter orchestration. These signal an internal
/// inconsistency (the per-edge primitives are total for valid inputs), so the
/// caller maps them to an `unsupported-feature` decode diagnostic rather than a
/// silent wrong-pixel output.
#[derive(Debug, thiserror::Error)]
pub(crate) enum DeblockError {
    /// A visited MI position was not covered by any decoded block.
    #[error("deblocking MI ({row}, {col}) is not covered by any decoded block")]
    UncoveredMi { row: usize, col: usize },
    /// A plane's `DeblockingTxSizes` entry was missing (chroma tx on a luma-only
    /// block).
    #[error("deblocking plane {plane} has no transform size for the covering block")]
    MissingTx { plane: usize },
    /// The § 7.17.7.2 filter-choice primitive rejected its inputs.
    #[error("deblocking filter-choice primitive rejected its inputs")]
    FilterChoice,
    /// The § 7.17.7.1 sample-filter primitive rejected its inputs.
    #[error("deblocking sample-filter primitive rejected its inputs")]
    SampleFilter,
    /// A workspace read/write or geometry computation went out of bounds.
    #[error("deblocking workspace sample access went out of bounds")]
    Workspace,
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn q_clamped_zero_delta_matches_spec() {
        // §7.17.6 q_clamped(qindex, 0): 0 iff qindex == 0, else qindex.
        assert_eq!(q_clamped_zero_delta(0), 0);
        assert_eq!(q_clamped_zero_delta(1), 1);
        assert_eq!(q_clamped_zero_delta(100), 100);
        assert_eq!(q_clamped_zero_delta(255), 255);
    }

    #[test]
    fn adaptive_strength_for_lvl_100_8bit() {
        // §7.17.5: lvl 100, 8-bit -> qInd 100, Side_Thresholds[100] == 46,
        // side = Max(46 + (1 << 4), 0) >> 5 = (62) >> 5 = 1. qThr is the
        // get_q-composed threshold (positive for a nonzero level).
        let (q_thr, side) = adaptive_strength(100, BitDepth::Eight);
        assert_eq!(side, 1, "side threshold for lvl 100 (8-bit)");
        assert!(q_thr > 0, "qThr must be positive for a nonzero level");
    }

    #[test]
    fn combine_strengths_averages_then_maxes() {
        // §7.17.2: average when both nonzero, else max.
        assert_eq!(
            combine_strengths(3, 5, 2, 4),
            ((3 + 5 + 1) >> 1, (2 + 4 + 1) >> 1)
        );
        assert_eq!(combine_strengths(0, 5, 0, 4), (5, 4));
        assert_eq!(combine_strengths(3, 0, 2, 0), (3, 2));
    }

    #[test]
    fn empty_apply_pattern_is_a_no_op() {
        // §7.17.1: with no pass enabled, the orchestration returns immediately
        // and never touches the workspace (or the empty block grid).
        use splot_recon::{
            CurrentFrameWorkspace, DecodedFrameInfo, OutputIndex, PixelFormat, PlaneRect, PlaneSize,
        };
        let info = DecodedFrameInfo::new(
            OutputIndex::new(0),
            BitDepth::Eight,
            PixelFormat::Yuv420,
            PlaneSize::new(64, 64).unwrap(),
            PlaneRect::new(0, 0, 64, 64).unwrap(),
        )
        .unwrap();
        let mut workspace = CurrentFrameWorkspace::<u8>::new(info, 100).unwrap();
        // An empty block grid would error if the loop ran, but the all-false
        // apply pattern returns before building or iterating it.
        deblock_general_intra_frame(
            &mut workspace,
            &[],
            16,
            16,
            [false; 4],
            100,
            BitDepth::Eight,
        )
        .unwrap();
        assert!(
            workspace
                .samples(PlaneId::Y)
                .unwrap()
                .iter()
                .all(|&s| s == 100),
            "no-op deblock leaves the workspace untouched"
        );
    }

    #[test]
    fn mi_grid_covers_decoded_blocks() {
        // A 32x32 DC block (n4w == n4h == 8) covers its 8x8 MI footprint.
        let blocks = [DeblockBlock {
            r: 0,
            c: 0,
            n4w: 8,
            n4h: 8,
            luma_tx: 3,
            chroma_tx: Some(2),
        }];
        let grid = build_mi_grid(&blocks, 16, 16).unwrap();
        assert!(grid.get(0, 0).is_some(), "top-left MI is covered");
        assert!(
            grid.get(7, 7).is_some(),
            "bottom-right of the 8x8 footprint is covered"
        );
        assert!(
            grid.get(8, 8).is_none(),
            "an MI outside the block is uncovered"
        );
        let info = grid.get(0, 0).unwrap();
        assert_eq!((info.base_row, info.base_col), (0, 0));
        assert_eq!(plane_tx(0, info), Some(3), "luma tx index");
        assert_eq!(plane_tx(1, info), Some(2), "chroma tx index");
    }

    #[test]
    fn luma_vertical_pass_filters_the_x64_block_edge() {
        // §7.17.1/2 pass 0 (vertical), luma: a clean small vertical step at the
        // x=64 boundary is smoothed, while the flat interior and the x=32 / x=96
        // within-region edges (no step) stay untouched. This deterministically
        // pins the luma-VERTICAL path (apply[0]) — admitted and reachable, but
        // never sample-changing in any avmenc-producible DC-multi-block oracle
        // fixture (the encoder leaves the DC subset before luma-vertical fires),
        // so it cannot be pinned by a decode-hash fixture alone (DEBLOCK-001).
        use splot_recon::{
            CurrentFrameWorkspace, DecodedFrameInfo, OutputIndex, PixelFormat, PlaneRect, PlaneSize,
        };
        let info = DecodedFrameInfo::new(
            OutputIndex::new(0),
            BitDepth::Eight,
            PixelFormat::Yuv420,
            PlaneSize::new(128, 64).unwrap(),
            PlaneRect::new(0, 0, 128, 64).unwrap(),
        )
        .unwrap();
        let mut ws = CurrentFrameWorkspace::<u8>::new(info, 100).unwrap();
        // Vertical step at x=64: left half luma 100, right half 108.
        for y in 0..64 {
            for x in 64..128 {
                ws.set_reconstructed_sample(PlaneId::Y, x, y, 108).unwrap();
            }
        }
        // Eight decoded 32x32 DC blocks tile the 128x64 frame (luma tx 32x32 =
        // TX_SIZES_ALL index 3, 4:2:0 chroma tx 16x16 = index 2).
        let mut blocks = Vec::new();
        for r in [0usize, 8] {
            for c in [0usize, 8, 16, 24] {
                blocks.push(DeblockBlock {
                    r,
                    c,
                    n4w: 8,
                    n4h: 8,
                    luma_tx: 3,
                    chroma_tx: Some(2),
                });
            }
        }
        // Luma-vertical pass only.
        deblock_general_intra_frame(
            &mut ws,
            &blocks,
            16,
            32,
            [true, false, false, false],
            100,
            BitDepth::Eight,
        )
        .unwrap();
        let at = |x, y| ws.reconstructed_sample(PlaneId::Y, x, y).unwrap();
        // Deep interior (far from any step) is untouched.
        assert_eq!(at(10, 32), 100, "left interior untouched");
        assert_eq!(at(120, 32), 108, "right interior untouched");
        // The x=32 and x=96 transform edges sit inside flat regions (no step),
        // so the filter computes a zero offset and changes nothing.
        assert_eq!(at(31, 32), 100, "x=32 within-region edge untouched");
        assert_eq!(at(32, 32), 100, "x=32 within-region edge untouched");
        assert_eq!(at(95, 32), 108, "x=96 within-region edge untouched");
        assert_eq!(at(96, 32), 108, "x=96 within-region edge untouched");
        // The x=64 vertical edge fired and smoothed the step toward the middle
        // (p0 rises, q0 falls), staying within the original [100, 108] band.
        let p0 = at(63, 32);
        let q0 = at(64, 32);
        assert!(
            (100..=108).contains(&p0) && (100..=108).contains(&q0),
            "smoothing stays within the step band: p0={p0} q0={q0}"
        );
        assert!(
            p0 > 100 || q0 < 108,
            "luma-vertical pass must change the x=64 edge: p0={p0} q0={q0}"
        );
    }

    #[test]
    fn luma_horizontal_pass_filters_the_y64_superblock_edge() {
        // §7.17.1/2 pass 1 (horizontal), luma, at y=64: the 64-sample grid row
        // (`horz64Edge`, y % 64 == 0) is an `sbEdge`, which caps the negative-side
        // (upward, cross-superblock) max width in §7.17.3. A clean small step at
        // y=64 is smoothed; the flat interior and the y=32 / y=96 non-sbEdge
        // within-region edges stay untouched. This pins the sample-changing
        // y=64 `sbEdge` path (DEBLOCK-001) — exercised end-to-end by the
        // 128x128 grid fixture's iteration, but flat there, so the cap math
        // needs a deterministic step to be pinned.
        use splot_recon::{
            CurrentFrameWorkspace, DecodedFrameInfo, OutputIndex, PixelFormat, PlaneRect, PlaneSize,
        };
        let info = DecodedFrameInfo::new(
            OutputIndex::new(0),
            BitDepth::Eight,
            PixelFormat::Yuv420,
            PlaneSize::new(128, 128).unwrap(),
            PlaneRect::new(0, 0, 128, 128).unwrap(),
        )
        .unwrap();
        let mut ws = CurrentFrameWorkspace::<u8>::new(info, 100).unwrap();
        // Horizontal step at y=64: top half luma 100, bottom half 108.
        for y in 64..128 {
            for x in 0..128 {
                ws.set_reconstructed_sample(PlaneId::Y, x, y, 108).unwrap();
            }
        }
        // Sixteen decoded 32x32 DC blocks tile the 128x128 frame.
        let mut blocks = Vec::new();
        for r in [0usize, 8, 16, 24] {
            for c in [0usize, 8, 16, 24] {
                blocks.push(DeblockBlock {
                    r,
                    c,
                    n4w: 8,
                    n4h: 8,
                    luma_tx: 3,
                    chroma_tx: Some(2),
                });
            }
        }
        // Luma-horizontal pass only.
        deblock_general_intra_frame(
            &mut ws,
            &blocks,
            32,
            32,
            [false, true, false, false],
            100,
            BitDepth::Eight,
        )
        .unwrap();
        let at = |x, y| ws.reconstructed_sample(PlaneId::Y, x, y).unwrap();
        // Deep interior (far from any step) is untouched.
        assert_eq!(at(64, 10), 100, "top interior untouched");
        assert_eq!(at(64, 120), 108, "bottom interior untouched");
        // The y=32 and y=96 transform edges sit inside flat regions (no step),
        // so the filter changes nothing there.
        assert_eq!(at(64, 31), 100, "y=32 within-region edge untouched");
        assert_eq!(at(64, 96), 108, "y=96 within-region edge untouched");
        // The y=64 sbEdge fired and smoothed the step toward the middle.
        let p0 = at(64, 63);
        let q0 = at(64, 64);
        assert!(
            (100..=108).contains(&p0) && (100..=108).contains(&q0),
            "smoothing stays within the step band: p0={p0} q0={q0}"
        );
        assert!(
            p0 > 100 || q0 < 108,
            "luma-horizontal sbEdge pass must change the y=64 edge: p0={p0} q0={q0}"
        );
        // §7.17.3 sbEdge negative-side cap: the upward (cross-superblock) extent
        // is bounded, so a sample well above the edge stays at the original value.
        assert_eq!(
            at(64, 59),
            100,
            "sbEdge caps the upward extent (row 59 unchanged)"
        );
    }
}
