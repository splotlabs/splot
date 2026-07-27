// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! One tile row's ordered commit into the frame.
//!
//! Feature tracking: `INFRA-DECODE-FRAME-PIPELINING`.
//!
//! [`replay_recon_row`] is the single serial frontier every walk commits
//! through. It reconstructs the row's entries — all of them on the serial and
//! multi-tile paths, and the ones the parallel prepass left behind on its own
//! out-of-order surface — folds the row's § 7.12 temporal motion records into
//! the frame's motion field, appends its filter records, and advances the
//! block-decoded availability grid.

use splot_core::headers::frame::FrameHeaderCore;
use splot_core::headers::sequence::SequenceHeader;
use splot_core::span::ByteOffset;
use splot_recon::{BitDepth, CurrentFrameWorkspace, ReconSample};

use super::super::find_mv_stack::{TemporalMotionField, TemporalMvContext};
use super::super::{InterReferenceState, SPEC_MODE_INFO, unsupported_at};
use super::ReconCommand;
use super::deferred_recon;
use super::tile::{ReconRow, ReconRowBuffers, TileFilterRecords};
use crate::Result;
use crate::bitstream::tile_payload::{FrameQuantizerSnapshot, TileBlockDecodedState};

fn append_row_filter_records(
    filter_records: &mut crate::filters::wienerns_lr::FrameFilterRecords,
    row_filter_records: &mut TileFilterRecords,
) {
    filter_records
        .deblock_blocks
        .append(&mut row_filter_records.deblock_blocks);
    filter_records.chroma_deblock_blocks[0]
        .append(&mut row_filter_records.chroma_deblock_blocks[0]);
    filter_records.chroma_deblock_blocks[1]
        .append(&mut row_filter_records.chroma_deblock_blocks[1]);
    filter_records
        .tx_skip_records
        .append(&mut row_filter_records.tx_skip_records);
}

/// Commits one parsed row into the frame, in tile order.
#[allow(clippy::too_many_arguments)]
pub(super) fn replay_recon_row<T: ReconSample>(
    mut row: ReconRow,
    expected_ordinal: &mut usize,
    decoded_any: &mut bool,
    quantizer: &FrameQuantizerSnapshot,
    scratch: &mut deferred_recon::InterReconScratch<T>,
    workspace: &mut CurrentFrameWorkspace<T>,
    block_decoded: &mut TileBlockDecodedState,
    current_superblock: &mut Option<[usize; 2]>,
    motion_field: &mut TemporalMotionField,
    filter_records: &mut crate::filters::wienerns_lr::FrameFilterRecords,
    temporal_context: &TemporalMvContext,
    reference: &InterReferenceState<T>,
    ref_frame_idx: &[u32],
    sequence: &SequenceHeader,
    core: &FrameHeaderCore,
    mi_rows: usize,
    mi_cols: usize,
    current_order_hint: u32,
    luma_use_tcq: bool,
    residual_use_ddt: bool,
    bit_depth: BitDepth,
    tile_offset: ByteOffset,
) -> Result<ReconRowBuffers> {
    if row.ordinal != *expected_ordinal {
        return Err(inter_cap!(
            "inter_row_recon_order",
            tile_offset,
            "inter.row.recon_order",
            SPEC_MODE_INFO
        ));
    }
    *expected_ordinal = expected_ordinal.saturating_add(1);
    let terminal = row.terminal.take();
    let row_has_entries = !row.superblocks.is_empty();
    let _quantizer_scopes = quantizer.install_frame();
    let ReconRow {
        mut superblocks,
        mut entries,
        mut motion_queue,
        mut pending_inter,
        mut residual_blocks,
        mut temporal,
        mut flag_log,
        filter_records: mut row_filter_records,
        ..
    } = row;
    for superblock in &superblocks {
        let superblock_entries = entries.get_mut(superblock.entries.clone()).ok_or_else(|| {
            inter_cap!(
                "inter_row_replay_entry_range",
                tile_offset,
                "inter.row.task_capacity",
                SPEC_MODE_INFO
            )
        })?;
        debug_assert!(
            superblock_entries
                .iter()
                .all(|entry| entry.publication.superblock_origin() == superblock.origin)
        );
        for entry in superblock_entries {
            entry
                .publication
                .prepare_block_decoded(block_decoded, current_superblock);
            if let Some(error) = entry.error.take() {
                return Err(error);
            }
            if let Some(command) = entry.command.take() {
                match command {
                    ReconCommand::GeneralIntra(command) => {
                        let _scope = crate::timing::WalkPhaseScope::new(
                            crate::timing::WalkPhase::CommitIntra,
                        );
                        command.reconstruct(
                            scratch.general_intra_mut(),
                            workspace,
                            block_decoded,
                        )?;
                    }
                    ReconCommand::Intrabc(command) => {
                        let _scope = crate::timing::WalkPhaseScope::new(
                            crate::timing::WalkPhase::CommitIntrabc,
                        );
                        scratch.reconstruct_intrabc(command, &residual_blocks, workspace)?;
                    }
                    ReconCommand::Inter(command) => {
                        let _scope = crate::timing::WalkPhaseScope::new(
                            crate::timing::WalkPhase::CommitInter,
                        );
                        scratch.reconstruct(
                            &command,
                            workspace,
                            block_decoded,
                            motion_field,
                            &residual_blocks,
                            temporal_context,
                            reference,
                            ref_frame_idx,
                            sequence,
                            core,
                            mi_rows,
                            mi_cols,
                            current_order_hint,
                            luma_use_tcq,
                            residual_use_ddt,
                            bit_depth,
                        )?;
                    }
                }
            } else {
                let _scope =
                    crate::timing::WalkPhaseScope::new(crate::timing::WalkPhase::CommitReplay);
                let records = temporal.get(entry.temporal.clone()).ok_or_else(|| {
                    inter_cap!(
                        "inter_row_replay_temporal_range",
                        tile_offset,
                        "inter.row.task_capacity",
                        SPEC_MODE_INFO
                    )
                })?;
                super::temporal::commit_temporal_motion_blocks(motion_field, records);
            }
            entry
                .publication
                .publish_block_decoded(block_decoded)
                .map_err(|_| {
                    inter_cap!(
                        "inter_row_block_decoded_publish",
                        tile_offset,
                        "inter.partition_walk",
                        SPEC_MODE_INFO
                    )
                })?;
        }
    }
    append_row_filter_records(filter_records, &mut row_filter_records);
    *decoded_any |= row_has_entries;
    if let Some(error) = terminal {
        return Err(error);
    }
    superblocks.clear();
    entries.clear();
    motion_queue.clear();
    pending_inter.clear();
    residual_blocks.clear();
    temporal.clear();
    flag_log.clear();
    Ok(ReconRowBuffers {
        superblocks,
        entries,
        motion_queue,
        pending_inter,
        residual_blocks,
        temporal,
        flag_log,
        filter_records: row_filter_records,
    })
}
