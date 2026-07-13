// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Deferred parallel reconstruction of pure inter blocks.
//!
//! The ordered walk parses every symbol and updates all parse-visible state
//! in raster order, but the reconstruction of a *pure* inter block — motion
//! compensation plus residual add — reads only reference frames and its own
//! rect, so it can run out of order. The walk queues those blocks here and
//! flushes the queue whenever a block that reads current-frame pixels
//! (intra, intraBC, BAWP, inter-intra) is reached, and at the end of the
//! walk. A flush renders every queued block into a [`BlockReconWindow`] on
//! the worker pool, publishes the windows into the frame workspace, and
//! applies the temporal-motion records in block order. Any job error falls
//! back to re-running that block inline against the frame workspace, so
//! failures surface exactly as they would on the ordered path.

use splot_core::headers::frame::FrameHeaderCore;
use splot_core::headers::sequence::SequenceHeader;
use splot_core::span::ByteOffset;
use splot_parallel::prelude::*;
use splot_recon::{BitDepth, CurrentFrameWorkspace, ReconSample};

use super::super::compound::CompoundBlockSyntax;
use super::super::find_mv_stack::{TemporalMotionField, TemporalMvContext};
use super::super::mc::{BlockReconWindow, CompoundMotionGrid, WorkspaceSink};
use super::super::{InterReferenceState, PlacedInterBlock};
use super::compound_path::record_compound_temporal_motion;
use super::prediction::reconstruct_pure_inter_block;
use super::tip::{self, TipTemporalRecord, apply_tip_temporal_records};
use crate::Result;
use crate::bitstream::tile_payload::{
    DecodeBlockFrontier, FrameQuantizerSnapshot, current_frame_qm_segment_id,
};

/// How a queued block reconstructs and which temporal record it produces.
#[derive(Clone, Copy, Debug)]
pub(super) enum PendingKind {
    Single,
    Compound {
        syntax: CompoundBlockSyntax,
        warp_params: [Option<[i32; 6]>; 2],
        mi_row: usize,
        mi_col: usize,
        use_refinemv: bool,
        refinemv_switchable: bool,
    },
    Tip,
}

#[derive(Debug)]
struct PendingBlock {
    placed: PlacedInterBlock,
    kind: PendingKind,
    segment_id: usize,
    qindex: u32,
    tile_offset: ByteOffset,
}

struct CompoundOutput {
    grid: Option<CompoundMotionGrid>,
    syntax: CompoundBlockSyntax,
    warp_params: [Option<[i32; 6]>; 2],
    mi_row: usize,
    mi_col: usize,
}

enum JobOutput {
    Single,
    Compound(Box<CompoundOutput>),
    Tip(Vec<TipTemporalRecord>),
}

type JobResult<T> = Result<(BlockReconWindow<T>, JobOutput)>;

/// Queue of pure inter blocks whose reconstruction is deferred to a flush.
#[derive(Debug)]
pub(super) struct DeferredInterRecon {
    pending: Vec<PendingBlock>,
}

impl DeferredInterRecon {
    pub(super) fn new() -> Self {
        Self {
            pending: Vec::new(),
        }
    }

    pub(super) fn push(
        &mut self,
        placed: PlacedInterBlock,
        kind: PendingKind,
        qindex: u32,
        tile_offset: ByteOffset,
    ) {
        self.pending.push(PendingBlock {
            segment_id: current_frame_qm_segment_id(),
            placed,
            kind,
            qindex,
            tile_offset,
        });
    }
}

/// A block is deferable only when every one of its reads and writes stays
/// inside its own luma-shaped rect: plain leaves whose chroma reference
/// geometry equals the luma geometry. Sub-8x8 group leaves and SDP
/// luma/chroma parts share pixels with siblings and stay on the ordered path.
pub(super) fn deferable_placed_geometry(
    placed: &PlacedInterBlock,
    frontier: &DecodeBlockFrontier,
) -> bool {
    !frontier.is_luma_part()
        && !frontier.is_chroma_part()
        && !placed.sub8x8_chroma
        && placed.chroma_luma_x == placed.luma_x
        && placed.chroma_luma_y == placed.luma_y
        && placed.chroma_luma_w == placed.luma_w
        && placed.chroma_luma_h == placed.luma_h
}

struct FlushShared<'a, 'r, T: ReconSample> {
    reference: &'a InterReferenceState<'r, T>,
    ref_frame_idx: &'a [u32],
    temporal_context: Option<&'a TemporalMvContext>,
    sequence: &'a SequenceHeader,
    core: &'a FrameHeaderCore,
    luma_use_tcq: bool,
    residual_use_ddt: bool,
    bit_depth: BitDepth,
}

fn execute<T: ReconSample>(
    block: &PendingBlock,
    sink: &mut WorkspaceSink<'_, T>,
    shared: &FlushShared<'_, '_, T>,
) -> Result<JobOutput> {
    match block.kind {
        PendingKind::Single => reconstruct_pure_inter_block(
            sink,
            &block.placed,
            false,
            false,
            shared.ref_frame_idx,
            shared.reference,
            block.qindex,
            shared.luma_use_tcq,
            shared.residual_use_ddt,
            shared.bit_depth,
            block.tile_offset,
        )
        .map(|_| JobOutput::Single),
        PendingKind::Compound {
            syntax,
            warp_params,
            mi_row,
            mi_col,
            use_refinemv,
            refinemv_switchable,
        } => reconstruct_pure_inter_block(
            sink,
            &block.placed,
            use_refinemv,
            refinemv_switchable,
            shared.ref_frame_idx,
            shared.reference,
            block.qindex,
            shared.luma_use_tcq,
            shared.residual_use_ddt,
            shared.bit_depth,
            block.tile_offset,
        )
        .map(|grid| {
            JobOutput::Compound(Box::new(CompoundOutput {
                grid,
                syntax,
                warp_params,
                mi_row,
                mi_col,
            }))
        }),
        PendingKind::Tip => tip::reconstruct(
            sink,
            &block.placed,
            shared.temporal_context.ok_or_else(|| {
                super::super::unsupported_at(
                    "inter_deferred_missing_temporal_context",
                    block.tile_offset,
                    "missing required input: inter.tip.temporal_context",
                    "7.10.6",
                )
            })?,
            shared.sequence,
            shared.core,
            shared.ref_frame_idx,
            shared.reference,
            block.qindex,
            shared.luma_use_tcq,
            shared.residual_use_ddt,
            shared.bit_depth,
            block.tile_offset,
        )
        .map(JobOutput::Tip),
    }
}

fn run_windowed<T: ReconSample>(
    block: &PendingBlock,
    workspace: &CurrentFrameWorkspace<T>,
    snapshot: FrameQuantizerSnapshot,
    shared: &FlushShared<'_, '_, T>,
) -> JobResult<T> {
    let mut window =
        BlockReconWindow::for_block(workspace, block.placed.motion_compensation_rect())?;
    let _scopes = snapshot.install(block.segment_id);
    let output = execute(block, &mut WorkspaceSink::Window(&mut window), shared)?;
    Ok((window, output))
}

#[allow(clippy::too_many_arguments)]
fn apply_output<T: ReconSample>(
    output: JobOutput,
    placed: &PlacedInterBlock,
    workspace: &CurrentFrameWorkspace<T>,
    motion_field: &mut TemporalMotionField,
    shared: &FlushShared<'_, '_, T>,
    mi_rows: usize,
    mi_cols: usize,
    current_order_hint: u32,
) -> Result<()> {
    match output {
        JobOutput::Single => Ok(()),
        JobOutput::Compound(compound) => record_compound_temporal_motion(
            motion_field,
            shared.reference,
            shared.ref_frame_idx,
            placed,
            compound.syntax,
            compound.warp_params,
            compound.grid.as_ref(),
            compound.mi_row,
            compound.mi_col,
            mi_rows,
            mi_cols,
            current_order_hint,
        ),
        JobOutput::Tip(records) => {
            let coded = workspace.info().coded_luma_size();
            apply_tip_temporal_records(
                motion_field,
                shared.reference,
                shared.ref_frame_idx,
                coded.height().div_ceil(4),
                coded.width().div_ceil(4),
                shared.core.display_order_hint().unwrap_or(0),
                &records,
            );
            Ok(())
        }
    }
}

/// Reconstructs and publishes every queued block, in queue order for all
/// order-observable effects.
#[allow(clippy::too_many_arguments)]
pub(super) fn flush_deferred<T: ReconSample>(
    deferred: &mut DeferredInterRecon,
    workspace: &mut CurrentFrameWorkspace<T>,
    motion_field: &mut TemporalMotionField,
    temporal_context: Option<&TemporalMvContext>,
    reference: &InterReferenceState<'_, T>,
    ref_frame_idx: &[u32],
    sequence: &SequenceHeader,
    core: &FrameHeaderCore,
    mi_rows: usize,
    mi_cols: usize,
    current_order_hint: u32,
    luma_use_tcq: bool,
    residual_use_ddt: bool,
    bit_depth: BitDepth,
) -> Result<()> {
    if deferred.pending.is_empty() {
        return Ok(());
    }
    let pending = core::mem::take(&mut deferred.pending);
    let snapshot = FrameQuantizerSnapshot::capture();
    let shared = FlushShared {
        reference,
        ref_frame_idx,
        temporal_context,
        sequence,
        core,
        luma_use_tcq,
        residual_use_ddt,
        bit_depth,
    };
    let mut results: Vec<Option<JobResult<T>>> = if pending.len() > 1 {
        let frame: &CurrentFrameWorkspace<T> = workspace;
        pending
            .par_iter()
            .map(|block| Some(run_windowed(block, frame, snapshot, &shared)))
            .collect()
    } else {
        pending.iter().map(|_| None).collect()
    };
    for (block, slot) in pending.iter().zip(results.iter_mut()) {
        let output = if let Some(Ok((window, output))) = slot.take() {
            window.publish(workspace)?;
            output
        } else {
            let _scopes = snapshot.install(block.segment_id);
            execute(block, &mut WorkspaceSink::Frame(workspace), &shared)?
        };
        apply_output(
            output,
            &block.placed,
            workspace,
            motion_field,
            &shared,
            mi_rows,
            mi_cols,
            current_order_hint,
        )?;
    }
    Ok(())
}
