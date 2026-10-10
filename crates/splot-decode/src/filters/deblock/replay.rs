// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The kernel calls of one chroma plane pass, recorded so the other chroma
//! plane can repeat them on its own samples.

use core::cell::Cell;

use splot_core::tables::conversion::{Q_THRESH_MULTS, W_MULT};
use splot_recon::{BitDepth, DeblockSampleFilter, ReconSample};

use super::{
    ContiguousEdge, DeblockError, FrameDeblock, GATHER_HALF, MI_SIZE, PerpLine, PlaneBand,
    PlaneCtx, PlanePass, apply_edge_samples, choose_filter_width, filter_contiguous_run,
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
}

impl LoneEdge {
    /// Chooses the filter width from the plane's own samples, then filters.
    pub(super) fn filter<T: ReconSample, const PASS: usize>(
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

impl FrameDeblock<'_> {
    /// Whether V repeats U's steps. V reads U's grid (see
    /// [`super::ChromaDeblockRecords::uv_twins`]), and the only plane inputs
    /// of the § 7.17.6 strength level are the AC delta and `DfDeltaQ[plane +
    /// 1]` (docs/spec/av2/1.0.0/07-decoding-process.md#s-7-17-6). When they
    /// match, the two walks make the same decisions; only the filter widths
    /// depend on samples, and each step chooses them on its own plane.
    pub(super) fn replays_u_on_v(&self) -> bool {
        self.filter.apply_deblocking_filter[3]
            && self.chroma[1].is_none()
            && self.filter.df_delta_q[2] == self.filter.df_delta_q[3]
            && self.quant_deltas.u_ac == self.quant_deltas.v_ac
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
        if plane_pass.pass == 0 {
            replay_pass::<T, 0>(&mut ctx, steps, plane_pass.bit_depth)?;
        } else {
            replay_pass::<T, 1>(&mut ctx, steps, plane_pass.bit_depth)?;
        }
    }
    Ok(())
}

fn replay_pass<T: ReconSample, const PASS: usize>(
    ctx: &mut PlaneCtx<'_, '_, T>,
    steps: &[ReplayStep],
    bit_depth: BitDepth,
) -> Result<(), DeblockError> {
    for &step in steps {
        match step {
            ReplayStep::Run(first, edges) => {
                filter_contiguous_run::<T, PASS>(ctx, first, edges, bit_depth)?;
            }
            ReplayStep::Lone(edge) => edge.filter::<T, PASS>(ctx, bit_depth)?,
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
        assert!(
            after
                .iter()
                .zip(&before)
                .all(|(after, before)| after != before)
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

    #[test]
    fn v_walks_its_own_edges_when_the_chroma_df_delta_q_differ() {
        let ([u, v], steps) = deblock_chroma([1, 4], [P, Q]);
        assert!(steps.is_empty());
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
}
