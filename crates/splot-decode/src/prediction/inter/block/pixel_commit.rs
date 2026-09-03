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
use splot_recon::{BitDepth, CurrentFrameWorkspace, ReconSample};

use super::super::InterReferenceState;
use super::super::find_mv_stack::TemporalMvContext;
use super::super::mc::WorkspaceSink;
use super::ReconCommand;
use super::deferred_recon;
use super::temporal::MotionFieldUnits;
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
    filter_records
        .chroma_deblock_blocks
        .append(&mut row_filter_records.chroma_deblock_blocks);
    filter_records
        .tx_skip_records
        .append(&mut row_filter_records.tx_skip_records);
}

/// Moves one parsed row's filter geometry into the frame owner before that row
/// enters the scheduled reconstruction graph.
///
/// The canonical replay then carries an empty row record owner, which makes
/// the complete frame geometry immutable while row tasks still own commands.
pub(super) fn detach_row_filter_records(
    row: &mut ReconRow,
    filter_records: &mut crate::filters::wienerns_lr::FrameFilterRecords,
) {
    append_row_filter_records(filter_records, &mut row.filter_records);
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
    motion: &MotionFieldUnits,
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
) -> Result<ReconRowBuffers> {
    if row.ordinal != *expected_ordinal {
        return Err(crate::DecodeHeaderStateError::InvalidInterTileSchedulingState.into());
    }
    row.return_terminal_error()?;
    let ordinal = row.ordinal;
    *expected_ordinal = expected_ordinal.saturating_add(1);
    let row_has_entries = !row.superblocks.is_empty();
    let motion_owed = !row.motion_folded;
    let motion_derived = row.motion_derived;
    let mut precompute_error = row.take_precompute_error();
    let _quantizer_scopes = quantizer.install_frame();
    let ReconRow {
        mut superblocks,
        mut entries,
        mut residual_blocks,
        mut temporal,
        mut motion_grids,
        mut flag_log,
        filter_records: mut row_filter_records,
        mut residual_planes,
        ..
    } = row;
    for superblock in &superblocks {
        let superblock_entries = entries
            .get_mut(superblock.entries.clone())
            .ok_or(crate::DecodeHeaderStateError::InvalidInterTileSchedulingState)?;
        debug_assert!(
            superblock_entries
                .iter()
                .all(|entry| entry.publication.superblock_origin() == superblock.origin)
        );
        for (offset, entry) in superblock_entries.iter_mut().enumerate() {
            entry
                .publication
                .prepare_block_decoded(block_decoded, current_superblock);
            if precompute_error
                .as_ref()
                .is_some_and(|(index, _)| *index == superblock.entries.start + offset)
                && let Some((_, error)) = precompute_error.take()
            {
                return Err(error);
            }
            let temporal_clear = if motion_owed {
                entry.temporal_clear_record(mi_rows, mi_cols, current_order_hint)
            } else {
                None
            };
            if let Some(clear) = temporal_clear {
                motion.fold_unit(ordinal, core::slice::from_ref(&clear));
            }
            if let Some(command) = entry.take_command() {
                match command {
                    ReconCommand::GeneralIntra(command) => {
                        command.reconstruct(
                            &mut residual_planes,
                            scratch.general_intra_mut(),
                            workspace,
                            block_decoded,
                        )?;
                    }
                    ReconCommand::Intrabc(command) => {
                        scratch.reconstruct_intrabc(command, &residual_blocks, workspace)?;
                    }
                    ReconCommand::Inter(command) => {
                        if motion_derived {
                            scratch.reconstruct_from_motion(
                                &command,
                                &mut WorkspaceSink::Frame(workspace),
                                block_decoded,
                                entry.take_motion(&mut motion_grids),
                                &residual_blocks,
                                &deferred_recon::ReconShared {
                                    reference,
                                    ref_frame_idx,
                                    temporal_context,
                                    sequence,
                                    core,
                                    luma_use_tcq,
                                    residual_use_ddt,
                                    bit_depth,
                                    mi_rows,
                                    mi_cols,
                                    current_order_hint,
                                },
                            )?;
                        } else {
                            scratch.reconstruct(
                                &command,
                                workspace,
                                block_decoded,
                                motion,
                                ordinal,
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
                }
            } else {
                let records = temporal
                    .get(entry.temporal.clone())
                    .ok_or(crate::DecodeHeaderStateError::InvalidInterTileSchedulingState)?;
                if motion_owed {
                    motion.fold_unit(ordinal, records);
                }
            }
            entry
                .publication
                .publish_block_decoded(block_decoded)
                .map_err(|_| crate::DecodeHeaderStateError::InvalidBlockGeometry)?;
        }
    }
    if motion_owed {
        motion.unit_landed_for(ordinal);
    }
    append_row_filter_records(filter_records, &mut row_filter_records);
    *decoded_any |= row_has_entries;
    residual_planes.clear();
    superblocks.clear();
    entries.clear();
    residual_blocks.clear();
    temporal.clear();
    motion_grids.clear();
    flag_log.clear();
    Ok(ReconRowBuffers {
        superblocks,
        entries,
        residual_blocks,
        temporal,
        motion_grids,
        flag_log,
        filter_records: row_filter_records,
        residual_planes,
    })
}
