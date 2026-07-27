// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Reference-sample access for AV2 § 7.13.3 inter prediction.
//!
//! Feature tracking: `INFRA-DECODE-FRAME-PIPELINING`.
//!
//! Prediction reads a reference frame through [`ReferenceSamples`], which
//! resolves to one of two storages holding the same bytes: the settled
//! [`DecodedFrame`] a reference slot publishes, or the filtered workspace a
//! still-pipelined frame is publishing stripe by stripe.
//! [`CurrentFrameWorkspace::freeze`] moves that workspace's plane storage into
//! the frozen planes unchanged, so a row a stripe has already published reads
//! identically through either storage.
//!
//! A partially published frame keeps its true geometry: every clamp and step in
//! § 7.13.3.18 uses the whole frame's dimensions, so a banded read produces the
//! same samples as the settled read. What the band adds is a fail-closed
//! admission check. Reads clamp their row to the plane's last row
//! (`splot-recon` `subpel_mc`), so a view shortened to the published prefix
//! would silently substitute the wrong row instead of failing; the published
//! row count is therefore carried beside the true geometry and checked once per
//! block plane, against the last row that block's prediction can reach.

use splot_core::span::ByteOffset;
use splot_recon::{
    CurrentFrameWorkspace, DecodedFrame, DecodedFrameInfo, PlaneId, PlaneRect, ReconSample,
    ReferencePlaneView, SubpelPredictParams,
};

use super::mv_scaling::PlaneScaling;
use super::{SPEC_MC, unsupported_at};
use crate::Result;
use crate::pipeline::frame_progress::PublishedFrame;

/// Row bound for a reader that cannot bound its reference rows before the
/// prediction geometry exists: the whole plane must be final.
pub(crate) const ALL_ROWS: i32 = i32::MAX;

/// Fractional bits carried by a § 7.13.3.18 scaled reference position.
const SCALE_SUBPEL_BITS: u32 = 10;

/// Fractional bits carried by a § 7.13.3.23 affine warp model.
const WARPEDMODEL_PREC_BITS: u32 = 16;

/// Rows the § 7.13.3.18 vertical filter reaches past the last predicted row.
const SUBPEL_TAP_REACH: i64 = 4;

/// Rows a warp kernel reaches past the row its projected sampling point lands
/// on; see [`warp_last_row`] for the derivation.
const WARP_ROW_REACH: i64 = 7;

/// Current-frame rows a compound prediction reaches past its parsed motion
/// vector; see [`compound_last_row`] for the derivation.
const COMPOUND_REFINE_REACH: i32 = 8;

/// How far down a partially published frame its samples are final.
#[derive(Clone, Copy, Debug)]
struct PublishedRows {
    luma: usize,
    chroma: usize,
}

/// Which storage a reference frame's samples currently live in.
#[derive(Clone, Copy, Debug)]
enum SampleSource<'a, T: ReconSample> {
    /// The frame has settled into its slot.
    Frozen(&'a DecodedFrame<T>),
    /// The frame's filter phase still owns its output workspace.
    Filtering(&'a CurrentFrameWorkspace<T>),
}

/// One reference frame's samples as one block's prediction reads them.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ReferenceSamples<'a, T: ReconSample> {
    source: SampleSource<'a, T>,
    published: Option<PublishedRows>,
}

impl<'a, T: ReconSample> ReferenceSamples<'a, T> {
    /// Reads a settled reference frame, every row of which is final.
    pub(crate) fn settled(frame: &'a DecodedFrame<T>) -> Self {
        Self {
            source: SampleSource::Frozen(frame),
            published: forced_band(frame),
        }
    }

    /// Reads the published prefix of a frame whose filter phase is still
    /// running, leaving the frame's true geometry intact.
    pub(crate) const fn filtering(
        workspace: &'a CurrentFrameWorkspace<T>,
        luma: usize,
        chroma: usize,
    ) -> Self {
        Self {
            source: SampleSource::Filtering(workspace),
            published: Some(PublishedRows { luma, chroma }),
        }
    }

    /// Returns the reference frame's decoded geometry.
    pub(crate) fn info(self) -> DecodedFrameInfo {
        match self.source {
            SampleSource::Frozen(frame) => frame.info(),
            SampleSource::Filtering(workspace) => workspace.info(),
        }
    }

    /// Borrows one reference plane over its visible rectangle, together with the
    /// § 7.13.3.23 reference mode-info dimensions warp bounds are derived from.
    ///
    /// `last_row` is the last row of this plane the caller's prediction can
    /// reach, or [`ALL_ROWS`] when the caller cannot bound it before the
    /// geometry exists.
    ///
    /// # Errors
    ///
    /// Returns a capability diagnostic when the reference is missing a plane or
    /// its storage does not cover the visible rectangle, and a fail-closed
    /// diagnostic when a still-filtering reference has not published `last_row`.
    pub(crate) fn plane_view(
        self,
        plane: PlaneId,
        last_row: i32,
        offset: ByteOffset,
    ) -> Result<(ReferencePlaneView<'a, T>, i32, i32)> {
        let Some((samples, stride, visible)) = self.plane_storage(plane) else {
            return Err(missing_plane(offset));
        };
        let view = visible
            .y()
            .checked_mul(stride)
            .and_then(|row| row.checked_add(visible.x()))
            .and_then(|start| samples.get(start..))
            .ok_or(())
            .and_then(|samples| {
                ReferencePlaneView::from_strided(samples, stride, visible.width(), visible.height())
                    .map_err(|_| ())
            })
            .map_err(|()| plane_geometry(offset))?;
        self.ensure_published(plane, last_row, visible.height(), offset)?;

        let Some((_, _, luma_visible)) = self.plane_storage(PlaneId::Y) else {
            return Err(missing_plane(offset));
        };
        let ref_mi_cols = luma_visible.width().div_ceil(4) as i32;
        let ref_mi_rows = luma_visible.height().div_ceil(4) as i32;

        Ok((view, ref_mi_cols, ref_mi_rows))
    }

    /// Refuses a read whose rows a still-filtering reference has not published.
    fn ensure_published(
        self,
        plane: PlaneId,
        last_row: i32,
        plane_rows: usize,
        offset: ByteOffset,
    ) -> Result<()> {
        let Some(published) = self.published else {
            return Ok(());
        };
        let published = if plane == PlaneId::Y {
            published.luma
        } else {
            published.chroma
        };
        let needed = (last_row.max(0) as usize).saturating_add(1).min(plane_rows);
        if needed <= published {
            return Ok(());
        }
        Err(unpublished_rows(offset))
    }

    /// Borrows one plane's backing samples, stride, and visible rectangle.
    fn plane_storage(self, plane: PlaneId) -> Option<(&'a [T], usize, PlaneRect)> {
        match self.source {
            SampleSource::Frozen(frame) => frame.plane(plane).map(|plane| {
                (
                    plane.samples(),
                    plane.stride_samples(),
                    plane.visible_rect(),
                )
            }),
            SampleSource::Filtering(workspace) => workspace.plane(plane).ok().map(|plane| {
                (
                    plane.samples(),
                    plane.stride_samples(),
                    plane.visible_rect(),
                )
            }),
        }
    }
}

/// The last reference row § 7.13.3.18 subpel prediction reads for `params`.
///
/// The vertical filter reaches [`SUBPEL_TAP_REACH`] rows past the row the last
/// predicted row starts at, and every read is clamped to `params.last_y`, which
/// already carries the § 7.13.3.22 refine-MV window when one applies.
pub(crate) fn subpel_last_reference_row(params: &SubpelPredictParams) -> i32 {
    subpel_last_row(params.start_y, params.step_y, params.h, params.last_y)
}

/// The last reference row a § 7.13.3.18 subpel read reaches, from the vertical
/// scaling alone.
///
/// A caller that bounds a read before the prediction parameters exist derives
/// the same row from the same § 7.13.3.18 scaling the read will use.
pub(crate) fn subpel_last_row(start_y: i32, step_y: i32, height: usize, last_y: i32) -> i32 {
    let span = i64::from(step_y) * (height.saturating_sub(1) as i64);
    let last = (i64::from(start_y).saturating_add(span) >> SCALE_SUBPEL_BITS)
        .saturating_add(SUBPEL_TAP_REACH);
    last.min(i64::from(last_y)).max(0) as i32
}

/// The last reference row a § 7.13.3.16 compound plane prediction reads, over
/// every subblock the § 7.13.3.6 refine-MV and § 7.13.3.9 optical-flow passes
/// may split the block into.
///
/// Refine-MV clamps every refined read to the reference area of its unrefined
/// candidate, so refinement alone reaches no further than the parsed vector
/// does. Unrefined optical flow is not clamped that way and reaches further in
/// two ways: its initial bilinear prediction rounds the block height up to the
/// optical-flow unit size (at most four extra rows, since both are multiples of
/// four), and each unit's refined vector moves by at most `MV_DELTA_LIMIT` —
/// one luma sample — in `splot-recon`'s `optflow.rs`.
/// [`COMPOUND_REFINE_REACH`] covers both, scaled into reference rows because a
/// § 7.13.3.18 scaled reference walks `step_y` per predicted row.
///
/// § 7.13.5 TIP callers predict a whole unit batch through one rect whose
/// motion vectors vary per unit, so this bound does not cover them; a row
/// holding a TIP block waits for its references to settle instead, which
/// switches the admission check off.
pub(crate) fn compound_last_row(start_y: i32, step_y: i32, height: usize, last_y: i32) -> i32 {
    let reach = ((i64::from(COMPOUND_REFINE_REACH) * i64::from(step_y)) >> SCALE_SUBPEL_BITS) + 1;
    (i64::from(subpel_last_row(start_y, step_y, height, last_y)).saturating_add(reach))
        .clamp(0, i64::from(last_y).max(0)) as i32
}

/// The last reference row a § 7.13.3.19 block-warp or § 7.13.3.20 extended-warp
/// plane prediction reads, over every section and unit the block splits into.
///
/// Both kernels project a sampling point through the § 7.13.3.23 model and read
/// a fixed window around the projected row: the block warp reads
/// `y4Int - 7 ..= y4Int + 7` (`splot-recon` `warp_prediction.rs`
/// `build_intermediate`), and the extended warp reads `iy4 - 4 ..= iy4 + 4`
/// (`ext_warp_predict_unit`), both clamped into the caller's reference
/// rectangle. Every sampling point either kernel projects — a section centre at
/// `blockX + 4` or a unit centre at `blockX + 4 j4 + 2` — lies inside the
/// block's plane rectangle, and the model is affine, so projecting the four
/// corners of that rectangle and taking the largest row dominates all of them.
/// [`WARP_ROW_REACH`] then covers the wider of the two read windows.
///
/// `warp_params[1]`, `[4]` and `[5]` are the vertical row of the model, applied
/// to luma-resolution source coordinates; the projection is shifted back into
/// plane rows exactly as both kernels shift it.
///
/// A scaled reference is out of scope: § 7.13.3.20 then resamples through the
/// § 7.13.3.18 scaling instead, and the caller bounds it another way.
pub(crate) fn warp_last_row(
    warp_params: [i32; 6],
    (plane_x, plane_y, block_w, block_h): (usize, usize, usize, usize),
    sub_x: u32,
    sub_y: u32,
    last_y: i32,
) -> i32 {
    let projected = |x: usize, y: usize| {
        let source_x = (x as i64) << sub_x;
        let source_y = (y as i64) << sub_y;
        let destination = i64::from(warp_params[4]) * source_x
            + i64::from(warp_params[5]) * source_y
            + i64::from(warp_params[1]);
        (destination >> sub_y) >> WARPEDMODEL_PREC_BITS
    };
    let bottom = plane_y.saturating_add(block_h);
    let right = plane_x.saturating_add(block_w);
    projected(plane_x, plane_y)
        .max(projected(right, plane_y))
        .max(projected(plane_x, bottom))
        .max(projected(right, bottom))
        .saturating_add(WARP_ROW_REACH)
        .clamp(0, i64::from(last_y).max(0)) as i32
}

/// The last reference row one warped plane prediction reads, or [`ALL_ROWS`]
/// when § 7.13.3.20 resamples a scaled reference through the § 7.13.3.18
/// scaling, which [`warp_last_row`]'s corner projection does not bound.
pub(crate) fn warp_plane_last_row(
    warp_params: [i32; 6],
    plane_rect: (usize, usize, usize, usize),
    sub_x: u32,
    sub_y: u32,
    scaling: PlaneScaling,
) -> i32 {
    if scaling.is_scaled() {
        return ALL_ROWS;
    }
    warp_last_row(warp_params, plane_rect, sub_x, sub_y, scaling.last_y)
}

/// A borrow of one reference slot's samples, held for one block's reads.
///
/// A still-filtering frame's published prefix is only readable while its shared
/// borrow lives, so the holder is kept on the reading thread's stack for the
/// block and never across a wait. A reader must also hold at most one borrow per
/// pending frame at a time: the filter phase takes the same lock exclusively
/// once per stripe, so borrowing one frame twice while that writer is queued
/// would deadlock.
pub(crate) enum HeldFrameSamples<'a, T: ReconSample> {
    /// The slot has settled; every row is final.
    Settled(&'a DecodedFrame<T>),
    /// The slot is still filtering; only its published prefix is final.
    Filtering(PublishedFrame<'a, T>),
}

impl<T: ReconSample> HeldFrameSamples<'_, T> {
    /// Resolves the held borrow into the samples one block reads.
    ///
    /// # Errors
    ///
    /// Returns an internal diagnostic when a still-filtering frame's workspace
    /// was taken by its freeze, which the live borrow already rules out.
    pub(crate) fn samples(&self) -> Result<ReferenceSamples<'_, T>> {
        Ok(match self {
            Self::Settled(frame) => ReferenceSamples::settled(frame),
            Self::Filtering(published) => ReferenceSamples::filtering(
                published.workspace()?,
                published.luma_rows(),
                published.chroma_rows(),
            ),
        })
    }
}

fn missing_plane(offset: ByteOffset) -> crate::error::DecodeError {
    unsupported_at(
        "inter_reference_missing_plane",
        offset,
        "minimal inter motion compensation requires the reference frame to carry every plane",
        SPEC_MC,
    )
}

fn plane_geometry(offset: ByteOffset) -> crate::error::DecodeError {
    unsupported_at(
        "inter_reference_plane_geometry",
        offset,
        "minimal inter motion compensation requires a reference plane whose storage covers its visible rectangle",
        SPEC_MC,
    )
}

fn unpublished_rows(offset: ByteOffset) -> crate::error::DecodeError {
    unsupported_at(
        "inter_reference_rows_unpublished",
        offset,
        "inter motion compensation requires every reference row it reads to have been published by the reference frame's filter phase",
        SPEC_MC,
    )
}

#[cfg(test)]
static FORCE_BANDED_READS: core::sync::atomic::AtomicBool =
    core::sync::atomic::AtomicBool::new(false);

/// Reads settled frames through the banded path, with the whole frame published.
///
/// The forced band changes no sample: it drives the same storage, geometry, and
/// admission check a pipelined read takes, with a watermark that admits every
/// row. Concurrent decodes therefore stay byte-identical while it is set.
#[cfg(test)]
pub(crate) fn set_forced_banded_reads(enabled: bool) {
    FORCE_BANDED_READS.store(enabled, core::sync::atomic::Ordering::Relaxed);
}

#[cfg(test)]
fn forced_band<T: ReconSample>(frame: &DecodedFrame<T>) -> Option<PublishedRows> {
    if !FORCE_BANDED_READS.load(core::sync::atomic::Ordering::Relaxed) {
        return None;
    }
    let luma = frame.y().visible_size().height();
    Some(PublishedRows {
        luma,
        chroma: frame
            .plane(PlaneId::U)
            .map_or(luma, |plane| plane.visible_rect().height()),
    })
}

#[cfg(not(test))]
const fn forced_band<T: ReconSample>(_frame: &DecodedFrame<T>) -> Option<PublishedRows> {
    None
}

#[cfg(test)]
#[path = "reference_tests.rs"]
mod tests;
