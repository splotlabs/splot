// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Per-row reference admission for the walk's ready-row engine.
//!
//! Feature tracking: `INFRA-DECODE-FRAME-PIPELINING`.
//!
//! A frame's entropy walk parses without reference samples; only its
//! reconstruction reads them. Waiting for a whole reference frame to settle
//! before any reconstruction starts throws away the prefix a still-filtering
//! reference has already published, so each parsed row instead carries the
//! reference rows its own reconstruction will reach, and is released as soon as
//! its references have published them.
//!
//! [`RowReferenceGate::bounds_for_row`] derives that requirement from parse data
//! alone, per reference list, as the last luma row of the reference the row's
//! § 7.13.3.18 subpel reads can touch:
//!
//! - single-reference translational prediction is exact: the row's blocks read
//!   through the same § 7.13.3.18 scaling the prediction derives, so the bound
//!   is [`subpel_last_row`] over the parsed motion vector;
//! - compound prediction uses [`compound_last_row`], which adds the vertical
//!   excursion § 7.13.3.6 refine-MV and § 7.13.3.9 optical-flow refinement can
//!   reach past the parsed motion vector;
//! - warped, BAWP and § 7.13.5 TIP blocks read through geometry the parse data
//!   does not bound, so a row holding one requires its references to settle.
//!
//! The requirement is an upper bound on the rows a read can reach, never a
//! licence to read: the per-block admission check in
//! [`ReferenceSamples::plane_view`](super::super::reference::ReferenceSamples)
//! still fails closed on every read, so an under-tight bound here is a
//! diagnostic and never a wrong sample.

use core::sync::atomic::{AtomicUsize, Ordering};

use splot_core::headers::frame::FrameHeaderCore;
use splot_recon::{DecodedFrameInfo, PlaneId, ReconSample, ReferenceSlot};

use super::super::mc::mc_planes;
use super::super::mv_scaling::derive_plane_scaling;
use super::super::reference::{compound_last_row, subpel_last_row};
use super::super::{
    InterReferenceState, Mv, PixelReferenceGate, PlacedInterBlock, named_pixel_reference_slots,
};
use super::deferred_recon::InterReconCommand;
use super::tile::ReconRow;
use crate::Result;
use crate::pipeline::inflight::RefFrameSlot;

/// Reference lists a frame header can name: `REFS_PER_FRAME` (AV2 § 3) plus the
/// bridge slot.
const MAX_LISTS: usize = 8;

/// Why one block's reference reads cannot be bounded from parse data.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SettleReason {
    /// § 7.13.3.19 warped motion reads through a model, not a translation.
    Warp,
    /// § 7.13.5 TIP synthesis reads through the TIP motion field.
    Tip,
    /// § 7.13.3.29 BAWP derives its own reference window.
    Bawp,
    /// The block's reference list does not resolve to a known slot.
    Slot,
}

/// Why rows had to wait for whole reference frames instead of published rows.
#[derive(Default)]
struct RowGateFallbacks {
    warp: AtomicUsize,
    tip: AtomicUsize,
    bawp: AtomicUsize,
    slot: AtomicUsize,
}

impl RowGateFallbacks {
    fn note(&self, reason: SettleReason) {
        let counter = match reason {
            SettleReason::Warp => &self.warp,
            SettleReason::Tip => &self.tip,
            SettleReason::Bawp => &self.bawp,
            SettleReason::Slot => &self.slot,
        };
        counter.fetch_add(1, Ordering::Relaxed);
    }

    fn summary(&self) -> String {
        format!(
            "settle_warp={} settle_tip={} settle_bawp={} settle_slot={}",
            self.warp.load(Ordering::Relaxed),
            self.tip.load(Ordering::Relaxed),
            self.bawp.load(Ordering::Relaxed),
            self.slot.load(Ordering::Relaxed),
        )
    }
}

/// Whether one block's prediction reads through geometry the parse data does
/// not bound, and if so which reader.
fn settle_reason(command: &InterReconCommand) -> Option<SettleReason> {
    let block = &command.placed().block;
    if command.is_tip() {
        Some(SettleReason::Tip)
    } else if block.bawp.enabled {
        Some(SettleReason::Bawp)
    } else if block.warp_params.iter().any(Option::is_some) {
        Some(SettleReason::Warp)
    } else {
        None
    }
}

/// One parsed row's reference requirement.
///
/// `needs[list]` is the number of luma rows the reference behind that list must
/// have published before the row's reconstruction may run, and zero when the
/// row reads nothing through that list.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct RowReferenceBounds {
    needs: [u32; MAX_LISTS],
    settle: bool,
}

/// One frame's reference lists, resolved to the slots its rows read.
pub(super) struct RowReferenceGate<'a, T: ReconSample> {
    lists: [Option<&'a RefFrameSlot<T>>; MAX_LISTS],
    settle: PixelReferenceGate<'a, T>,
    frame: DecodedFrameInfo,
    fallbacks: RowGateFallbacks,
}

impl<'a, T: ReconSample> RowReferenceGate<'a, T> {
    /// Resolves the frame's reference lists, keeping the whole-frame gate the
    /// unbounded readers and the end-of-walk drain still need.
    pub(super) fn new(
        reference: &'a InterReferenceState<T>,
        core: &FrameHeaderCore,
        ref_frame_idx: &[u32],
        frame: DecodedFrameInfo,
    ) -> Self {
        let mut lists = [None; MAX_LISTS];
        for (entry, slot) in lists.iter_mut().zip(ref_frame_idx) {
            *entry = ReferenceSlot::new(*slot as usize)
                .ok()
                .and_then(|slot| reference.store.get(slot).ok().flatten());
        }
        Self {
            lists,
            settle: reference.pixel_reference_gate(named_pixel_reference_slots(core)),
            frame,
            fallbacks: RowGateFallbacks::default(),
        }
    }

    /// Whether every reference row `bounds` requires has been published.
    pub(super) fn admits(&self, bounds: &RowReferenceBounds) -> bool {
        if bounds.settle {
            return self.settle.is_ready();
        }
        lists_published(&self.lists, bounds)
    }

    /// Whether every named reference frame has settled, which admits every row.
    pub(super) fn is_ready(&self) -> bool {
        self.settle.is_ready()
    }

    /// Blocks the calling driver thread until every named reference frame has
    /// settled, which admits every row.
    ///
    /// # Errors
    ///
    /// Returns the referenced frame's own filter-phase diagnostic.
    pub(super) fn wait(&self, arm: &str) -> Result<()> {
        self.settle.wait(arm)
    }

    /// The per-list reference-row requirement of one parsed row.
    ///
    /// A frame whose references have all settled by the time the row is parsed
    /// requires nothing: settled slots never unsettle, so the empty requirement
    /// admits the row for good and the walk skips the whole derivation.
    pub(super) fn bounds_for_row(&self, row: &ReconRow) -> RowReferenceBounds {
        let mut bounds = RowReferenceBounds::default();
        if self.is_ready() {
            return bounds;
        }
        for entry in &row.entries {
            if let Some(super::ReconCommand::Inter(command)) = entry.command.as_ref() {
                self.note_block(command, &mut bounds);
            }
        }
        bounds
    }

    /// Reports how many blocks fell back to whole-frame settling, by reason.
    pub(super) fn fallback_summary(&self) -> String {
        self.fallbacks.summary()
    }

    fn note_block(&self, command: &InterReconCommand, bounds: &mut RowReferenceBounds) {
        let placed = command.placed();
        let block = &placed.block;
        if let Some(reason) = settle_reason(command) {
            self.fallbacks.note(reason);
            bounds.settle = true;
            return;
        }
        let compound = block.ref_frame1.is_some();
        self.note_list(bounds, block.ref_frame0, block.mv, placed, compound);
        if let Some(ref_frame1) = block.ref_frame1 {
            self.note_list(bounds, ref_frame1, block.mv1, placed, compound);
        }
    }

    fn note_list(
        &self,
        bounds: &mut RowReferenceBounds,
        ref_frame: i8,
        mv: Mv,
        placed: &PlacedInterBlock,
        compound: bool,
    ) {
        let list = usize::try_from(ref_frame).ok();
        let slot = list.and_then(|list| self.lists.get(list).copied().flatten());
        let Some((list, slot)) = list.zip(slot) else {
            self.fallbacks.note(SettleReason::Slot);
            bounds.settle = true;
            return;
        };
        if slot.is_settled() {
            return;
        }
        let rows = block_published_rows(self.frame, slot.info(), placed, mv, compound);
        if let Some(entry) = bounds.needs.get_mut(list) {
            *entry = (*entry).max(rows);
        }
    }
}

/// The luma rows one block's reference must have published, over every plane
/// the block predicts.
fn block_published_rows(
    frame: DecodedFrameInfo,
    reference: DecodedFrameInfo,
    placed: &PlacedInterBlock,
    mv: Mv,
    compound: bool,
) -> u32 {
    let rect = placed.motion_compensation_rect();
    let reference_size = reference.coded_luma_size();
    let frame_size = frame.coded_luma_size();
    let mut rows = 0u32;
    for (plane, sub_x, sub_y) in mc_planes(frame.pixel_format()) {
        if plane == PlaneId::V || (plane != PlaneId::Y && !placed.predict_chroma) {
            continue;
        }
        let (plane_x, plane_y, _, height) = rect.plane_rect(plane, sub_x, sub_y);
        let scaling = derive_plane_scaling(
            plane_x as i32,
            plane_y as i32,
            mv.row,
            mv.col,
            sub_x,
            sub_y,
            reference_size.width() as i32,
            reference_size.height() as i32,
            frame_size.width() as i32,
            frame_size.height() as i32,
        );
        let last = if compound {
            compound_last_row(scaling.start_y, scaling.step_y, height, scaling.last_y)
        } else {
            subpel_last_row(scaling.start_y, scaling.step_y, height, scaling.last_y)
        };
        rows = rows.max((last.max(0) as u32).saturating_add(1) << sub_y);
    }
    rows
}

/// Whether every list the row reads has published the rows it needs.
fn lists_published<T: ReconSample>(
    lists: &[Option<&RefFrameSlot<T>>; MAX_LISTS],
    bounds: &RowReferenceBounds,
) -> bool {
    lists.iter().zip(bounds.needs).all(|(slot, need)| {
        need == 0
            || slot.is_some_and(|slot| {
                slot.is_settled() || slot.published_luma_rows() >= need as usize
            })
    })
}

#[cfg(test)]
#[path = "row_gate_tests.rs"]
mod tests;
