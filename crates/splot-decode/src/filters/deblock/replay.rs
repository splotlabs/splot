// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The kernel calls of one chroma plane pass, recorded so the other chroma
//! plane can repeat them on its own samples.

use core::cell::Cell;

use splot_core::tables::conversion::{Q_FIRST, Q_THRESH_MULTS, W_MULT};
use splot_recon::{
    BitDepth, DeblockFilterChoice, DeblockSampleFilter, ReconSample, deblock_edge_columns,
    deblock_edge_rows,
};

use super::{
    ContiguousEdge, DeblockError, FrameDeblock, GATHER_HALF, MI_SIZE, PerpLine, PlaneBand,
    PlaneCtx, PlanePass, StrengthCache, StrengthKey, apply_edge_samples, choose_filter_width,
    filter_contiguous_run,
};

/// One kernel call of a chroma pass: a run of contiguous edges, or an edge the
/// contiguous kernels cannot reach.
#[derive(Clone, Copy)]
pub(super) enum ReplayStep {
    Run(ContiguousEdge, usize),
    Lone(LoneEdge),
}

/// The steps one advance's U passes took, by pass.
pub(super) type ReplayLog = [Vec<ReplayStep>; 2];

std::thread_local! {
    static REPLAY_LOG: Cell<ReplayLog> = const { Cell::new([const { Vec::new() }; 2]) };
}

/// Takes this worker's log storage.
pub(super) fn take_log() -> ReplayLog {
    REPLAY_LOG.take()
}

/// Hands log storage back to this worker for its next advance.
pub(super) fn keep_log(log: ReplayLog) {
    REPLAY_LOG.set(log);
}

/// An edge at plane position (`x_p`, `y_p`) that is filtered sample by sample,
/// with the filter inputs its records decided.
#[derive(Clone, Copy)]
pub(super) struct LoneEdge {
    pub(super) x_p: usize,
    pub(super) y_p: usize,
    pub(super) q_thr: i32,
    pub(super) side: i32,
    pub(super) max_width_neg: u8,
    pub(super) max_width_pos: u8,
    pub(super) prev_lossless: bool,
    pub(super) curr_lossless: bool,
    pub(super) key: StrengthKey,
}

/// Samples a lone edge's window holds: its four lines of `2 * GATHER_HALF`.
const WINDOW: usize = MI_SIZE * 2 * GATHER_HALF;

/// [`deblock_edge_rows`] and [`deblock_edge_columns`].
type EdgeKernel<T> = fn(
    &mut [T],
    usize,
    &DeblockFilterChoice,
    usize,
    &[i32; 8],
    &[i32; 8],
    bool,
    bool,
    BitDepth,
) -> splot_recon::Result<()>;

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
    fn filter_lines<T: ReconSample, const PASS: usize>(
        self,
        plane_ctx: &mut PlaneCtx<'_, '_, T>,
        bit_depth: BitDepth,
    ) -> Result<(), DeblockError> {
        let (dx, dy) = if PASS == 0 { (1, 0) } else { (0, 1) };
        let (max_width_neg, max_width_pos) = (self.max_width_neg.into(), self.max_width_pos.into());
        let width = choose_filter_width(
            plane_ctx,
            self.x_p,
            self.y_p,
            dx,
            dy,
            self.q_thr,
            self.side,
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
            q_thr: self.q_thr,
            max_width_neg: eff_neg,
            max_width_pos: eff_pos,
            q_thresh_mult: Q_THRESH_MULTS[eff_neg.max(eff_pos) - 1],
            w_mult_neg: W_MULT[eff_neg - 1],
            w_mult_pos: W_MULT[eff_pos - 1],
            prev_lossless: self.prev_lossless,
            curr_lossless: self.curr_lossless,
            bit_depth,
        };
        apply_edge_samples(
            plane_ctx,
            PerpLine::new(self.x_p, self.y_p, dx, dy),
            MI_SIZE,
            sample_params,
        )
    }

    /// Filters the edge with the contiguous kernel on a copy of its four
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
        let choice = DeblockFilterChoice {
            boundary: if PASS == 0 {
                GATHER_HALF
            } else {
                GATHER_HALF * MI_SIZE
            },
            q_thr: self.q_thr,
            side_thr: self.side,
            max_width_pos: self.max_width_pos.into(),
            max_width_neg: self.max_width_neg.into(),
            q_first: Q_FIRST,
        };
        let (filter, stride) = if PASS == 0 {
            (deblock_edge_rows::<T> as EdgeKernel<T>, line)
        } else {
            (deblock_edge_columns::<T> as EdgeKernel<T>, MI_SIZE)
        };
        filter(
            &mut window,
            stride,
            &choice,
            1,
            &Q_THRESH_MULTS,
            &W_MULT,
            self.prev_lossless,
            self.curr_lossless,
            bit_depth,
        )
        .map_err(|_| DeblockError::SampleFilter)?;
        let (low, high) = if PASS == 0 {
            (0, ctx.width)
        } else {
            (ctx.y_origin, band_end.min(ctx.height))
        };
        let base = if PASS == 0 { x } else { y };
        let first = (low + GATHER_HALF).saturating_sub(base).min(line);
        let end = (high + GATHER_HALF).saturating_sub(base).clamp(first, line);
        let outside = |i: usize| i < first || i >= end;
        let changed_outside = if PASS == 0 {
            window
                .iter()
                .zip(&original)
                .enumerate()
                .any(|(k, (new, old))| outside(k % line) && new.to_u16() != old.to_u16())
        } else {
            window
                .iter()
                .zip(&original)
                .enumerate()
                .any(|(k, (new, old))| outside(k / MI_SIZE) && new.to_u16() != old.to_u16())
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

impl<T: ReconSample> PlaneCtx<'_, '_, T> {
    /// Appends `step` to the pass's log when this plane records one.
    pub(super) fn record<const PASS: usize>(
        &mut self,
        step: ReplayStep,
    ) -> Result<(), DeblockError> {
        if let Some(steps) = self.log.as_deref_mut().and_then(|log| log.get_mut(PASS)) {
            steps.try_reserve(1).map_err(|_| DeblockError::Allocation {
                plane: splot_recon::PlaneId::U,
                context: "deblock replay log",
            })?;
            steps.push(step);
        }
        Ok(())
    }
}

impl FrameDeblock {
    /// Whether V repeats U's steps. V reads U's grid (see
    /// [`super::ChromaDeblockRecords::uv_twins`]), and the only plane inputs
    /// of the § 7.17.6 strength level are the AC delta and `DfDeltaQ[plane +
    /// 1]` (docs/spec/av2/1.0.0/07-decoding-process.md#s-7-17-6). Each step
    /// derives its strengths from its records on its own plane, as the
    /// filter widths from its samples, so the walks differ only in the edges
    /// they skip for want of strength: V must skip each one U skips.
    pub(super) fn replays_u_on_v(&self) -> bool {
        self.filter.apply_deblocking_filter[3] && self.chroma[1].is_none() && self.v_strong_in_u
    }

    /// Whether every record `qindex` of the frame that V filters with is one U
    /// filters with too.
    pub(super) fn v_strong_in_u(&self, bit_depth: BitDepth) -> bool {
        let qindexes = self.records.blocks.iter().chain(
            self.records
                .chroma
                .iter_plane(0)
                .chain(self.records.chroma.iter_plane(1))
                .map(|(_, block)| block),
        );
        let Some((low, high)) =
            qindexes
                .map(|block| block.qindex)
                .fold(None, |range: Option<(u32, u32)>, q| {
                    Some(range.map_or((q, q), |(low, high)| (low.min(q), high.max(q))))
                })
        else {
            return true;
        };
        let cache = |plane: usize| {
            StrengthCache::new(
                self.quant_deltas.ac_delta(plane),
                self.filter.df_delta_q[plane + 1],
                bit_depth,
            )
        };
        let (u, v) = (cache(1), cache(2));
        (low..=high).all(|qindex| !v.strong(qindex) || u.strong(qindex))
    }
}

/// Repeats the logged steps of each of `passes` on `band`, in the order U ran
/// them. Run boundaries are band offsets, so `band` must have the geometry of
/// the U band that logged them.
pub(super) fn replay<T: ReconSample>(
    mut band: PlaneBand<'_, T>,
    passes: [Option<PlanePass>; 2],
    log: &ReplayLog,
) -> Result<(), DeblockError> {
    for plane_pass in passes.into_iter().flatten() {
        let mut ctx = PlaneCtx::new(&mut band)?;
        let steps = log.get(plane_pass.pass).map_or(&[][..], Vec::as_slice);
        let strengths = StrengthCache::new(
            plane_pass.quant_delta,
            plane_pass.df_delta_q,
            plane_pass.bit_depth,
        );
        if plane_pass.pass == 0 {
            replay_pass::<T, 0>(&mut ctx, steps, &strengths, plane_pass.bit_depth)?;
        } else {
            replay_pass::<T, 1>(&mut ctx, steps, &strengths, plane_pass.bit_depth)?;
        }
    }
    Ok(())
}

fn replay_pass<T: ReconSample, const PASS: usize>(
    ctx: &mut PlaneCtx<'_, '_, T>,
    steps: &[ReplayStep],
    strengths: &StrengthCache,
    bit_depth: BitDepth,
) -> Result<(), DeblockError> {
    for &step in steps {
        match step {
            ReplayStep::Run(first, edges) => {
                let (q_thr, side) = first.key.strengths(strengths);
                let first = ContiguousEdge {
                    q_thr,
                    side,
                    ..first
                };
                filter_contiguous_run::<T, PASS>(ctx, first, edges, bit_depth)?;
            }
            ReplayStep::Lone(edge) => {
                let (q_thr, side) = edge.key.strengths(strengths);
                LoneEdge {
                    q_thr,
                    side,
                    ..edge
                }
                .filter::<T, PASS>(ctx, bit_depth)?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use splot_recon::{CurrentFrameWorkspace, PlaneId};

    use super::super::tests::{deblock_blocks, filter, source_from_workspace};
    use super::super::{
        ChromaDeblockRecords, DeblockBlock, DeblockGridStorage, DeblockPredictionUnit,
        DeblockQuantDeltas,
    };
    use super::*;
    use crate::test_support::{copy_rows_to_workspace, yuv420_workspace};

    type Pattern = fn(usize, usize) -> u8;

    const P: Pattern = |x, y| (100 + 6 * ((x / 4 + y / 4) % 2) + (x * y) % 3) as u8;
    const Q: Pattern = |x, y| (96 + 5 * ((x / 4 + 2 * (y / 4)) % 3) + (x + 2 * y) % 4) as u8;

    /// A `DfDeltaQ` that drops every record's § 7.17.6 level to zero.
    const WEAK_DF_DELTA_Q: i32 = -40;

    /// Deblocks a 128x128 4:2:0 frame in two advances, with U and V filled from
    /// `patterns` and twin chroma records whose edges reach the plane borders.
    /// Returns the filtered U and V planes and the steps the last advance logged.
    fn deblock_chroma(
        df_delta_q: [i32; 2],
        patterns: [Pattern; 2],
    ) -> ([Vec<u8>; 2], Vec<ReplayStep>) {
        let (mi_rows, mi_cols) = (32, 32);
        let blocks = deblock_blocks(mi_rows, mi_cols);
        let mut chroma = ChromaDeblockRecords::default();
        for (index, (r, c)) in (2..mi_rows as u32)
            .step_by(4)
            .flat_map(|r| (2..mi_cols as u32).step_by(4).map(move |c| (r, c)))
            .enumerate()
        {
            let block = DeblockBlock {
                r,
                c,
                n4w: 2,
                n4h: 2,
                chroma_base_r: r,
                chroma_base_c: c,
                chroma_prediction: DeblockPredictionUnit::new(r as usize, c as usize, 0),
                chroma_tx: Some(0),
                ..blocks[0]
            };
            if index % 2 == 0 {
                chroma.push_both(block);
            } else {
                chroma.push(0, block);
                chroma.push(1, block);
            }
        }
        let mut params = filter([true; 4]);
        params.df_delta_q[2..].copy_from_slice(&df_delta_q);
        let mut workspace = yuv420_workspace(128, 128, 0);
        let planes = [
            (PlaneId::Y, 128, patterns[0]),
            (PlaneId::U, 64, patterns[0]),
            (PlaneId::V, 64, patterns[1]),
        ];
        for (plane, size, pattern) in planes {
            for y in 0..size {
                for x in 0..size {
                    workspace
                        .set_reconstructed_sample(plane, x, y, pattern(x, y))
                        .unwrap();
                }
            }
        }
        let chroma_samples = |workspace: &CurrentFrameWorkspace<u8>| {
            [PlaneId::U, PlaneId::V].map(|plane| workspace.plane(plane).unwrap().samples().to_vec())
        };
        let before = chroma_samples(&workspace);
        drop(take_log());
        let mut plan = FrameDeblock::prepare(
            &blocks,
            &chroma,
            mi_rows,
            mi_cols,
            params,
            None,
            false,
            DeblockQuantDeltas::ZERO,
            (1, 1),
            &mut DeblockGridStorage::default(),
        )
        .unwrap()
        .unwrap();
        let mut source = source_from_workspace(&mut workspace);
        for mi_row_end in [16, 32] {
            plan.advance_source(&mut source, mi_row_end, BitDepth::Eight)
                .unwrap();
        }
        copy_rows_to_workspace(&mut source, &mut workspace);
        let after = chroma_samples(&workspace);
        let weak = df_delta_q.map(|delta| delta <= WEAK_DF_DELTA_Q);
        assert!(
            after
                .iter()
                .zip(&before)
                .zip(weak)
                .all(|((after, before), weak)| (after != before) != weak)
        );
        (after, take_log().concat())
    }

    #[test]
    fn v_replays_u_steps_when_both_planes_share_their_strength_inputs() {
        let ([u, v], steps) = deblock_chroma([1, 1], [P, Q]);
        assert!(steps.iter().any(|step| matches!(step, ReplayStep::Run(..))));
        assert!(steps.iter().any(|step| matches!(step, ReplayStep::Lone(_))));
        let ([walked_q, walked_p], _) = deblock_chroma([1, 1], [Q, P]);
        assert_eq!(v, walked_q, "V replayed over Q matches U walked over Q");
        assert_eq!(u, walked_p);
    }

    /// Filters the lone edge at plane (`x_p`, `y_p`) of a noisy 10-bit plane
    /// through `filter`.
    fn lone_edge<const PASS: usize>(
        (x_p, y_p): (usize, usize),
        filter: fn(LoneEdge, &mut PlaneCtx<'_, '_, u16>) -> Result<(), DeblockError>,
    ) -> Vec<u16> {
        let (width, height) = (24, 24);
        let mut samples: Vec<u16> = (0..width * height)
            .map(|i| (512 + 37 * (i % 7) + 11 * (i % 5) + 90 * (i % 24 / 12)) as u16)
            .collect();
        let mut band = PlaneBand::plane(&mut samples, width, width, height);
        let mut ctx = PlaneCtx::new(&mut band).unwrap();
        let edge = LoneEdge {
            x_p,
            y_p,
            q_thr: 60,
            side: 400,
            max_width_neg: 2,
            max_width_pos: 3,
            prev_lossless: false,
            curr_lossless: false,
            key: StrengthKey::default(),
        };
        filter(edge, &mut ctx).unwrap();
        samples
    }

    #[test]
    fn a_lone_edge_window_filters_as_its_lines() {
        let window = |edge: LoneEdge, ctx: &mut PlaneCtx<'_, '_, u16>| {
            assert!(edge.filter_window::<u16, 0>(ctx, BitDepth::Ten)?);
            Ok(())
        };
        let lines = |edge: LoneEdge, ctx: &mut PlaneCtx<'_, '_, u16>| {
            edge.filter_lines::<u16, 0>(ctx, BitDepth::Ten)
        };
        for at in [(4, 8), (20, 0), (12, 20)] {
            assert_eq!(lone_edge::<0>(at, window), lone_edge::<0>(at, lines));
        }
        let window = |edge: LoneEdge, ctx: &mut PlaneCtx<'_, '_, u16>| {
            assert!(edge.filter_window::<u16, 1>(ctx, BitDepth::Ten)?);
            Ok(())
        };
        let lines = |edge: LoneEdge, ctx: &mut PlaneCtx<'_, '_, u16>| {
            edge.filter_lines::<u16, 1>(ctx, BitDepth::Ten)
        };
        for at in [(8, 4), (0, 20), (20, 12)] {
            assert_eq!(lone_edge::<1>(at, window), lone_edge::<1>(at, lines));
        }
        let unfiltered = lone_edge::<1>((8, 4), |_, _| Ok(()));
        assert_ne!(lone_edge::<1>((8, 4), window), unfiltered);
    }

    #[test]
    fn v_replays_u_steps_at_its_own_strengths_when_the_chroma_df_delta_q_differ() {
        let ([u, v], steps) = deblock_chroma([1, 4], [P, Q]);
        assert!(!steps.is_empty());
        let ([walked_q, walked_p], _) = deblock_chroma([4, 1], [Q, P]);
        assert_eq!(
            v, walked_q,
            "V over Q matches U walked over Q with V's delta"
        );
        assert_eq!(u, walked_p);
        assert_ne!(
            v,
            deblock_chroma([1, 1], [P, Q]).0[1],
            "the delta changes V"
        );
    }

    #[test]
    fn v_walks_its_own_edges_when_it_filters_a_record_u_does_not() {
        let ([_, v], steps) = deblock_chroma([WEAK_DF_DELTA_Q, 1], [P, Q]);
        assert!(steps.is_empty());
        let ([walked_q, _], _) = deblock_chroma([1, WEAK_DF_DELTA_Q], [Q, P]);
        assert_eq!(v, walked_q);
    }
}
