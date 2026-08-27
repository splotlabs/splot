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
//! [`RowReferenceGate::bounds_for_row`] derives that requirement per reference
//! list, as the last luma row of the reference the row's reads can touch. It
//! runs after the row's § 7.12 resolve pass, so warp models, final motion
//! vectors and the § 7.13.5 TIP motion field are all settled by then:
//!
//! - single-reference translational prediction is exact: the row's blocks read
//!   through the same § 7.13.3.18 scaling the prediction derives, so the bound
//!   is [`subpel_last_row`] over the resolved motion vector;
//! - compound prediction uses [`compound_last_row`], which adds the vertical
//!   excursion § 7.13.3.6 refine-MV and § 7.13.3.9 optical-flow refinement can
//!   reach past the resolved motion vector;
//! - a warped list adds [`warp_last_row`], the largest row the § 7.13.3.23
//!   model projects the block's plane rectangle onto;
//! - a § 7.13.3.25 BAWP list adds [`bawp_reference_luma_rows`], the template
//!   window below the block's full-pel reference position;
//! - a § 7.13.5 TIP block reads the frame's TIP reference pair through the
//!   projected motion field, so it is bounded band by band over the 8x8 field
//!   cells its prediction units sample.
//!
//! What is left is genuinely unboundable here: a list that resolves to no slot,
//! a scaled reference read through the § 7.13.3.20 extended warp, and a TIP
//! block whose reference pair or motion field is missing. Those rows require
//! their references to settle.
//!
//! The requirement is an upper bound on the rows a read can reach, never a
//! licence to read: the per-block admission check in
//! [`ReferenceSamples::plane_view`](super::super::reference::ReferenceSamples)
//! still fails closed on every read, so an under-tight bound here is a
//! diagnostic and never a wrong sample.

use core::sync::atomic::{AtomicUsize, Ordering};

use splot_core::headers::frame::FrameHeaderCore;
use splot_recon::{DecodedFrameInfo, PlaneId, ReconSample, ReferenceSlot};

use super::super::bawp::bawp_reference_luma_rows;
use super::super::find_mv_stack::{TemporalMvContext, TipReferencePair};
use super::super::mc::{McBlockRect, mc_planes};
use super::super::mv_scaling::derive_plane_scaling;
use super::super::reference::{compound_last_row, subpel_last_row, warp_last_row};
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

/// Luma side of the § 7.13.5 motion-field cell one TIP prediction unit samples.
const TIP_FIELD_CELL: usize = 8;

/// Largest § 7.13.5 prediction unit, the luma rows one field cell's motion
/// vector can predict.
const TIP_MAX_UNIT: usize = 16;

/// Why one block's reference reads cannot be bounded from resolved data.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SettleReason {
    /// § 7.13.3.20 extended warp resamples a scaled reference, which the
    /// § 7.13.3.23 corner projection does not bound.
    Warp,
    /// § 7.13.5 TIP synthesis has no reference pair or motion field to project.
    Tip,
    /// The block's reference list does not resolve to a known slot.
    Slot,
}

/// Why rows had to wait for whole reference frames instead of published rows.
#[derive(Default)]
struct RowGateFallbacks {
    warp: AtomicUsize,
    tip: AtomicUsize,
    slot: AtomicUsize,
}

impl RowGateFallbacks {
    fn note(&self, reason: SettleReason) {
        let counter = match reason {
            SettleReason::Warp => &self.warp,
            SettleReason::Tip => &self.tip,
            SettleReason::Slot => &self.slot,
        };
        counter.fetch_add(1, Ordering::Relaxed);
    }

    fn summary(&self) -> String {
        format!(
            "settle_warp={} settle_tip={} settle_slot={}",
            self.warp.load(Ordering::Relaxed),
            self.tip.load(Ordering::Relaxed),
            self.slot.load(Ordering::Relaxed),
        )
    }
}

/// How far past its plane rectangle one list's reads reach.
#[derive(Clone, Copy, Default)]
struct ListReach {
    /// Whether § 7.13.3.16 compound refinement may move the read.
    compound: bool,
    /// The list's § 7.13.3.23 warp model, when it has one.
    warp: Option<[i32; 6]>,
    /// Luma rows the § 7.13.3.25 BAWP template needs, or zero.
    bawp: u32,
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

impl RowReferenceBounds {
    pub(super) fn merge(&mut self, other: Self) {
        for (need, other) in self.needs.iter_mut().zip(other.needs) {
            *need = (*need).max(other);
        }
        self.settle |= other.settle;
    }
}

/// One frame's reference lists, resolved to the slots its rows read.
pub(super) struct RowReferenceGate<'a, T: ReconSample> {
    lists: [Option<&'a RefFrameSlot<T>>; MAX_LISTS],
    settle: PixelReferenceGate<'a, T>,
    frame: DecodedFrameInfo,
    temporal: &'a TemporalMvContext,
    tip: Option<TipReferencePair>,
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
        temporal: &'a TemporalMvContext,
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
            temporal,
            tip: temporal.tip_references(),
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

    /// Returns the scheduler conditions that replace waiting for `bounds`.
    pub(super) fn conditions(
        &self,
        bounds: &RowReferenceBounds,
    ) -> Vec<splot_parallel::Condition<'a>> {
        if bounds.settle {
            return self.settle.conditions();
        }
        self.lists
            .iter()
            .zip(bounds.needs)
            .filter_map(|(slot, need)| {
                (need != 0)
                    .then(|| slot.map(|slot| slot.row_condition(need as usize)))
                    .flatten()
            })
            .collect()
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
            if let Some(super::ReconCommand::Inter(command)) = entry.command() {
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
        if command.is_tip() {
            self.note_tip(placed, bounds);
            return;
        }
        let compound = block.ref_frame1.is_some();
        let bawp = if block.bawp.enabled {
            bawp_reference_luma_rows(placed.luma_y, placed.luma_h, block.mv.row)
        } else {
            0
        };
        let reach = ListReach {
            compound,
            warp: block.warp_params[0],
            bawp,
        };
        self.note_list(
            bounds,
            block.ref_frame0,
            block.mv,
            placed.motion_compensation_rect(),
            placed.predict_chroma,
            reach,
        );
        if let Some(ref_frame1) = block.ref_frame1 {
            self.note_list(
                bounds,
                ref_frame1,
                block.mv1,
                placed.motion_compensation_rect(),
                placed.predict_chroma,
                ListReach {
                    compound,
                    warp: block.warp_params[1],
                    bawp: 0,
                },
            );
        }
    }

    /// Bounds one § 7.13.5 TIP block against the frame's TIP reference pair.
    ///
    /// The block's prediction units read the projected motion field at
    /// [`TIP_FIELD_CELL`] granularity and predict at most [`TIP_MAX_UNIT`] rows
    /// from each cell they sample, so each band of cells is bounded as a
    /// [`TIP_MAX_UNIT`]-tall rectangle carrying that band's furthest-reaching
    /// projected vector. The batched optical-flow path also opens the whole
    /// block from its first unit's candidate before applying that grid, so the
    /// same candidate bounds the full block rectangle. Both lists are bounded
    /// even when the frame's weighting leaves the future reference unread,
    /// which only delays admission.
    fn note_tip(&self, placed: &PlacedInterBlock, bounds: &mut RowReferenceBounds) {
        let Some(references) = self.tip else {
            self.fallbacks.note(SettleReason::Tip);
            bounds.settle = true;
            return;
        };
        let base = placed.block.mv;
        let Some(batch_mvs) = self.temporal.tip_candidate(
            placed.luma_y / TIP_FIELD_CELL,
            placed.luma_x / TIP_FIELD_CELL,
            base,
        ) else {
            self.fallbacks.note(SettleReason::Tip);
            bounds.settle = true;
            return;
        };
        let batch_reach = ListReach {
            compound: true,
            ..ListReach::default()
        };
        for (list, mv) in [references.past_ref, references.future_ref]
            .into_iter()
            .zip(batch_mvs)
        {
            self.note_list(
                bounds,
                list,
                mv,
                placed.motion_compensation_rect(),
                placed.predict_chroma,
                batch_reach,
            );
        }
        for band in (0..placed.luma_h).step_by(TIP_FIELD_CELL) {
            let luma_y = placed.luma_y.saturating_add(band);
            let mut furthest = [Mv::ZERO; 2];
            for column in (0..placed.luma_w).step_by(TIP_FIELD_CELL) {
                let luma_x = placed.luma_x.saturating_add(column);
                let Some(mvs) = self.temporal.tip_candidate(
                    luma_y / TIP_FIELD_CELL,
                    luma_x / TIP_FIELD_CELL,
                    base,
                ) else {
                    self.fallbacks.note(SettleReason::Tip);
                    bounds.settle = true;
                    return;
                };
                for (reach, candidate) in furthest.iter_mut().zip(mvs) {
                    reach.row = reach.row.max(candidate.row);
                }
            }
            let rect =
                McBlockRect::from_luma_rect(placed.luma_x, luma_y, placed.luma_w, TIP_MAX_UNIT);
            let reach = ListReach {
                compound: true,
                ..ListReach::default()
            };
            for (list, mv) in [references.past_ref, references.future_ref]
                .into_iter()
                .zip(furthest)
            {
                self.note_list(bounds, list, mv, rect, placed.predict_chroma, reach);
            }
        }
    }

    fn note_list(
        &self,
        bounds: &mut RowReferenceBounds,
        ref_frame: i8,
        mv: Mv,
        rect: McBlockRect,
        predict_chroma: bool,
        reach: ListReach,
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
        let Some(rows) =
            block_published_rows(self.frame, slot.info(), rect, predict_chroma, mv, reach)
        else {
            self.fallbacks.note(SettleReason::Warp);
            bounds.settle = true;
            return;
        };
        if let Some(entry) = bounds.needs.get_mut(list) {
            *entry = (*entry).max(rows);
        }
    }
}

/// The luma rows one list's reference must have published for one block
/// rectangle, over every plane the block predicts.
///
/// Returns `None` when a warped list resamples a scaled reference, which
/// [`warp_last_row`] does not bound.
fn block_published_rows(
    frame: DecodedFrameInfo,
    reference: DecodedFrameInfo,
    rect: McBlockRect,
    predict_chroma: bool,
    mv: Mv,
    reach: ListReach,
) -> Option<u32> {
    let reference_size = reference.coded_luma_size();
    let frame_size = frame.coded_luma_size();
    let mut rows = if reach.bawp == 0 {
        0
    } else {
        let visible_y = u32::try_from(reference.visible_luma_rect().y()).unwrap_or(u32::MAX);
        reach
            .bawp
            .saturating_add(visible_y)
            .min(reference_size.height() as u32)
    };
    for (plane, sub_x, sub_y) in mc_planes(frame.pixel_format()) {
        if plane == PlaneId::V || (plane != PlaneId::Y && !predict_chroma) {
            continue;
        }
        let (plane_x, plane_y, width, height) = rect.plane_rect(plane, sub_x, sub_y);
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
        let mut last = if reach.compound {
            compound_last_row(scaling.start_y, scaling.step_y, height, scaling.last_y)
        } else {
            subpel_last_row(scaling.start_y, scaling.step_y, height, scaling.last_y)
        };
        if let Some(warp_params) = reach.warp {
            if scaling.is_scaled() {
                return None;
            }
            last = last.max(warp_last_row(
                warp_params,
                (plane_x, plane_y, width, height),
                sub_x,
                sub_y,
                scaling.last_y,
            ));
        }
        let plane_visible_y =
            u32::try_from(reference.visible_luma_rect().y() >> sub_y).unwrap_or(u32::MAX);
        let plane_rows = (last.max(0) as u32)
            .saturating_add(1)
            .saturating_add(plane_visible_y);
        rows = rows.max(
            plane_rows
                .saturating_mul(1 << sub_y)
                .min(reference_size.height() as u32),
        );
    }
    Some(rows)
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
