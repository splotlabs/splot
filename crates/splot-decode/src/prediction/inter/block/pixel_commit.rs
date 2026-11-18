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
//! the frame's motion field, and advances the block-decoded availability
//! grid.

use splot_core::headers::frame::FrameHeaderCore;
use splot_core::headers::sequence::SequenceHeader;
use splot_recon::{BitDepth, CurrentFrameWorkspace, ReconSample};

use super::super::InterReferenceState;
use super::super::find_mv_stack::TemporalMvContext;
use super::super::mc::WorkspaceSink;
use super::deferred_recon;
use super::temporal::MotionFieldUnits;
use super::tile::{ReconRow, ReconRowBuffers};
use super::{ReconCommandMut, ReconCommandRef};
use crate::Result;
use crate::bitstream::tile_payload::TileBlockDecodedState;

/// Commits one parsed row into the frame, in tile order.
#[allow(clippy::too_many_arguments)]
pub(super) fn replay_recon_row<T: ReconSample>(
    mut row: ReconRow,
    tile_offset: splot_core::span::ByteOffset,
    expected_ordinal: &mut usize,
    decoded_any: &mut bool,
    scratch: &mut deferred_recon::InterReconScratch<T>,
    workspace: &mut CurrentFrameWorkspace<T>,
    block_decoded: &mut TileBlockDecodedState,
    motion: &MotionFieldUnits,
    temporal_context: &TemporalMvContext,
    reference: &InterReferenceState<T>,
    lists: (&[u32], &super::FrameRefFacts),
    sequence: &SequenceHeader,
    core: &FrameHeaderCore,
    mi_rows: usize,
    mi_cols: usize,
    current_order_hint: u32,
    luma_use_tcq: bool,
    residual_use_ddt: bool,
    bit_depth: BitDepth,
) -> Result<ReconRowBuffers> {
    let mut folded = std::mem::take(&mut scratch.temporal);
    folded.clear();
    let mut has_intra = false;
    let result = (|| {
        let ordinal = row.ordinal;
        *expected_ordinal = expected_ordinal.saturating_add(1);
        let row_has_entries = !row.superblocks.is_empty();
        let motion_owed = !row.motion_folded;
        let mut precompute_error = row.take_precompute_error();
        let ReconRow {
            mut superblocks,
            mut residual_coeffs,
            mut entries,
            mut slots,
            mut residual_blocks,
            mut temporal,
            mut residual_planes,
            mut filter_records,
            ..
        } = row;
        let shared = deferred_recon::ReconShared {
            tile_offset,
            reference,
            ref_frame_idx: lists.0,
            ref_facts: lists.1,
            held: super::super::hold_row_references(reference, lists),
            temporal_context,
            sequence,
            core,
            luma_use_tcq,
            residual_use_ddt,
            bit_depth,
            mi_rows,
            mi_cols,
            current_order_hint,
            tip: core::cell::OnceCell::new(),
        };
        for superblock in &superblocks {
            let superblock_entries = entries
                .get_mut(superblock.entries.clone())
                .ok_or(crate::DecodeHeaderStateError::InvalidInterTileSchedulingState)?;
            let superblock_needs_decoded =
                superblock_entries
                    .iter()
                    .any(|entry| match entry.command(&slots) {
                        Some(ReconCommandRef::GeneralIntra) => true,
                        Some(ReconCommandRef::Inter(c)) => c.reads_current_frame(),
                        Some(ReconCommandRef::Intrabc) | None => false,
                    });
            if superblock_needs_decoded {
                let [r, c] = superblock.origin;
                block_decoded.clear_superblock(r, c);
            }
            for (offset, entry) in superblock_entries.iter_mut().enumerate() {
                if precompute_error
                    .as_ref()
                    .is_some_and(|(index, _)| *index == superblock.entries.start + offset)
                    && let Some((_, error)) = precompute_error.take()
                {
                    return Err(error);
                }
                let temporal_clear = if motion_owed {
                    entry.temporal_clear_record(
                        superblock.origin,
                        &slots,
                        mi_rows,
                        mi_cols,
                        current_order_hint,
                    )
                } else {
                    None
                };
                if let Some(clear) = temporal_clear {
                    folded.push(clear);
                }
                if let Some(command) = entry.command_mut(&mut slots) {
                    match command {
                        ReconCommandMut::GeneralIntra(command) => {
                            if !has_intra {
                                crate::pipeline::general_intra::swap_general_intra_scratch(
                                    scratch.general_intra_mut(),
                                    workspace,
                                );
                                has_intra = true;
                            }
                            command.reconstruct(
                                &mut residual_planes,
                                &residual_coeffs,
                                scratch.general_intra_mut(),
                                workspace,
                                block_decoded,
                                tile_offset,
                            )
                        }
                        ReconCommandMut::Intrabc(command) => {
                            command.reconstruct(&residual_blocks, &residual_coeffs, workspace)
                        }
                        ReconCommandMut::Inter(command) => scratch.reconstruct(
                            command,
                            &mut WorkspaceSink::Frame(workspace),
                            block_decoded,
                            &mut folded,
                            &residual_blocks,
                            &residual_coeffs,
                            &shared,
                        ),
                    }?;
                }
                if superblock_needs_decoded {
                    entry
                        .publication
                        .publish_block_decoded(block_decoded)
                        .map_err(|_| crate::DecodeHeaderStateError::InvalidBlockGeometry)?;
                }
            }
        }
        if let Some((_, error)) = precompute_error {
            return Err(error);
        }
        if has_intra {
            crate::pipeline::general_intra::swap_general_intra_scratch(
                scratch.general_intra_mut(),
                workspace,
            );
            has_intra = false;
        }
        if !motion_owed && row_has_entries && !temporal.is_empty() {
            motion.fold_unit(ordinal, &temporal)?;
        }
        if !folded.is_empty() {
            motion.fold_unit(ordinal, &folded)?;
        }
        if motion_owed || row_has_entries {
            motion.unit_landed_for(ordinal);
        }
        *decoded_any |= row_has_entries;
        let [luma, chroma, tx_skip, leaves] = filter_records.published;
        let [planes, units] = residual_planes.lengths();
        let lengths = [
            residual_coeffs.len(),
            entries.len(),
            slots.len(),
            residual_blocks.len(),
            temporal.len(),
            planes,
            units,
            luma,
            chroma,
            tx_skip,
            leaves,
        ];
        residual_planes.clear();
        superblocks.clear();
        entries.clear();
        slots.clear();
        residual_blocks.clear();
        residual_coeffs.clear();
        temporal.clear();
        filter_records.clear();
        Ok(ReconRowBuffers {
            superblocks,
            residual_coeffs,
            entries,
            slots,
            residual_blocks,
            temporal,
            residual_planes,
            filter_records,
            lengths,
        })
    })();
    if has_intra {
        crate::pipeline::general_intra::swap_general_intra_scratch(
            scratch.general_intra_mut(),
            workspace,
        );
    }
    folded.clear();
    scratch.temporal = folded;
    result
}
