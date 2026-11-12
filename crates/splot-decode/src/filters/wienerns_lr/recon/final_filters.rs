// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Final loop-filter application for the reconstruction sink.

use super::{MI_SIZE, StripeChain, WienerNsLrReconSink};
use crate::Result;
use crate::bitstream::tile_payload::{
    LrUnitRestorationType, WienerNsLrSourceBlock, WienerNsLrUnitFilter,
};
use crate::filters::cdef::{CdefFrame, CdefSkipGrid};
use crate::filters::source::{FramePlane, StripeCopyError, StripeInitialization, StripePlane};
use crate::support::reusable_scratch::with_reusable_scratch;
use splot_core::headers::frame::{FrameHeaderCore, FrameRestorationType, LrPlaneParams};
use splot_recon::{
    LoopRestorationPlaneBounds, LoopRestorationSource, LoopRestorationSourceBounds,
    PC_WIENER_CLASSIFY_READ_RADIUS, PC_WIENER_FILTER_TAP_RADIUS, PC_WIENER_FULL_CLASSES,
    PcWienerClassifyPaddedSource, PcWienerClassifyParams, PcWienerClassifyScratch, PcWienerFilter,
    PcWienerPaddedSource, PlaneId, PlaneRect, ReconError, ReconSample, Result as ReconResult,
    WIENER_NS_CHROMA_COEFFS, WIENER_NS_CHROMA_TAP_RADIUS, WIENER_NS_LUMA_COEFFS,
    WIENER_NS_LUMA_TAP_RADIUS, WienerNsChromaFilter, WienerNsChromaPaddedSource,
    WienerNsChromaScratch, WienerNsLumaFilter, WienerNsLumaPaddedSource, WienerNsLumaScratch,
    pc_wiener_classify_grid_padded_classes_into, pc_wiener_filter_block_padded,
    pc_wiener_filter_block_padded_u8_into, pc_wiener_filter_set_index, pc_wiener_subclass_table,
    wiener_ns_filter_chroma_block_padded_into, wiener_ns_filter_luma_block_padded_cells_into,
    wiener_ns_filter_luma_block_padded_cells_u8_into, wiener_ns_filter_luma_block_padded_into,
    wiener_ns_filter_luma_block_padded_u8_into,
};

thread_local! {
    static PC_WIENER_CLASSIFY_SCRATCH: std::cell::RefCell<PcWienerClassifyScratch> =
        std::cell::RefCell::new(PcWienerClassifyScratch::default());
    static WIENER_NS_LUMA_SCRATCH: std::cell::Cell<Option<Box<dyn std::any::Any>>> =
        const { std::cell::Cell::new(None) };
    static WIENER_NS_CHROMA_SCRATCH: std::cell::RefCell<WienerNsChromaScratch> =
        std::cell::RefCell::new(WienerNsChromaScratch::default());
    static LR_SOURCE_SCRATCH: std::cell::Cell<Option<Box<LrSourceScratch>>> =
        const { std::cell::Cell::new(None) };
}

const MAX_RETAINED_WIENER_NS_LUMA_SAMPLES: usize = 512 * 64;

fn with_wiener_ns_luma_scratch<T: ReconSample, R>(
    sample_count: usize,
    f: impl FnOnce(&mut WienerNsLumaScratch<T>) -> R,
) -> R {
    WIENER_NS_LUMA_SCRATCH.with(|slot| {
        let mut scratch = slot
            .take()
            .and_then(|scratch| scratch.downcast::<WienerNsLumaScratch<T>>().ok())
            .unwrap_or_default();
        let result = f(&mut scratch);
        if sample_count <= MAX_RETAINED_WIENER_NS_LUMA_SAMPLES {
            slot.set(Some(scratch));
        }
        result
    })
}

const MAX_RETAINED_LR_SCRATCH_ELEMENTS: usize = 64 * 1024;

#[derive(Default)]
struct LrSourceScratch {
    primary: Vec<u16>,
    secondary: Vec<u16>,
    cell_subclasses: Vec<u8>,
}

pub(crate) struct LrFrame<'a, T: ReconSample> {
    pub(crate) deblocked_y: FramePlane<'a, T>,
    pub(crate) deblocked_u: Option<FramePlane<'a, T>>,
    pub(crate) deblocked_v: Option<FramePlane<'a, T>>,
    pub(crate) cdef_y: StripePlane<T>,
    pub(crate) cdef_u: Option<StripePlane<T>>,
    pub(crate) cdef_v: Option<StripePlane<T>>,
    pub(crate) post_lr_y: Option<StripePlane<T>>,
    pub(crate) post_lr_u: Option<StripePlane<T>>,
    pub(crate) post_lr_v: Option<StripePlane<T>>,
    /// The luma post-LR plane already holds the CDEF output, so a flat LR
    /// strip needs no write.
    post_lr_y_holds_cdef: bool,
}

pub(crate) struct FilteredStripe<T: ReconSample> {
    pub(crate) y: StripePlane<T>,
    pub(crate) u: Option<StripePlane<T>>,
    pub(crate) v: Option<StripePlane<T>>,
}

pub(crate) struct LrStripeOutput<T> {
    pub(crate) active_planes: [bool; 3],
    pub(crate) initializations: [StripeInitialization; 3],
    pub(crate) target: crate::pipeline::frame_progress::DirectStripeTarget<T>,
}

impl<'a, T: ReconSample> LrFrame<'a, T> {
    /// An active plane CDEF already filtered in place in its target keeps that
    /// target as the LR output, and LR reads a copy of the CDEF rows instead.
    fn from_cdef(
        mut frame: CdefFrame<'a, T>,
        active_planes: [bool; 3],
        initializations: [StripeInitialization; 3],
        mut target: crate::pipeline::frame_progress::DirectStripeTarget<T>,
    ) -> core::result::Result<Self, StripeCopyError> {
        let planes = [
            Some(&frame.filtered_y),
            frame.filtered_u.as_ref(),
            frame.filtered_v.as_ref(),
        ];
        for plane_id in [PlaneId::Y, PlaneId::U, PlaneId::V] {
            if !active_planes[plane_id.index()] {
                continue;
            }
            let plane = planes[plane_id.index()].ok_or(StripeCopyError::Geometry)?;
            let Some(direct_target) = target.get(plane_id) else {
                if plane.is_direct() {
                    continue;
                }
                return Err(StripeCopyError::Geometry);
            };
            plane.preflight_copy_rows_into(
                plane.origin_y(),
                plane.end_y().ok_or(StripeCopyError::Geometry)?,
                Some(direct_target),
                initializations[plane_id.index()],
            )?;
        }
        let post_lr_y_holds_cdef = target.get(PlaneId::Y).is_none()
            || initializations[PlaneId::Y.index()] == StripeInitialization::CopyAll;
        let mut copy = |plane_id: PlaneId, plane: &mut StripePlane<T>| {
            let end_y = plane.end_y().ok_or(StripeCopyError::Geometry)?;
            let Some(target) = target.take(plane_id) else {
                let cdef = plane.copy_rows_into_mode(
                    plane.origin_y(),
                    end_y,
                    None,
                    StripeInitialization::CopyAll,
                )?;
                return Ok(core::mem::replace(plane, cdef));
            };
            plane.copy_rows_into_mode(
                plane.origin_y(),
                end_y,
                Some(target),
                initializations[plane_id.index()],
            )
        };
        let post_lr_y = if active_planes[PlaneId::Y.index()] {
            Some(
                copy(PlaneId::Y, &mut frame.filtered_y)
                    .map_err(|error| error.for_plane(PlaneId::Y))?,
            )
        } else {
            None
        };
        let post_lr_u = if active_planes[PlaneId::U.index()] {
            match frame.filtered_u.as_mut() {
                Some(plane) => {
                    Some(copy(PlaneId::U, plane).map_err(|error| error.for_plane(PlaneId::U))?)
                }
                None => None,
            }
        } else {
            None
        };
        let post_lr_v = if active_planes[PlaneId::V.index()] {
            match frame.filtered_v.as_mut() {
                Some(plane) => {
                    Some(copy(PlaneId::V, plane).map_err(|error| error.for_plane(PlaneId::V))?)
                }
                None => None,
            }
        } else {
            None
        };
        Ok(Self {
            deblocked_y: frame.deblocked_y,
            deblocked_u: frame.deblocked_u,
            deblocked_v: frame.deblocked_v,
            cdef_y: frame.filtered_y,
            cdef_u: frame.filtered_u,
            cdef_v: frame.filtered_v,
            post_lr_y,
            post_lr_u,
            post_lr_v,
            post_lr_y_holds_cdef,
        })
    }

    pub(crate) fn into_filtered(self) -> FilteredStripe<T> {
        FilteredStripe {
            y: self.post_lr_y.unwrap_or(self.cdef_y),
            u: self.post_lr_u.or(self.cdef_u),
            v: self.post_lr_v.or(self.cdef_v),
        }
    }
}

impl LrSourceScratch {
    fn is_bounded(&self) -> bool {
        [
            self.primary.capacity(),
            self.secondary.capacity(),
            self.cell_subclasses.capacity(),
        ]
        .into_iter()
        .all(|capacity| capacity <= MAX_RETAINED_LR_SCRATCH_ELEMENTS)
    }
}

fn lr_window_error(error: ReconError) -> crate::error::DecodeError {
    match error {
        ReconError::WorkspaceAllocationFailed { .. } => error.into(),
        _ => super::lr_pipeline_state_error(),
    }
}

fn lr_plane_window_error(error: &ReconError, plane: PlaneId) -> crate::error::DecodeError {
    match error {
        ReconError::WorkspaceAllocationFailed { context, .. } => {
            ReconError::WorkspaceAllocationFailed { plane, context }.into()
        }
        _ => super::lr_pipeline_state_error(),
    }
}

fn with_lr_source_scratch<R>(f: impl FnOnce(&mut LrSourceScratch) -> R) -> R {
    LR_SOURCE_SCRATCH.with(|slot| {
        let mut scratch = slot.take().unwrap_or_default();
        let result = f(&mut scratch);
        if scratch.is_bounded() {
            slot.set(Some(scratch));
        }
        result
    })
}

/// The frame filter bank's coefficients, one entry per class.
///
/// The bank holds at most sixteen classes, so the whole set
/// travels inline rather than in a list the caller allocates per stripe.
type LrFrameCoeffs = splot_core::tile::InlineVec<
    [i16; WIENER_NS_LUMA_COEFFS],
    { splot_core::headers::frame::MAX_WIENER_NS_CLASSES },
>;

fn luma_lr_frame_coeffs(plane: &LrPlaneParams, num_classes: usize) -> Result<LrFrameCoeffs> {
    if num_classes == 0 {
        return Err(super::lr_pipeline_state_error());
    }
    let Some(bank) = plane.frame_filter_bank.as_ref() else {
        return Err(super::lr_pipeline_state_error());
    };
    if bank.classes.len() != num_classes {
        return Err(super::lr_pipeline_state_error());
    }
    let mut coeffs = LrFrameCoeffs::default();
    for class in &bank.classes {
        let coeff: [i16; WIENER_NS_LUMA_COEFFS] = class
            .coeffs
            .as_ref()
            .try_into()
            .map_err(|_| super::lr_pipeline_state_error())?;
        coeffs
            .push(coeff)
            .ok_or_else(super::lr_pipeline_state_error)?;
    }
    Ok(coeffs)
}

fn luma_lr_unit_coeffs(
    filters: &[WienerNsLrUnitFilter],
    block: &WienerNsLrSourceBlock,
) -> Result<[i16; WIENER_NS_LUMA_COEFFS]> {
    let filter = lr_unit_filter_for_block(filters, block)?;
    if filter.coeff_count != WIENER_NS_LUMA_COEFFS {
        return Err(super::lr_pipeline_state_error());
    }
    let mut coeffs = [0i16; WIENER_NS_LUMA_COEFFS];
    coeffs.copy_from_slice(&filter.coeffs[..WIENER_NS_LUMA_COEFFS]);
    Ok(coeffs)
}

fn chroma_lr_frame_coeffs(plane: &LrPlaneParams) -> Result<[i16; WIENER_NS_CHROMA_COEFFS]> {
    let Some(bank) = plane.frame_filter_bank.as_ref() else {
        return Err(super::lr_pipeline_state_error());
    };
    let [class] = bank.classes.as_ref() else {
        return Err(super::lr_pipeline_state_error());
    };
    class
        .coeffs
        .as_ref()
        .try_into()
        .map_err(|_| super::lr_pipeline_state_error())
}

fn chroma_lr_unit_coeffs(
    filters: &[WienerNsLrUnitFilter],
    block: &WienerNsLrSourceBlock,
) -> Result<[i16; WIENER_NS_CHROMA_COEFFS]> {
    let filter = lr_unit_filter_for_block(filters, block)?;
    if filter.coeff_count != WIENER_NS_CHROMA_COEFFS {
        return Err(super::lr_pipeline_state_error());
    }
    Ok(filter.coeffs)
}

fn lr_unit_filter_for_block<'a>(
    filters: &'a [WienerNsLrUnitFilter],
    block: &WienerNsLrSourceBlock,
) -> Result<&'a WienerNsLrUnitFilter> {
    let filter = block
        .unit_filter_index
        .and_then(|index| filters.get(index))
        .ok_or_else(super::lr_pipeline_state_error)?;
    if filter.plane != block.plane
        || filter.unit_row != block.unit_row
        || filter.unit_col != block.unit_col
    {
        return Err(super::lr_pipeline_state_error());
    }
    Ok(filter)
}

/// Sorts LR source blocks by `(plane, y, x)` and returns the Y and U
/// partition ends. The parse already merges each unit's rows per stripe,
/// and blocks of other units, tiles or stripes never merge.
pub(crate) fn lr_source_blocks_by_plane(
    mut blocks: Vec<WienerNsLrSourceBlock>,
) -> (Vec<WienerNsLrSourceBlock>, [usize; 2]) {
    blocks.retain(|block| block.plane < 3);
    blocks.sort_unstable_by_key(|block| (block.plane, block.y, block.x));
    debug_assert!(blocks.windows(2).all(|pair| {
        (pair[0].plane, pair[0].y, pair[0].x) != (pair[1].plane, pair[1].y, pair[1].x)
    }));
    let y_end = blocks.partition_point(|block| block.plane < 1);
    let u_end = blocks.partition_point(|block| block.plane < 2);
    (blocks, [y_end, u_end])
}

fn clipped_lr_source_block(
    block: &WienerNsLrSourceBlock,
    plane_width: usize,
    plane_height: usize,
    luma_width: usize,
    luma_height: usize,
) -> Result<WienerNsLrSourceBlock> {
    let mut clipped = *block;
    let remaining_width = plane_width
        .checked_sub(block.x)
        .ok_or_else(super::lr_pipeline_state_error)?;
    let remaining_height = plane_height
        .checked_sub(block.y)
        .ok_or_else(super::lr_pipeline_state_error)?;
    clipped.width = block.width.min(remaining_width);
    clipped.height = block.height.min(remaining_height);
    if clipped.width == 0 || clipped.height == 0 || luma_width == 0 || luma_height == 0 {
        return Err(super::lr_pipeline_state_error());
    }

    let luma_end_x = luma_width - 1;
    let luma_end_y = luma_height - 1;
    clipped.luma_end_x = clipped.luma_end_x.min(luma_end_x);
    clipped.luma_end_y = clipped.luma_end_y.min(luma_end_y);
    clipped.luma_stripe_end_y = clipped.luma_stripe_end_y.min(luma_end_y);
    if clipped.luma_start_x > clipped.luma_end_x
        || clipped.luma_start_y > clipped.luma_end_y
        || clipped.luma_stripe_start_y > clipped.luma_stripe_end_y
    {
        return Err(super::lr_pipeline_state_error());
    }
    Ok(clipped)
}

fn lr_block_in_rows(
    block: &WienerNsLrSourceBlock,
    start_y: usize,
    end_y: usize,
) -> Option<WienerNsLrSourceBlock> {
    let block_end = block.y.checked_add(block.height)?;
    let y = block.y.max(start_y);
    let end = block_end.min(end_y);
    if y >= end {
        return None;
    }
    let mut clipped = *block;
    clipped.y = y;
    clipped.height = end - y;
    Some(clipped)
}

fn lr_restoration_writes_complete_rectangle(
    plane: PlaneId,
    frame_type: FrameRestorationType,
    block_type: LrUnitRestorationType,
) -> bool {
    match plane {
        PlaneId::Y => match frame_type {
            FrameRestorationType::WienerNonsep => block_type == LrUnitRestorationType::WienerNonsep,
            FrameRestorationType::PcWiener => block_type == LrUnitRestorationType::PcWiener,
            FrameRestorationType::Switchable => matches!(
                block_type,
                LrUnitRestorationType::PcWiener | LrUnitRestorationType::WienerNonsep
            ),
            FrameRestorationType::None => false,
        },
        PlaneId::U | PlaneId::V => {
            frame_type == FrameRestorationType::WienerNonsep
                && block_type == LrUnitRestorationType::WienerNonsep
        }
    }
}

fn lr_blocks_near_rows(
    blocks: &[WienerNsLrSourceBlock],
    start_y: usize,
    end_y: usize,
) -> &[WienerNsLrSourceBlock] {
    let relevant_end = blocks.partition_point(|block| block.y < end_y);
    let relevant_start = blocks[..relevant_end]
        .iter()
        .position(|block| {
            block
                .y
                .checked_add(block.height)
                .is_some_and(|block_end| block_end > start_y)
        })
        .unwrap_or(relevant_end);
    &blocks[relevant_start..relevant_end]
}

pub(crate) fn lr_plane_fully_overwritten(
    blocks: &[WienerNsLrSourceBlock],
    plane: PlaneId,
    frame_type: FrameRestorationType,
    width: usize,
    frame_height: usize,
    start_y: usize,
    end_y: usize,
) -> bool {
    if width == 0 || start_y >= end_y || end_y > frame_height {
        return false;
    }
    let blocks = lr_blocks_near_rows(blocks, start_y, end_y);
    let mut y = start_y;
    while y < end_y {
        let mut x = 0usize;
        let mut next_y = end_y;
        for block in blocks {
            let Some(block_end_y) = block.y.checked_add(block.height) else {
                return false;
            };
            let block_end_y = block_end_y.min(frame_height);
            if block.y > y {
                next_y = next_y.min(block.y);
                continue;
            }
            if block_end_y <= y {
                continue;
            }
            next_y = next_y.min(block_end_y);
            let Some(block_end_x) = block.x.checked_add(block.width) else {
                return false;
            };
            let block_end_x = block_end_x.min(width);
            if !lr_restoration_writes_complete_rectangle(plane, frame_type, block.restoration_type)
                || block.x >= block_end_x
                || block.x != x
            {
                return false;
            }
            x = block_end_x;
        }
        if x != width {
            return false;
        }
        y = next_y;
    }
    true
}

pub(crate) fn lr_initializations<T>(
    core: &FrameHeaderCore,
    active_planes: [bool; 3],
    plane_blocks: [&[WienerNsLrSourceBlock]; 3],
    target: &crate::pipeline::frame_progress::DirectStripeTarget<T>,
) -> [StripeInitialization; 3] {
    core::array::from_fn(|index| {
        let plane = [PlaneId::Y, PlaneId::U, PlaneId::V][index];
        let Some(target) = target.get(plane) else {
            return StripeInitialization::CopyAll;
        };
        if !active_planes[index] {
            return StripeInitialization::CopyAll;
        }
        let covered = target.end_y().is_some_and(|end_y| {
            core.lr_params
                .as_ref()
                .and_then(|params| params.planes.get(index))
                .is_some_and(|params| {
                    lr_plane_fully_overwritten(
                        plane_blocks[index],
                        plane,
                        params.restoration_type,
                        target.width(),
                        target.frame_height(),
                        target.origin_y(),
                        end_y,
                    )
                })
        });
        if covered {
            StripeInitialization::FullyOverwritten
        } else {
            StripeInitialization::CopyAll
        }
    })
}

fn lr_block_destination<'a, T: ReconSample>(
    plane: &'a mut StripePlane<T>,
    block: &WienerNsLrSourceBlock,
) -> Result<(&'a mut [T], usize)> {
    if block
        .x
        .checked_add(block.width)
        .is_none_or(|end_x| end_x > plane.width())
    {
        return Err(super::lr_pipeline_state_error());
    }
    let rect = PlaneRect::new(block.x, block.y, block.width, block.height)
        .map_err(|_| super::lr_pipeline_state_error())?;
    plane
        .rect_mut(rect)
        .ok_or_else(super::lr_pipeline_state_error)
}

/// Runs `ten` on `u16` output or `eight` on `u8` output.
fn by_depth<T: ReconSample, C>(
    output: &mut [T],
    context: C,
    ten: impl FnOnce(&mut [u16], C) -> ReconResult<()>,
    eight: impl FnOnce(&mut [u8], C) -> ReconResult<()>,
) -> ReconResult<()> {
    if let Some(output) = T::u8_slice_mut(output) {
        return eight(output, context);
    }
    let output = T::u16_slice_mut(output).ok_or(ReconError::ArithmeticOverflow {
        context: "LR output sample type",
    })?;
    ten(output, context)
}

/// Post-CCSO rows adjacent to a stripe band, split per plane.
///
/// § 7.20.2 sources every in-stripe sample from `CdefFrame`, and § 7.20.1
/// stripes restart per tile row, so a band abutting a tile-row boundary needs
/// deringed rows the band itself never covers.
#[derive(Default)]
pub(crate) struct CdefOverlap<T: ReconSample> {
    pub(crate) y: Vec<StripePlane<T>>,
    pub(crate) u: Vec<StripePlane<T>>,
    pub(crate) v: Vec<StripePlane<T>>,
}

impl<T: ReconSample> CdefOverlap<T> {
    fn plane(&self, plane_id: PlaneId) -> &[StripePlane<T>] {
        match plane_id {
            PlaneId::Y => &self.y,
            PlaneId::U => &self.u,
            PlaneId::V => &self.v,
        }
    }
}

/// A padded § 7.20.2 source window held as `u16` for every sample storage.
struct LrSourceWindow<'a> {
    samples: &'a [u16],
    stride: usize,
    origin_x: isize,
    origin_y: isize,
}

/// The geometry every row of one padded § 7.20.2 source window shares.
///
/// The § 7.20.2 column clip does not depend on the row, so every row
/// replicates its left edge sample over the first `pre` columns, copies
/// `mid` samples from `mid_start`, and replicates its right edge sample over
/// the rest.
struct LrWindowGeometry {
    resolved: LoopRestorationPlaneBounds,
    plane_height: usize,
    y0: isize,
    x0: isize,
    stride: usize,
    rows: usize,
    left: usize,
    right: usize,
    pre: usize,
    mid: usize,
    mid_start: usize,
}

impl LrWindowGeometry {
    #[allow(clippy::too_many_arguments)]
    fn new<T: ReconSample>(
        plane: PlaneId,
        curr_plane: FramePlane<'_, T>,
        cdef_plane: &StripePlane<T>,
        bounds: &LoopRestorationSourceBounds,
        block_x: isize,
        block_y: isize,
        width: usize,
        height: usize,
        (radius_x, radius_y): (usize, usize),
    ) -> ReconResult<Self> {
        let plane_width = curr_plane.width();
        let plane_height = curr_plane.frame_height();
        if cdef_plane.width() != plane_width || cdef_plane.frame_height() != plane_height {
            return Err(ReconError::PcWienerInvalidBounds {
                field: "LR source frame bounds",
            });
        }
        let stride = width
            .checked_add(radius_x.checked_mul(2).ok_or(OVERFLOW_WINDOW)?)
            .ok_or(OVERFLOW_WINDOW)?;
        let rows = height
            .checked_add(radius_y.checked_mul(2).ok_or(OVERFLOW_WINDOW)?)
            .ok_or(OVERFLOW_WINDOW)?;
        let offset = |value: isize, radius: usize| value.checked_sub(isize::try_from(radius).ok()?);
        let x0 = offset(block_x, radius_x).ok_or(OVERFLOW_WINDOW)?;
        let resolved = LoopRestorationPlaneBounds::new(plane, bounds)?;
        let left = resolved.sample(isize::MIN, 0).x;
        let right = resolved.sample(isize::MAX, 0).x;
        if right >= plane_width {
            return Err(ReconError::PcWienerInvalidBounds {
                field: "LR source frame bounds",
            });
        }
        let min_x = isize::try_from(left).map_err(|_| OVERFLOW_WINDOW)?;
        let max_x = isize::try_from(right).map_err(|_| OVERFLOW_WINDOW)?;
        let stride_i = isize::try_from(stride).map_err(|_| OVERFLOW_WINDOW)?;
        let pre = min_x
            .checked_sub(x0)
            .ok_or(OVERFLOW_WINDOW)?
            .clamp(0, stride_i) as usize;
        let post = x0
            .checked_add(stride_i)
            .and_then(|end| end.checked_sub(1))
            .and_then(|last| last.checked_sub(max_x))
            .ok_or(OVERFLOW_WINDOW)?
            .clamp(
                0,
                stride_i.checked_sub(pre as isize).ok_or(OVERFLOW_WINDOW)?,
            ) as usize;
        Ok(Self {
            resolved,
            plane_height,
            y0: offset(block_y, radius_y).ok_or(OVERFLOW_WINDOW)?,
            x0,
            stride,
            rows,
            left,
            right,
            pre,
            mid: stride - pre - post,
            mid_start: (x0 + pre as isize) as usize,
        })
    }

    /// Resolves window row `row_index` to its § 7.20.2 source row.
    fn row<'a, T: ReconSample>(
        &self,
        row_index: usize,
        curr_plane: FramePlane<'a, T>,
        cdef_plane: &'a StripePlane<T>,
        cdef_overlap: &'a [StripePlane<T>],
    ) -> ReconResult<&'a [T]> {
        let y = isize::try_from(row_index)
            .ok()
            .and_then(|row| self.y0.checked_add(row))
            .ok_or(OVERFLOW_WINDOW)?;
        let sample = self.resolved.sample(0, y);
        if sample.y >= self.plane_height {
            return Err(ReconError::PcWienerInvalidBounds {
                field: "LR source frame bounds",
            });
        }
        match sample.source {
            LoopRestorationSource::CurrFrame => {
                curr_plane
                    .row(sample.y)
                    .ok_or_else(|| ReconError::BufferLengthMismatch {
                        expected: sample.y.saturating_add(1),
                        actual: curr_plane.frame_height(),
                    })
            }
            LoopRestorationSource::CdefFrame => cdef_plane
                .row(sample.y)
                .or_else(|| cdef_overlap.iter().find_map(|plane| plane.row(sample.y)))
                .ok_or_else(|| ReconError::BufferLengthMismatch {
                    expected: sample.y.saturating_add(1),
                    actual: cdef_plane.end_y().unwrap_or(cdef_plane.origin_y()),
                }),
        }
    }

    /// The source samples of `row` themselves when no window column is
    /// clipped.
    fn borrowed<'a, T>(&self, row: &'a [T]) -> Option<&'a [T]> {
        if self.mid != self.stride {
            return None;
        }
        row.get(self.mid_start..self.mid_start.checked_add(self.stride)?)
    }

    fn write<T: Copy, O: Copy>(
        &self,
        source_row: &[T],
        output: &mut [O],
        convert: impl Fn(T) -> O,
    ) -> ReconResult<()> {
        let edge = |x: usize| {
            source_row.get(x).map(|&sample| convert(sample)).ok_or(
                ReconError::BufferLengthMismatch {
                    expected: x.saturating_add(1),
                    actual: source_row.len(),
                },
            )
        };
        let (pre, mid) = (self.pre, self.mid);
        output[..pre].fill(edge(self.left)?);
        if mid > 0 {
            let mid_end = self.mid_start.saturating_add(mid);
            let source = source_row.get(self.mid_start..mid_end).ok_or(
                ReconError::BufferLengthMismatch {
                    expected: mid_end,
                    actual: source_row.len(),
                },
            )?;
            for (output, &value) in output[pre..pre + mid].iter_mut().zip(source) {
                *output = convert(value);
            }
        }
        output[pre + mid..].fill(edge(self.right)?);
        Ok(())
    }
}

/// A § 7.20.1 stripe is 64 luma rows and no LR block spans two, so a luma
/// source window never holds more rows than this, and a `u128` row mask
/// covers it.
const MAX_LR_WINDOW_ROWS: usize = 64 + 2 * PC_WIENER_CLASSIFY_READ_RADIUS;
const _: () = assert!(MAX_LR_WINDOW_ROWS <= 128);

/// A padded § 7.20.2 luma source window as rows: an unclipped row borrows its
/// source plane row, and only a clipped or `u8` row is resolved into the
/// scratch.
struct LrSourceRows<'a> {
    rows: [&'a [u16]; MAX_LR_WINDOW_ROWS],
    len: usize,
    radius: usize,
    origin_x: isize,
    origin_y: isize,
}

impl<'a> LrSourceRows<'a> {
    #[allow(clippy::too_many_arguments)]
    fn resolve<T: ReconSample>(
        scratch: &'a mut Vec<u16>,
        curr_plane: FramePlane<'a, T>,
        cdef_plane: &'a StripePlane<T>,
        cdef_overlap: &'a [StripePlane<T>],
        bounds: &LoopRestorationSourceBounds,
        block_x: isize,
        block_y: isize,
        width: usize,
        height: usize,
        radius: usize,
    ) -> ReconResult<Self> {
        let geometry = LrWindowGeometry::new(
            PlaneId::Y,
            curr_plane,
            cdef_plane,
            bounds,
            block_x,
            block_y,
            width,
            height,
            (radius, radius),
        )?;
        let (stride, len) = (geometry.stride, geometry.rows);
        if len > MAX_LR_WINDOW_ROWS {
            return Err(OVERFLOW_WINDOW);
        }
        reserve_window(
            scratch,
            PlaneId::Y,
            stride.checked_mul(len).ok_or(OVERFLOW_WINDOW)?,
        )?;
        let cdef_origin = usize_to_isize_recon(cdef_plane.origin_y(), "LR source window rows")?;
        let (cdef_first, cdef_last) = geometry.resolved.cdef_rows();
        let cdef_first = cdef_first.max(cdef_origin);
        let cdef_start = |y: isize| {
            let row = usize::try_from(y.checked_sub(cdef_origin)?).ok()?;
            row.checked_mul(cdef_plane.stride())?
                .checked_add(geometry.mid_start)
        };
        let unclipped = geometry.mid == stride;
        let mut rows: [&'a [u16]; MAX_LR_WINDOW_ROWS] = [&[]; MAX_LR_WINDOW_ROWS];
        let mut in_scratch = 0u128;
        for (row_index, row) in rows.iter_mut().enumerate().take(len) {
            let y = geometry.y0 + row_index as isize;
            if unclipped
                && (cdef_first..=cdef_last).contains(&y)
                && let Some(samples) = cdef_start(y)
                    .and_then(|start| cdef_plane.samples().get(start..start.checked_add(stride)?))
            {
                if let Some(samples) = T::u16_slice(samples) {
                    *row = samples;
                } else {
                    let output = &mut scratch[row_index * stride..(row_index + 1) * stride];
                    for (output, sample) in output.iter_mut().zip(samples) {
                        *output = sample.to_u16();
                    }
                    in_scratch |= 1 << row_index;
                }
                continue;
            }
            let source = geometry.row(row_index, curr_plane, cdef_plane, cdef_overlap)?;
            if let Some(samples) = geometry.borrowed(source).and_then(T::u16_slice) {
                *row = samples;
                continue;
            }
            geometry.write(
                source,
                &mut scratch[row_index * stride..(row_index + 1) * stride],
                T::to_u16,
            )?;
            in_scratch |= 1 << row_index;
        }
        let scratch: &'a [u16] = scratch;
        for (row_index, row) in rows.iter_mut().enumerate().take(len) {
            if in_scratch & (1 << row_index) != 0 {
                *row = &scratch[row_index * stride..(row_index + 1) * stride];
            }
        }
        Ok(Self {
            rows,
            len,
            radius,
            origin_x: geometry.x0,
            origin_y: geometry.y0,
        })
    }

    /// The rows reaching `tap_radius` around the block, and the column in
    /// each row where they start.
    fn tail(&self, tap_radius: usize) -> Option<(&[&'a [u16]], usize)> {
        let skip = self.radius.checked_sub(tap_radius)?;
        Some((self.rows.get(skip..self.len - skip)?, skip))
    }
}

/// The one value that the `width` columns from `col` of every row hold.
fn flat_lr_window<T: ReconSample>(rows: &[&[T]], col: usize, width: usize) -> Option<u16> {
    let value = rows.first()?.get(col)?.to_u16();
    rows.iter()
        .all(|row| {
            col.checked_add(width)
                .and_then(|end| row.get(col..end))
                .is_some_and(|row| {
                    crate::filters::cdef::holds_value(row, value)
                        .unwrap_or_else(|| row.iter().all(|sample| sample.to_u16() == value))
                })
        })
        .then_some(value)
}

/// Walks the `width` output columns of a luma LR block in 64-column strips
/// and calls `run(x, end, flat)` once for each flat strip, with its value,
/// and once for each run of uneven strips, with `None`. A strip is flat when
/// its whole tap reach, `radius` columns and rows past it in `rows` from
/// column `col`, holds one value `v <= max_sample`: the § 7.20.3 tap
/// differences are then zero and the § 7.20.4 taps sum to zero, so both
/// filters return `v`, and no other process reads the strip's classes.
fn for_each_lr_strip<T: ReconSample>(
    rows: &[&[T]],
    col: usize,
    width: usize,
    radius: usize,
    max_sample: u16,
    mut run: impl FnMut(usize, usize, Option<u16>) -> Result<()>,
) -> Result<()> {
    const STRIP: usize = 64;
    let flat_at = |x: usize| {
        flat_lr_window(rows, col + x, STRIP.min(width - x) + 2 * radius)
            .filter(|&value| value <= max_sample)
    };
    let (mut x, mut next) = (0, flat_at(0));
    while x < width {
        let flat = next.take();
        let mut end = (x + STRIP).min(width);
        while end < width {
            next = flat_at(end);
            if flat.is_some() || next.is_some() {
                break;
            }
            end = (end + STRIP).min(width);
        }
        run(x, end, flat)?;
        x = end;
    }
    Ok(())
}

fn fill_lr_rect<O: Copy>(
    output: &mut [O],
    stride: usize,
    width: usize,
    height: usize,
    value: O,
) -> Result<()> {
    for row in 0..height {
        let start = row
            .checked_mul(stride)
            .ok_or_else(super::lr_pipeline_state_error)?;
        output
            .get_mut(start..start.saturating_add(width))
            .ok_or_else(super::lr_pipeline_state_error)?
            .fill(value);
    }
    Ok(())
}

fn reserve_window(samples: &mut Vec<u16>, plane: PlaneId, sample_count: usize) -> ReconResult<()> {
    if let Some(missing) = sample_count.checked_sub(samples.len()).filter(|n| *n > 0) {
        samples
            .try_reserve_exact(missing)
            .map_err(|_| ReconError::WorkspaceAllocationFailed {
                plane,
                context: "LR source window",
            })?;
        samples.resize(sample_count, 0);
    }
    Ok(())
}

impl<'a> LrSourceWindow<'a> {
    /// Resolves one padded § 7.17 source window into `samples`.
    ///
    /// Every row is written across the whole stride, so a buffer that already
    /// holds enough samples keeps its previous contents instead of being
    /// cleared and refilled.
    #[allow(clippy::too_many_arguments)]
    fn materialize<T: ReconSample>(
        samples: &'a mut Vec<u16>,
        plane: PlaneId,
        curr_plane: FramePlane<'_, T>,
        cdef_plane: &StripePlane<T>,
        cdef_overlap: &[StripePlane<T>],
        bounds: &LoopRestorationSourceBounds,
        block_x: isize,
        block_y: isize,
        width: usize,
        height: usize,
        radius: (usize, usize),
    ) -> ReconResult<Self> {
        let geometry = LrWindowGeometry::new(
            plane, curr_plane, cdef_plane, bounds, block_x, block_y, width, height, radius,
        )?;
        let stride = geometry.stride;
        let sample_count = stride.checked_mul(geometry.rows).ok_or(OVERFLOW_WINDOW)?;
        reserve_window(samples, plane, sample_count)?;
        for (row_index, output) in samples[..sample_count].chunks_exact_mut(stride).enumerate() {
            let row = geometry.row(row_index, curr_plane, cdef_plane, cdef_overlap)?;
            geometry.write(row, output, T::to_u16)?;
        }
        Ok(Self {
            samples: samples.get(..sample_count).ok_or(OVERFLOW_WINDOW)?,
            stride,
            origin_x: geometry.x0,
            origin_y: geometry.y0,
        })
    }

    fn tail_from(&self, x: isize, y: isize) -> Option<(&'a [u16], usize)> {
        let col = usize::try_from(x.checked_sub(self.origin_x)?).ok()?;
        let row = usize::try_from(y.checked_sub(self.origin_y)?).ok()?;
        if col >= self.stride {
            return None;
        }
        let start = row.checked_mul(self.stride)?.checked_add(col)?;
        self.samples.get(start..).map(|tail| (tail, self.stride))
    }

    #[cfg(test)]
    fn get_abs(&self, x: isize, y: isize) -> u16 {
        let col = x.saturating_sub(self.origin_x);
        let row = y.saturating_sub(self.origin_y);
        if col < 0 || row < 0 || col as usize >= self.stride {
            return 0;
        }
        self.samples
            .get(
                (row as usize)
                    .saturating_mul(self.stride)
                    .saturating_add(col as usize),
            )
            .copied()
            .unwrap_or_default()
    }
}

const OVERFLOW_WINDOW: ReconError = ReconError::ArithmeticOverflow {
    context: "LR source window geometry",
};

fn usize_to_isize_recon(value: usize, context: &'static str) -> ReconResult<isize> {
    isize::try_from(value).map_err(|_| ReconError::ArithmeticOverflow { context })
}

fn mi_to_luma_start_recon(mi: usize, context: &'static str) -> ReconResult<usize> {
    mi.checked_mul(MI_SIZE)
        .ok_or(ReconError::ArithmeticOverflow { context })
}

fn mi_to_luma_end_recon(mi_end: usize, context: &'static str) -> ReconResult<usize> {
    mi_to_luma_start_recon(mi_end, context)?
        .checked_sub(1)
        .ok_or(ReconError::ArithmeticOverflow { context })
}

impl<T: ReconSample> WienerNsLrReconSink<T> {
    pub(crate) fn cdef_skip_grid(
        &self,
        core: &FrameHeaderCore,
        mi_rows: usize,
        mi_cols: usize,
    ) -> Result<Option<CdefSkipGrid>> {
        let Some(cdef) = core.cdef_params.as_ref() else {
            return Ok(None);
        };
        if cdef.cdef_on_skip_txfm_frame_enable != Some(false) {
            return Ok(None);
        }
        let skip_grid = crate::filters::wienerns_lr::derive_cdef_skip_grid(
            mi_rows,
            mi_cols,
            &self.filter_records.tx_skip_records,
        )
        .map_err(|_| super::lr_pipeline_state_error())?;
        Ok(Some(skip_grid))
    }
}

impl StripeChain<'_> {
    pub(crate) fn active_lr_planes(
        &self,
        luma_start: usize,
        luma_end: usize,
        plane_blocks: [&[WienerNsLrSourceBlock]; 3],
    ) -> [bool; 3] {
        let sub_y = usize::from(self.pixel_format.subsampling_y());
        core::array::from_fn(|plane| {
            let shift = usize::from(plane != PlaneId::Y.index()) * sub_y;
            let start = luma_start >> shift;
            let end = luma_end.div_ceil(1 << shift);
            plane_blocks[plane]
                .iter()
                .any(|block| lr_block_in_rows(block, start, end).is_some())
        })
    }

    pub(crate) fn apply_lr_stripe<'a, T: ReconSample>(
        &self,
        core: &FrameHeaderCore,
        cdef: CdefFrame<'a, T>,
        cdef_overlap: &CdefOverlap<T>,
        plane_blocks: [&[WienerNsLrSourceBlock]; 3],
        lr_unit_filters: &[WienerNsLrUnitFilter],
        output: LrStripeOutput<T>,
    ) -> Result<LrFrame<'a, T>> {
        let mut frame = LrFrame::from_cdef(
            cdef,
            output.active_planes,
            output.initializations,
            output.target,
        )
        .map_err(|error| match error {
            StripeCopyError::Allocation(plane) => {
                crate::error::DecodeError::from(ReconError::WorkspaceAllocationFailed {
                    plane,
                    context: "post-LR stripe copy",
                })
            }
            StripeCopyError::Geometry => super::lr_pipeline_state_error(),
        })?;
        let Some(lr_params) = core.lr_params.as_ref() else {
            return Ok(frame);
        };

        if let Some(plane) = lr_params.planes.first()
            && matches!(
                plane.restoration_type,
                FrameRestorationType::WienerNonsep
                    | FrameRestorationType::PcWiener
                    | FrameRestorationType::Switchable
            )
            && output.active_planes[PlaneId::Y.index()]
        {
            let qindex = core
                .quantization_params
                .as_ref()
                .ok_or_else(crate::filters::wienerns_lr::selectable_missing_quantization_error)?
                .base_q_idx;
            let filter_set_index = pc_wiener_filter_set_index(qindex);
            let frame_coeffs = if matches!(
                plane.restoration_type,
                FrameRestorationType::WienerNonsep | FrameRestorationType::Switchable
            ) && plane.frame_filters_on
            {
                let num_classes = usize::from(plane.num_filter_classes.unwrap_or(1));
                Some((luma_lr_frame_coeffs(plane, num_classes)?, num_classes))
            } else {
                None
            };
            let start_y = frame.cdef_y.origin_y();
            let end_y = frame
                .cdef_y
                .end_y()
                .ok_or_else(super::lr_pipeline_state_error)?;
            let fill_flat = !frame.post_lr_y_holds_cdef;
            let post_lr_y = frame
                .post_lr_y
                .as_mut()
                .ok_or_else(super::lr_pipeline_state_error)?;
            for block in plane_blocks[PlaneId::Y.index()]
                .iter()
                .filter_map(|block| lr_block_in_rows(block, start_y, end_y))
            {
                match block.restoration_type {
                    LrUnitRestorationType::PcWiener => {
                        self.compute_pc_wiener_block(
                            &block,
                            frame.deblocked_y,
                            &frame.cdef_y,
                            &cdef_overlap.y,
                            qindex,
                            filter_set_index,
                            post_lr_y,
                            fill_flat,
                        )?;
                    }
                    LrUnitRestorationType::WienerNonsep => {
                        if let Some((coeffs, num_classes)) = frame_coeffs.as_ref() {
                            self.compute_luma_lr_block(
                                &block,
                                frame.deblocked_y,
                                &frame.cdef_y,
                                &cdef_overlap.y,
                                qindex,
                                *num_classes,
                                filter_set_index,
                                coeffs,
                                post_lr_y,
                                fill_flat,
                            )?;
                        } else {
                            let coeffs = [luma_lr_unit_coeffs(lr_unit_filters, &block)?];
                            self.compute_luma_lr_block(
                                &block,
                                frame.deblocked_y,
                                &frame.cdef_y,
                                &cdef_overlap.y,
                                qindex,
                                1,
                                0,
                                &coeffs,
                                post_lr_y,
                                fill_flat,
                            )?;
                        }
                    }
                    LrUnitRestorationType::None => return Err(super::lr_pipeline_state_error()),
                }
            }
        }

        for plane_id in [PlaneId::U, PlaneId::V] {
            if !output.active_planes[plane_id.index()] {
                continue;
            }
            let Some(plane) = lr_params.planes.get(plane_id.index()) else {
                continue;
            };
            if plane.restoration_type != FrameRestorationType::WienerNonsep {
                continue;
            }
            let frame_coeffs = if plane.frame_filters_on {
                Some(chroma_lr_frame_coeffs(plane)?)
            } else {
                None
            };
            let (curr, cdef, post_lr) = match plane_id {
                PlaneId::U => (
                    frame.deblocked_u.as_ref(),
                    frame.cdef_u.as_ref(),
                    frame.post_lr_u.as_mut(),
                ),
                PlaneId::V => (
                    frame.deblocked_v.as_ref(),
                    frame.cdef_v.as_ref(),
                    frame.post_lr_v.as_mut(),
                ),
                PlaneId::Y => return Err(super::lr_pipeline_state_error()),
            };
            let curr = curr.ok_or_else(super::lr_pipeline_state_error)?;
            let cdef = cdef.ok_or_else(super::lr_pipeline_state_error)?;
            let post_lr = post_lr.ok_or_else(super::lr_pipeline_state_error)?;
            let start_y = post_lr.origin_y();
            let end_y = post_lr.end_y().ok_or_else(super::lr_pipeline_state_error)?;
            for block in plane_blocks[plane_id.index()]
                .iter()
                .filter_map(|block| lr_block_in_rows(block, start_y, end_y))
            {
                if block.restoration_type != LrUnitRestorationType::WienerNonsep {
                    return Err(super::lr_pipeline_state_error());
                }
                self.compute_chroma_lr_block(
                    plane_id,
                    &block,
                    lr_unit_filters,
                    frame_coeffs.as_ref(),
                    *curr,
                    cdef,
                    cdef_overlap.plane(plane_id),
                    frame.deblocked_y,
                    &frame.cdef_y,
                    &cdef_overlap.y,
                    post_lr,
                )?;
            }
        }
        Ok(frame)
    }

    #[allow(clippy::too_many_arguments)]
    fn compute_pc_wiener_block<T: ReconSample>(
        &self,
        block: &WienerNsLrSourceBlock,
        curr_luma: FramePlane<'_, T>,
        cdef_luma: &StripePlane<T>,
        cdef_luma_overlap: &[StripePlane<T>],
        qindex: u32,
        filter_set_index: usize,
        post_lr: &mut StripePlane<T>,
        fill_flat: bool,
    ) -> Result<()> {
        let block = clipped_lr_source_block(
            block,
            self.luma_width,
            self.luma_height,
            self.luma_width,
            self.luma_height,
        )?;
        let bounds = crate::filters::wienerns_lr::wienerns_lr_source_block_bounds(&block, 0, 0);
        let block_x = usize_to_isize_recon(block.x, "PC-Wiener block x")
            .map_err(|_| super::lr_pipeline_state_error())?;
        let block_y = usize_to_isize_recon(block.y, "PC-Wiener block y")
            .map_err(|_| super::lr_pipeline_state_error())?;
        let (output, output_stride) = lr_block_destination(post_lr, &block)?;
        with_lr_source_scratch(|scratch| {
            let LrSourceScratch {
                primary,
                cell_subclasses,
                ..
            } = scratch;
            let window = LrSourceRows::resolve(
                primary,
                curr_luma,
                cdef_luma,
                cdef_luma_overlap,
                &bounds,
                block_x,
                block_y,
                block.width,
                block.height,
                PC_WIENER_CLASSIFY_READ_RADIUS.max(PC_WIENER_FILTER_TAP_RADIUS),
            )
            .map_err(lr_window_error)?;
            let (rows, col) = window
                .tail(PC_WIENER_FILTER_TAP_RADIUS)
                .ok_or_else(super::lr_pipeline_state_error)?;
            for_each_lr_strip(
                rows,
                col,
                block.width,
                PC_WIENER_FILTER_TAP_RADIUS,
                self.bit_depth.max_sample(),
                |x, end, flat| {
                    if flat.is_some() && !fill_flat {
                        return Ok(());
                    }
                    let output = output
                        .get_mut(x..)
                        .ok_or_else(super::lr_pipeline_state_error)?;
                    if let Some(value) = flat {
                        let value =
                            T::try_from_u16(value).map_err(|_| super::lr_pipeline_state_error())?;
                        return fill_lr_rect(output, output_stride, end - x, block.height, value);
                    }
                    let mut sub = block;
                    sub.x += x;
                    sub.width = end - x;
                    let subclass_map = self.luma_lr_cell_subclasses(
                        &sub,
                        &window,
                        qindex,
                        PC_WIENER_FULL_CLASSES,
                        filter_set_index,
                        cell_subclasses,
                    )?;
                    let params = PcWienerFilter {
                        width: sub.width,
                        height: block.height,
                        output_stride,
                        bit_depth: self.bit_depth,
                        filter_set_index,
                        subclass_block_size: MI_SIZE,
                        subclasses: subclass_map,
                    };
                    let padded_source =
                        PcWienerPaddedSource::from_rows(rows, col + x, sub.width, block.height)
                            .map_err(lr_window_error)?;
                    by_depth(
                        output,
                        (&params, &padded_source),
                        |out, (params, source)| pc_wiener_filter_block_padded(out, params, source),
                        |out, (params, source)| {
                            pc_wiener_filter_block_padded_u8_into(out, params, source)
                        },
                    )
                    .map_err(lr_window_error)
                },
            )?;
            self.preserve_lossless_lr_samples(PlaneId::Y, &block, curr_luma, output, output_stride)
        })
    }

    fn plane_dimensions(&self, plane_id: PlaneId) -> (usize, usize) {
        let (sub_x, sub_y) = self.plane_subsampling(plane_id);
        (
            self.luma_width.div_ceil(1 << sub_x),
            self.luma_height.div_ceil(1 << sub_y),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn compute_chroma_lr_block<T: ReconSample>(
        &self,
        plane_id: PlaneId,
        block: &WienerNsLrSourceBlock,
        lr_unit_filters: &[WienerNsLrUnitFilter],
        frame_coeffs: Option<&[i16; WIENER_NS_CHROMA_COEFFS]>,
        curr_chroma: FramePlane<'_, T>,
        cdef_chroma: &StripePlane<T>,
        cdef_chroma_overlap: &[StripePlane<T>],
        curr_luma: FramePlane<'_, T>,
        cdef_luma: &StripePlane<T>,
        cdef_luma_overlap: &[StripePlane<T>],
        post_lr: &mut StripePlane<T>,
    ) -> Result<()> {
        let (plane_width, plane_height) = self.plane_dimensions(plane_id);
        let (sub_x, sub_y) = self.plane_subsampling(plane_id);
        let block = clipped_lr_source_block(
            block,
            plane_width,
            plane_height,
            self.luma_width,
            self.luma_height,
        )?;
        let coeffs = match frame_coeffs {
            Some(coeffs) => *coeffs,
            None => chroma_lr_unit_coeffs(lr_unit_filters, &block)?,
        };
        let bounds = crate::filters::wienerns_lr::wienerns_lr_source_block_bounds(
            &block,
            sub_x as u8,
            sub_y as u8,
        );
        let block_x = usize_to_isize_recon(block.x, "chroma LR block x")
            .map_err(|_| super::lr_pipeline_state_error())?;
        let block_y = usize_to_isize_recon(block.y, "chroma LR block y")
            .map_err(|_| super::lr_pipeline_state_error())?;
        let (output, output_stride) = lr_block_destination(post_lr, &block)?;
        {
            let params = WienerNsChromaFilter {
                x: block.x,
                y: block.y,
                width: block.width,
                height: block.height,
                output_stride,
                bit_depth: self.bit_depth,
                coeffs: &coeffs,
                subsampling_x: sub_x as u8,
                subsampling_y: sub_y as u8,
                luma_start_x: block.luma_start_x,
                luma_end_x: block.luma_end_x,
                mi_rows: self.luma_height.div_ceil(MI_SIZE),
                cfl_ds_filter_index: self.cfl_ds_filter_index,
            };
            with_lr_source_scratch(|scratch| -> Result<()> {
                let chroma_window = LrSourceWindow::materialize(
                    &mut scratch.primary,
                    plane_id,
                    curr_chroma,
                    cdef_chroma,
                    cdef_chroma_overlap,
                    &bounds,
                    block_x,
                    block_y,
                    block.width,
                    block.height,
                    (WIENER_NS_CHROMA_TAP_RADIUS, WIENER_NS_CHROMA_TAP_RADIUS),
                )
                .map_err(lr_window_error)?;
                let radius = WIENER_NS_CHROMA_TAP_RADIUS as isize;
                let (chroma_padded, chroma_stride) = chroma_window
                    .tail_from(
                        block_x.saturating_sub(radius),
                        block_y.saturating_sub(radius),
                    )
                    .ok_or_else(super::lr_pipeline_state_error)?;
                let padded_source = if params.reads_luma() {
                    let luma_radius_x = WIENER_NS_CHROMA_TAP_RADIUS << sub_x;
                    let luma_radius_y = WIENER_NS_CHROMA_TAP_RADIUS << sub_y;
                    let luma_block_x = block_x
                        .checked_mul(1 << sub_x)
                        .ok_or_else(super::lr_pipeline_state_error)?;
                    let luma_block_y = block_y
                        .checked_mul(1 << sub_y)
                        .ok_or_else(super::lr_pipeline_state_error)?;
                    let luma_window = LrSourceWindow::materialize(
                        &mut scratch.secondary,
                        PlaneId::Y,
                        curr_luma,
                        cdef_luma,
                        cdef_luma_overlap,
                        &bounds,
                        luma_block_x,
                        luma_block_y,
                        block
                            .width
                            .checked_shl(sub_x as u32)
                            .ok_or_else(super::lr_pipeline_state_error)?,
                        block
                            .height
                            .checked_shl(sub_y as u32)
                            .ok_or_else(super::lr_pipeline_state_error)?,
                        (luma_radius_x, luma_radius_y),
                    )
                    .map_err(lr_window_error)?;
                    let (luma_padded, luma_stride) = luma_window
                        .tail_from(
                            luma_block_x.saturating_sub(luma_radius_x as isize),
                            luma_block_y.saturating_sub(luma_radius_y as isize),
                        )
                        .ok_or_else(super::lr_pipeline_state_error)?;
                    WienerNsChromaPaddedSource::new(
                        chroma_padded,
                        chroma_stride,
                        luma_padded,
                        luma_stride,
                        block.width,
                        block.height,
                        (sub_x as u8, sub_y as u8),
                    )
                } else {
                    WienerNsChromaPaddedSource::chroma_only(
                        chroma_padded,
                        chroma_stride,
                        block.width,
                        block.height,
                    )
                }
                .map_err(|error| lr_plane_window_error(&error, plane_id))?;
                with_reusable_scratch(&WIENER_NS_CHROMA_SCRATCH, |scratch| {
                    wiener_ns_filter_chroma_block_padded_into(
                        output,
                        &params,
                        &padded_source,
                        scratch,
                    )
                })
                .map_err(|error| lr_plane_window_error(&error, plane_id))
            })?;
            self.preserve_lossless_lr_samples(plane_id, &block, curr_chroma, output, output_stride)
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn compute_luma_lr_block<T: ReconSample>(
        &self,
        block: &WienerNsLrSourceBlock,
        curr_luma: FramePlane<'_, T>,
        cdef_luma: &StripePlane<T>,
        cdef_luma_overlap: &[StripePlane<T>],
        qindex: u32,
        num_classes: usize,
        filter_set_index: usize,
        coeffs: &[[i16; WIENER_NS_LUMA_COEFFS]],
        post_lr: &mut StripePlane<T>,
        fill_flat: bool,
    ) -> Result<()> {
        let block = clipped_lr_source_block(
            block,
            self.luma_width,
            self.luma_height,
            self.luma_width,
            self.luma_height,
        )?;
        let bounds = crate::filters::wienerns_lr::wienerns_lr_source_block_bounds(&block, 0, 0);
        let block_x = usize_to_isize_recon(block.x, "luma LR block x")
            .map_err(|_| super::lr_pipeline_state_error())?;
        let block_y = usize_to_isize_recon(block.y, "luma LR block y")
            .map_err(|_| super::lr_pipeline_state_error())?;
        let (output, output_stride) = lr_block_destination(post_lr, &block)?;
        with_lr_source_scratch(|scratch| -> Result<()> {
            let LrSourceScratch {
                primary,
                cell_subclasses,
                ..
            } = scratch;
            let window = LrSourceRows::resolve(
                primary,
                curr_luma,
                cdef_luma,
                cdef_luma_overlap,
                &bounds,
                block_x,
                block_y,
                block.width,
                block.height,
                WIENER_NS_LUMA_TAP_RADIUS.max(PC_WIENER_CLASSIFY_READ_RADIUS),
            )
            .map_err(lr_window_error)?;
            let (rows, col) = window
                .tail(WIENER_NS_LUMA_TAP_RADIUS)
                .ok_or_else(super::lr_pipeline_state_error)?;
            for_each_lr_strip(
                rows,
                col,
                block.width,
                WIENER_NS_LUMA_TAP_RADIUS,
                self.bit_depth.max_sample(),
                |x, end, flat| {
                    if flat.is_some() && !fill_flat {
                        return Ok(());
                    }
                    let output = output
                        .get_mut(x..)
                        .ok_or_else(super::lr_pipeline_state_error)?;
                    let mut sub = block;
                    sub.x += x;
                    sub.width = end - x;
                    if let Some(value) = flat {
                        let value =
                            T::try_from_u16(value).map_err(|_| super::lr_pipeline_state_error())?;
                        return fill_lr_rect(output, output_stride, sub.width, sub.height, value);
                    }
                    let sample_count = sub
                        .width
                        .checked_mul(sub.height)
                        .ok_or_else(super::lr_pipeline_state_error)?;
                    let cell_subclass_map = if num_classes > 1 {
                        Some(self.luma_lr_cell_subclasses(
                            &sub,
                            &window,
                            qindex,
                            num_classes,
                            filter_set_index,
                            cell_subclasses,
                        )?)
                    } else {
                        None
                    };
                    let params = WienerNsLumaFilter {
                        width: sub.width,
                        height: sub.height,
                        output_stride,
                        bit_depth: self.bit_depth,
                        coeffs_by_class: coeffs,
                        subclasses: None,
                    };
                    let padded_source =
                        WienerNsLumaPaddedSource::from_rows(rows, col + x, sub.width, sub.height)
                            .map_err(lr_window_error)?;
                    let source = &padded_source;
                    with_wiener_ns_luma_scratch::<u16, _>(sample_count, |scratch| {
                        if let Some(cells) = cell_subclass_map {
                            by_depth(
                                output,
                                scratch,
                                |out, scratch| {
                                    wiener_ns_filter_luma_block_padded_cells_into(
                                        out, &params, source, cells, scratch,
                                    )
                                },
                                |out, scratch| {
                                    wiener_ns_filter_luma_block_padded_cells_u8_into(
                                        out, &params, source, cells, scratch,
                                    )
                                },
                            )
                        } else {
                            by_depth(
                                output,
                                scratch,
                                |out, scratch| {
                                    wiener_ns_filter_luma_block_padded_into(
                                        out, &params, source, scratch,
                                    )
                                },
                                |out, scratch| {
                                    wiener_ns_filter_luma_block_padded_u8_into(
                                        out, &params, source, scratch,
                                    )
                                },
                            )
                        }
                    })
                    .map_err(lr_window_error)
                },
            )
        })?;
        self.preserve_lossless_lr_samples(PlaneId::Y, &block, curr_luma, output, output_stride)
    }

    #[allow(clippy::too_many_arguments)]
    fn preserve_lossless_lr_samples<T: ReconSample>(
        &self,
        plane_id: PlaneId,
        block: &WienerNsLrSourceBlock,
        curr_plane: FramePlane<'_, T>,
        output: &mut [T],
        output_stride: usize,
    ) -> Result<()> {
        let Some(lossless_grid) = self.lossless_grid else {
            return Ok(());
        };
        let (sub_x, sub_y) = self.plane_subsampling(plane_id);
        for row in 0..block.height {
            let y = block
                .y
                .checked_add(row)
                .ok_or_else(super::lr_pipeline_state_error)?;
            let source = curr_plane
                .row(y)
                .ok_or_else(super::lr_pipeline_state_error)?;
            let start = row
                .checked_mul(output_stride)
                .ok_or_else(super::lr_pipeline_state_error)?;
            let filtered = output
                .get_mut(start..start + block.width)
                .ok_or_else(super::lr_pipeline_state_error)?;
            let mut col = 0;
            while col < block.width {
                let x = block.x + col;
                let next = (((((x << sub_x) / MI_SIZE + 1) * MI_SIZE) >> sub_x) - block.x)
                    .clamp(col + 1, block.width);
                if lossless_grid.plane_sample_lossless(plane_id, x, y, sub_x, sub_y) {
                    let kept = source
                        .get(x..x + next - col)
                        .ok_or_else(super::lr_pipeline_state_error)?;
                    filtered[col..next].copy_from_slice(kept); // splot-copy-ok: lossless samples skip loop restoration
                }
                col = next;
            }
        }
        Ok(())
    }

    fn plane_subsampling(&self, plane_id: PlaneId) -> (usize, usize) {
        match plane_id {
            PlaneId::Y => (0, 0),
            PlaneId::U | PlaneId::V => (
                usize::from(self.pixel_format.subsampling_x()),
                usize::from(self.pixel_format.subsampling_y()),
            ),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn luma_lr_cell_subclasses<'a>(
        &self,
        block: &WienerNsLrSourceBlock,
        window: &LrSourceRows<'_>,
        qindex: u32,
        num_classes: usize,
        filter_set_index: usize,
        cell_subclasses: &'a mut Vec<u8>,
    ) -> Result<&'a [u8]> {
        let cell_cols = block.width.div_ceil(MI_SIZE).max(1);
        let cell_rows = block.height.div_ceil(MI_SIZE).max(1);
        let Some(tx_skip_grid) = self.tx_skip_grid else {
            return Err(super::lr_pipeline_state_error());
        };
        let tile_start_y = mi_to_luma_start_recon(block.tile_mi_row_start, "luma LR tile start y")
            .map_err(|_| super::lr_pipeline_state_error())?;
        let tile_end_y = mi_to_luma_end_recon(block.tile_mi_row_end, "luma LR tile end y")
            .map_err(|_| super::lr_pipeline_state_error())?;
        let subclass_table =
            pc_wiener_subclass_table(num_classes, filter_set_index).map_err(lr_window_error)?;
        let padded_source = PcWienerClassifyPaddedSource::from_rows(
            &window.rows[..window.len],
            0,
            window.origin_x,
            window.origin_y,
            self.bit_depth,
        )
        .map_err(lr_window_error)?;
        let last_x = cell_cols
            .checked_sub(1)
            .and_then(|last| last.checked_mul(MI_SIZE))
            .and_then(|span| block.x.checked_add(span))
            .ok_or_else(super::lr_pipeline_state_error)?;
        let block_end_x = super::super::pc_wiener_block_end_x(block, (last_x >> 6) << 6)
            .map_err(|_| super::lr_pipeline_state_error())?;
        let params = PcWienerClassifyParams {
            x: usize_to_isize_recon(block.x, "luma LR PC-Wiener x")
                .map_err(|_| super::lr_pipeline_state_error())?,
            y: usize_to_isize_recon(block.y, "luma LR PC-Wiener y")
                .map_err(|_| super::lr_pipeline_state_error())?,
            bit_depth: self.bit_depth,
            base_q_idx: qindex,
            block_start_x: (block.x >> 6) << 6,
            block_end_x,
            luma_stripe_start_y: block.luma_stripe_start_y,
            luma_stripe_end_y: block.luma_stripe_end_y,
            tile_start_y,
            tile_end_y,
        };
        with_reusable_scratch(&PC_WIENER_CLASSIFY_SCRATCH, |scratch| {
            pc_wiener_classify_grid_padded_classes_into::<u16, _>(
                &params,
                cell_cols,
                cell_rows,
                &padded_source,
                |run| tx_skip_grid.run(run.row, run.col, run.len),
                subclass_table,
                scratch,
                cell_subclasses,
            )
            .map_err(lr_window_error)
        })
    }
}

#[cfg(test)]
#[path = "final_filters_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "final_filters_direct_tests.rs"]
mod direct_tests;
