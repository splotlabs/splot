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

pub(super) fn take_tile_filter_records(
    filter_records: &mut crate::filters::wienerns_lr::FrameFilterRecords,
    tile_records: &mut TileFilterRecords,
) {
    if filter_records.deblock_blocks.is_empty()
        && filter_records.chroma_deblock_blocks.is_empty()
        && filter_records.tx_skip_records.is_empty()
    {
        core::mem::swap(
            &mut filter_records.deblock_blocks,
            &mut tile_records.deblock_blocks,
        );
        core::mem::swap(
            &mut filter_records.chroma_deblock_blocks,
            &mut tile_records.chroma_deblock_blocks,
        );
        core::mem::swap(
            &mut filter_records.tx_skip_records,
            &mut tile_records.tx_skip_records,
        );
    } else {
        append_row_filter_records(filter_records, tile_records);
    }
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
    let mut coefficient_scratch = std::mem::take(&mut scratch.coefficients);
    let mut plane_scratch = std::mem::take(&mut scratch.plane_records);
    let mut folded = std::mem::take(&mut scratch.temporal);
    folded.clear();
    let mut lists = std::mem::take(&mut scratch.lists);
    let mut has_intra = false;
    let result = (|| {
        if row.ordinal != *expected_ordinal {
            return Err(crate::DecodeHeaderStateError::InvalidInterTileSchedulingState.into());
        }
        row.return_terminal_error()?;
        row.load_parsed(&mut lists);
        row.return_terminal_error()?;
        let ordinal = row.ordinal;
        *expected_ordinal = expected_ordinal.saturating_add(1);
        let row_has_entries = !row.superblocks.is_empty();
        let motion_owed = !row.motion_folded;
        let mut precompute_error = row.take_precompute_error();
        let _quantizer_scopes = quantizer.install_frame();
        let ReconRow {
            mut superblocks,
            residual_source,
            mut residual_coeffs,
            mut entries,
            mut residual_blocks,
            mut temporal,
            mut flag_log,
            filter_records: mut row_filter_records,
            mut residual_planes,
            ..
        } = row;
        let shared_coefficients = residual_source.is_some();
        if let Some(source) = residual_source
            && entries.iter().any(|entry| entry.command().is_some())
        {
            source.copy_into(&mut coefficient_scratch)?;
            source.take_planes(&mut plane_scratch)?;
        }
        has_intra = entries
            .iter()
            .any(|entry| matches!(entry.command(), Some(ReconCommand::GeneralIntra(_))));
        if has_intra {
            crate::pipeline::general_intra::swap_general_intra_scratch(
                scratch.general_intra_mut(),
                workspace,
            );
        }
        for superblock in &superblocks {
            let superblock_entries = entries
                .get_mut(superblock.entries.clone())
                .ok_or(crate::DecodeHeaderStateError::InvalidInterTileSchedulingState)?;
            debug_assert!(
                superblock_entries
                    .iter()
                    .all(|entry| entry.publication.superblock_origin() == superblock.origin)
            );
            let superblock_needs_decoded = superblock_entries.iter().any(|entry| {
                matches!(entry.command(), Some(ReconCommand::GeneralIntra(_)))
                    || matches!(
                        entry.command(),
                        Some(ReconCommand::Inter(c)) if c.reads_current_frame()
                    )
            });
            for (offset, entry) in superblock_entries.iter_mut().enumerate() {
                if superblock_needs_decoded {
                    entry
                        .publication
                        .prepare_block_decoded(block_decoded, current_superblock);
                }
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
                    folded.push(clear);
                }
                let temporal_records = entry.temporal.clone();
                match entry.command_mut() {
                    Some(ReconCommand::GeneralIntra(command)) => {
                        command.reconstruct(
                            if shared_coefficients {
                                &mut plane_scratch
                            } else {
                                &mut residual_planes
                            },
                            if shared_coefficients {
                                &coefficient_scratch
                            } else {
                                &residual_coeffs
                            },
                            scratch.general_intra_mut(),
                            workspace,
                            block_decoded,
                        )?;
                    }
                    Some(ReconCommand::Intrabc(_)) => {
                        if let Some(ReconCommand::Intrabc(command)) = entry.take_command() {
                            scratch.reconstruct_intrabc(
                                command,
                                &residual_blocks,
                                if shared_coefficients {
                                    &coefficient_scratch
                                } else {
                                    &residual_coeffs
                                },
                                workspace,
                            )?;
                        }
                    }
                    Some(ReconCommand::Inter(command)) => {
                        let prederived_motion = command.has_derived_motion();
                        scratch.reconstruct(
                            command,
                            workspace,
                            block_decoded,
                            &mut folded,
                            &residual_blocks,
                            if shared_coefficients {
                                &coefficient_scratch
                            } else {
                                &residual_coeffs
                            },
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
                        if prederived_motion && motion_owed {
                            let records = temporal.get(temporal_records).ok_or(
                                crate::DecodeHeaderStateError::InvalidInterTileSchedulingState,
                            )?;
                            folded.extend_from_slice(records);
                        }
                    }
                    None => {
                        let records = temporal.get(temporal_records).ok_or(
                            crate::DecodeHeaderStateError::InvalidInterTileSchedulingState,
                        )?;
                        if motion_owed {
                            folded.extend_from_slice(records);
                        }
                    }
                }
                if superblock_needs_decoded {
                    entry
                        .publication
                        .publish_block_decoded(block_decoded)
                        .map_err(|_| crate::DecodeHeaderStateError::InvalidBlockGeometry)?;
                }
            }
        }
        if has_intra {
            crate::pipeline::general_intra::swap_general_intra_scratch(
                scratch.general_intra_mut(),
                workspace,
            );
            has_intra = false;
        }
        if !folded.is_empty() {
            motion.fold_unit(ordinal, &folded)?;
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
        residual_coeffs.clear();
        temporal.clear();
        flag_log.clear();
        if shared_coefficients {
            lists.swap_into(&mut entries, &mut residual_blocks);
        }
        Ok(ReconRowBuffers {
            superblocks,
            residual_coeffs,
            entries,
            residual_blocks,
            temporal,
            flag_log,
            filter_records: row_filter_records,
            residual_planes,
        })
    })();
    if has_intra {
        crate::pipeline::general_intra::swap_general_intra_scratch(
            scratch.general_intra_mut(),
            workspace,
        );
    }
    scratch.coefficients = coefficient_scratch;
    folded.clear();
    scratch.temporal = folded;
    plane_scratch.clear();
    scratch.plane_records = plane_scratch;
    scratch.lists = lists;
    result
}
