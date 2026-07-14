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
//!
//! Feature tracking: `INFRA-DECODE-PARALLEL-STAGES`.

use std::sync::{Mutex, MutexGuard};

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
use super::tip::{self, TipReconstructScratch, apply_tip_temporal_records};
use crate::Result;
use crate::bitstream::tile_payload::{
    DecodeBlockFrontier, FrameQuantizerSnapshot, current_frame_qm_segment_id,
};
static BLOCK_RECON_U8_STORAGE: Mutex<Vec<Vec<Vec<u8>>>> = Mutex::new(Vec::new());
static BLOCK_RECON_U16_STORAGE: Mutex<Vec<Vec<Vec<u16>>>> = Mutex::new(Vec::new());

fn lock_storage<T>(storage: &Mutex<Vec<Vec<Vec<T>>>>) -> MutexGuard<'_, Vec<Vec<Vec<T>>>> {
    storage
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn take_storage<T>(storage: &Mutex<Vec<Vec<Vec<T>>>>) -> Vec<Vec<T>> {
    lock_storage(storage).pop().unwrap_or_default()
}

fn recycle_storage<T>(storage: &Mutex<Vec<Vec<Vec<T>>>>, buffers: Vec<Vec<T>>) {
    let mut storage = lock_storage(storage);
    if storage.len() < splot_parallel::current_pool_width() {
        storage.push(buffers);
    }
}

/// Decoded sample storage with a type-safe deferred-window recycler.
pub(crate) trait DeferredReconSample: ReconSample {
    /// Takes retained block-window storage.
    fn take_deferred_storage() -> Vec<Vec<Self>>;

    /// Returns block-window storage to the shared recycler.
    fn recycle_deferred_storage(storage: Vec<Vec<Self>>);
}

impl DeferredReconSample for u8 {
    fn take_deferred_storage() -> Vec<Vec<Self>> {
        take_storage(&BLOCK_RECON_U8_STORAGE)
    }

    fn recycle_deferred_storage(storage: Vec<Vec<Self>>) {
        recycle_storage(&BLOCK_RECON_U8_STORAGE, storage);
    }
}

impl DeferredReconSample for u16 {
    fn take_deferred_storage() -> Vec<Vec<Self>> {
        take_storage(&BLOCK_RECON_U16_STORAGE)
    }

    fn recycle_deferred_storage(storage: Vec<Vec<Self>>) {
        recycle_storage(&BLOCK_RECON_U16_STORAGE, storage);
    }
}

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
struct PendingBlock<T: ReconSample> {
    placed: PlacedInterBlock,
    kind: PendingKind,
    segment_id: usize,
    qindex: u32,
    tile_offset: ByteOffset,
    tip_scratch: TipReconstructScratch<T>,
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
    Tip,
}

type JobResult<T> = std::result::Result<(BlockReconWindow<T>, JobOutput), Vec<T>>;

/// Queue of pure inter blocks whose reconstruction is deferred to a flush.
#[derive(Debug)]
pub(super) struct DeferredInterRecon<T: DeferredReconSample> {
    pending: Vec<PendingBlock<T>>,
    storage: Vec<Vec<T>>,
    tip_scratch: Vec<TipReconstructScratch<T>>,
}

impl<T: DeferredReconSample> DeferredInterRecon<T> {
    pub(super) fn new() -> Self {
        Self {
            pending: Vec::new(),
            storage: T::take_deferred_storage(),
            tip_scratch: Vec::new(),
        }
    }

    pub(super) fn push(
        &mut self,
        placed: PlacedInterBlock,
        kind: PendingKind,
        qindex: u32,
        tile_offset: ByteOffset,
    ) {
        let tip_scratch = if matches!(kind, PendingKind::Tip) {
            self.take_tip_scratch()
        } else {
            TipReconstructScratch::default()
        };
        self.pending.push(PendingBlock {
            segment_id: current_frame_qm_segment_id(),
            placed,
            kind,
            qindex,
            tile_offset,
            tip_scratch,
        });
    }

    pub(super) fn take_tip_scratch(&mut self) -> TipReconstructScratch<T> {
        self.tip_scratch.pop().unwrap_or_default()
    }

    pub(super) fn recycle_tip_scratch(&mut self, scratch: TipReconstructScratch<T>) {
        self.tip_scratch.push(scratch);
    }
}

impl<T: DeferredReconSample> Drop for DeferredInterRecon<T> {
    fn drop(&mut self) {
        for buffer in &mut self.storage {
            buffer.clear();
        }
        T::recycle_deferred_storage(core::mem::take(&mut self.storage));
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
    block: &mut PendingBlock<T>,
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
        PendingKind::Tip => {
            tip::reconstruct(
                &mut block.tip_scratch,
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
            )?;
            Ok(JobOutput::Tip)
        }
    }
}

fn run_windowed<T: ReconSample>(
    block: &mut PendingBlock<T>,
    workspace: &CurrentFrameWorkspace<T>,
    snapshot: &FrameQuantizerSnapshot,
    shared: &FlushShared<'_, '_, T>,
    samples: Vec<T>,
) -> JobResult<T> {
    let mut samples = samples;
    let Ok(mut window) = BlockReconWindow::for_block_with_storage(
        workspace,
        block.placed.motion_compensation_rect(),
        &mut samples,
    ) else {
        return Err(samples);
    };
    let _scopes = snapshot.install(block.segment_id);
    match execute(block, &mut WorkspaceSink::Window(&mut window), shared) {
        Ok(output) => Ok((window, output)),
        Err(_) => Err(window.into_samples()),
    }
}

#[allow(clippy::too_many_arguments)]
fn apply_output<T: ReconSample>(
    output: JobOutput,
    block: &PendingBlock<T>,
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
            &block.placed,
            compound.syntax,
            compound.warp_params,
            compound.grid.as_ref(),
            compound.mi_row,
            compound.mi_col,
            mi_rows,
            mi_cols,
            current_order_hint,
        ),
        JobOutput::Tip => {
            let coded = workspace.info().coded_luma_size();
            apply_tip_temporal_records(
                motion_field,
                shared.reference,
                shared.ref_frame_idx,
                coded.height().div_ceil(4),
                coded.width().div_ceil(4),
                shared.core.display_order_hint().unwrap_or(0),
                block.tip_scratch.records(),
            );
            Ok(())
        }
    }
}

/// Reconstructs and publishes every queued block, in queue order for all
/// order-observable effects.
#[allow(clippy::too_many_arguments)]
pub(super) fn flush_deferred<T: DeferredReconSample>(
    deferred: &mut DeferredInterRecon<T>,
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
    let mut pending = core::mem::take(&mut deferred.pending);
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
    if pending.len() == 1 {
        let block = &mut pending[0];
        let _scopes = snapshot.install(block.segment_id);
        let output = execute(block, &mut WorkspaceSink::Frame(workspace), &shared)?;
        let result = apply_output(
            output,
            block,
            workspace,
            motion_field,
            &shared,
            mi_rows,
            mi_cols,
            current_order_hint,
        );
        if matches!(block.kind, PendingKind::Tip) {
            deferred.recycle_tip_scratch(core::mem::take(&mut block.tip_scratch));
        }
        return result;
    }

    let batch_size = splot_parallel::current_pool_width()
        .min(pending.len())
        .max(1);
    if deferred.storage.len() < batch_size {
        deferred.storage.resize_with(batch_size, Vec::new);
    }
    let mut results = Vec::with_capacity(batch_size);
    for blocks in pending.chunks_mut(batch_size) {
        let slots = &mut deferred.storage[..blocks.len()];
        results.clear();
        {
            let frame: &CurrentFrameWorkspace<T> = workspace;
            slots
                .par_iter_mut()
                .zip(blocks.par_iter_mut())
                .map(|(samples, block)| {
                    run_windowed(block, frame, &snapshot, &shared, core::mem::take(samples))
                })
                .collect_into_vec(&mut results);
        }
        for ((block, result), samples) in blocks.iter_mut().zip(results.drain(..)).zip(slots) {
            let output = match result {
                Ok((window, output)) => {
                    let published = window.publish(workspace);
                    *samples = window.into_samples();
                    published?;
                    output
                }
                Err(storage) => {
                    *samples = storage;
                    let _scopes = snapshot.install(block.segment_id);
                    execute(block, &mut WorkspaceSink::Frame(workspace), &shared)?
                }
            };
            apply_output(
                output,
                block,
                workspace,
                motion_field,
                &shared,
                mi_rows,
                mi_cols,
                current_order_hint,
            )?;
            if matches!(block.kind, PendingKind::Tip) {
                deferred
                    .tip_scratch
                    .push(core::mem::take(&mut block.tip_scratch));
            }
        }
    }
    Ok(())
}
