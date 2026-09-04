// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Final loop-filter application for the reconstruction sink.

use super::{MI_SIZE, StripeChain, WienerNsLrReconSink};
use crate::Result;
use crate::bitstream::tile_payload::{
    LrUnitRestorationType, WienerNsLrSourceBlock, WienerNsLrUnitFilter,
};
use crate::filters::cdef::{CdefFrame, CdefSkipGrid};
use crate::filters::source::{
    FramePlane, StripeCopyError, StripeInitialization, StripeOutputPlane, StripePlane,
};
use crate::support::reusable_scratch::with_reusable_scratch;
use splot_core::headers::frame::{FrameHeaderCore, FrameRestorationType, LrPlaneParams};
use splot_recon::{
    BitDepth, LoopRestorationSource, LoopRestorationSourceBounds, PC_WIENER_CLASSIFY_READ_RADIUS,
    PC_WIENER_FILTER_TAP_RADIUS, PC_WIENER_FULL_CLASSES, PcWienerClassifyPaddedSource,
    PcWienerClassifyParams, PcWienerClassifyScratch, PcWienerFilter, PcWienerPaddedSource, PlaneId,
    PlaneRect, ReconError, ReconSample, Result as ReconResult, WIENER_NS_CHROMA_COEFFS,
    WIENER_NS_CHROMA_TAP_RADIUS, WIENER_NS_LUMA_COEFFS, WIENER_NS_LUMA_TAP_RADIUS,
    WienerNsChromaFilter, WienerNsChromaPaddedSource, WienerNsChromaScratch, WienerNsLumaFilter,
    WienerNsLumaPaddedSource, WienerNsLumaScratch, loop_restoration_source_sample,
    pc_wiener_classify_grid_padded_classes_into, pc_wiener_filter_block_padded,
    pc_wiener_filter_block_padded_u16_into, pc_wiener_filter_set_index, pc_wiener_subclass_table,
    wiener_ns_filter_chroma_block_padded_u8_into, wiener_ns_filter_chroma_block_padded_u16_into,
    wiener_ns_filter_luma_block_padded_cells_into,
    wiener_ns_filter_luma_block_padded_cells_u8_into,
    wiener_ns_filter_luma_block_padded_cells_u16_into, wiener_ns_filter_luma_block_padded_into,
    wiener_ns_filter_luma_block_padded_u8_into, wiener_ns_filter_luma_block_padded_u16_into,
};

thread_local! {
    static PC_WIENER_CLASSIFY_SCRATCH: std::cell::RefCell<PcWienerClassifyScratch> =
        std::cell::RefCell::new(PcWienerClassifyScratch::default());
    static WIENER_NS_LUMA_SCRATCH: std::cell::Cell<Option<Box<dyn std::any::Any>>> =
        const { std::cell::Cell::new(None) };
    static WIENER_NS_CHROMA_SCRATCH: std::cell::Cell<Option<Box<dyn std::any::Any>>> =
        const { std::cell::Cell::new(None) };
    static LR_SOURCE_SCRATCH: std::cell::Cell<Option<Box<dyn std::any::Any>>> =
        const { std::cell::Cell::new(None) };
    #[cfg(test)]
    static LR_OUTPUT_SCRATCH: std::cell::Cell<Option<Box<dyn std::any::Any>>> =
        const { std::cell::Cell::new(None) };
}

fn with_wiener_ns_chroma_scratch<T: ReconSample, R>(
    f: impl FnOnce(&mut WienerNsChromaScratch<T>) -> R,
) -> R {
    WIENER_NS_CHROMA_SCRATCH.with(|slot| {
        let mut scratch = slot
            .take()
            .and_then(|scratch| scratch.downcast::<WienerNsChromaScratch<T>>().ok())
            .unwrap_or_default();
        let result = f(&mut scratch);
        slot.set(Some(scratch));
        result
    })
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
struct LrSourceScratch<T> {
    primary: Vec<T>,
    secondary: Vec<T>,
    cell_subclasses: Vec<usize>,
}

pub(crate) struct LrFrame<'a, T> {
    pub(crate) deblocked_y: FramePlane<'a, T>,
    pub(crate) deblocked_u: Option<FramePlane<'a, T>>,
    pub(crate) deblocked_v: Option<FramePlane<'a, T>>,
    pub(crate) cdef_y: StripePlane,
    pub(crate) cdef_u: Option<StripePlane>,
    pub(crate) cdef_v: Option<StripePlane>,
    pub(crate) post_lr_y: Option<StripeOutputPlane>,
    pub(crate) post_lr_u: Option<StripeOutputPlane>,
    pub(crate) post_lr_v: Option<StripeOutputPlane>,
}

pub(crate) struct FilteredStripe {
    pub(crate) y: StripeOutputPlane,
    pub(crate) u: Option<StripeOutputPlane>,
    pub(crate) v: Option<StripeOutputPlane>,
}

pub(crate) struct LrStripeOutput {
    pub(crate) active_planes: [bool; 3],
    pub(crate) direct_u8_planes: [bool; 3],
    pub(crate) initializations: [StripeInitialization; 3],
    pub(crate) target: crate::pipeline::frame_progress::DirectStripeTarget,
}

impl<'a, T: ReconSample> LrFrame<'a, T> {
    fn from_cdef(
        frame: CdefFrame<'a, T>,
        active_planes: [bool; 3],
        direct_u8_planes: [bool; 3],
        initializations: [StripeInitialization; 3],
        mut target: crate::pipeline::frame_progress::DirectStripeTarget,
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
            let direct_target = target.get(plane_id).ok_or(StripeCopyError::Geometry)?;
            if direct_u8_planes[plane_id.index()] && direct_target.is_u16() {
                return Err(StripeCopyError::Geometry);
            }
            plane.preflight_copy_rows_into(
                plane.origin_y(),
                plane.end_y().ok_or(StripeCopyError::Geometry)?,
                Some(direct_target),
                initializations[plane_id.index()],
            )?;
        }
        let mut copy = |plane_id: PlaneId, plane: &StripePlane| {
            let target = target.take(plane_id).ok_or(StripeCopyError::Geometry)?;
            if direct_u8_planes[plane_id.index()] {
                return StripeOutputPlane::direct_u8(target, plane);
            }
            plane
                .copy_rows_into_mode(
                    plane.origin_y(),
                    plane.end_y().ok_or(StripeCopyError::Geometry)?,
                    Some(target),
                    initializations[plane_id.index()],
                )
                .map(StripeOutputPlane::u16)
        };
        let post_lr_y = if active_planes[PlaneId::Y.index()] {
            Some(copy(PlaneId::Y, &frame.filtered_y).map_err(|error| error.for_plane(PlaneId::Y))?)
        } else {
            None
        };
        let post_lr_u = if active_planes[PlaneId::U.index()] {
            match frame.filtered_u.as_ref() {
                Some(plane) => {
                    Some(copy(PlaneId::U, plane).map_err(|error| error.for_plane(PlaneId::U))?)
                }
                None => None,
            }
        } else {
            None
        };
        let post_lr_v = if active_planes[PlaneId::V.index()] {
            match frame.filtered_v.as_ref() {
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
        })
    }

    pub(crate) fn into_filtered(self) -> FilteredStripe {
        FilteredStripe {
            y: self
                .post_lr_y
                .unwrap_or_else(|| StripeOutputPlane::u16(self.cdef_y)),
            u: self
                .post_lr_u
                .or_else(|| self.cdef_u.map(StripeOutputPlane::u16)),
            v: self
                .post_lr_v
                .or_else(|| self.cdef_v.map(StripeOutputPlane::u16)),
        }
    }
}

impl<T> LrSourceScratch<T> {
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

fn with_lr_source_scratch<T: ReconSample, R>(f: impl FnOnce(&mut LrSourceScratch<T>) -> R) -> R {
    LR_SOURCE_SCRATCH.with(|slot| {
        let mut scratch = slot
            .take()
            .and_then(|scratch| scratch.downcast::<LrSourceScratch<T>>().ok())
            .unwrap_or_default();
        let result = f(&mut scratch);
        if scratch.is_bounded() {
            slot.set(Some(scratch));
        }
        result
    })
}

#[cfg(test)]
fn with_lr_output_scratch<T: ReconSample, R>(f: impl FnOnce(&mut Vec<T>) -> R) -> R {
    LR_OUTPUT_SCRATCH.with(|slot| {
        let mut output = slot
            .take()
            .and_then(|output| output.downcast::<Vec<T>>().ok())
            .unwrap_or_default();
        let result = f(&mut output);
        if output.capacity() <= MAX_RETAINED_LR_SCRATCH_ELEMENTS {
            slot.set(Some(output));
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

#[cfg(test)]
fn coalesced_lr_source_rows(
    lr_source_blocks: &[WienerNsLrSourceBlock],
    plane_index: usize,
) -> Vec<WienerNsLrSourceBlock> {
    let (blocks, [y_end, u_end]) = coalesced_lr_source_rows_all(lr_source_blocks.to_vec());
    let starts = [0, y_end, u_end];
    let ends = [y_end, u_end, blocks.len()];
    blocks[starts[plane_index]..ends[plane_index]].to_vec()
}

pub(crate) fn coalesced_lr_source_rows_all(
    mut blocks: Vec<WienerNsLrSourceBlock>,
) -> (Vec<WienerNsLrSourceBlock>, [usize; 2]) {
    blocks.retain(|block| block.plane < 3);
    blocks.sort_unstable_by_key(|block| (block.plane, block.y, block.x));
    blocks.dedup_by(|next, run| {
        let Some(width) = run.merged_width_with(next) else {
            return false;
        };
        run.width = width;
        true
    });

    blocks.sort_unstable_by_key(|block| (block.vertical_merge_key(), block.y));
    blocks.dedup_by(|next, rectangle| {
        let Some(height) = rectangle.merged_height_with(next) else {
            return false;
        };
        rectangle.height = height;
        true
    });
    blocks.sort_unstable_by_key(|block| (block.plane, block.y, block.x));

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

pub(crate) fn terminal_luma_wiener_covers(
    blocks: &[WienerNsLrSourceBlock],
    width: usize,
    frame_height: usize,
    start_y: usize,
    end_y: usize,
) -> bool {
    lr_plane_fully_overwritten(
        blocks,
        PlaneId::Y,
        FrameRestorationType::WienerNonsep,
        width,
        frame_height,
        start_y,
        end_y,
    )
}

pub(crate) fn terminal_luma_wiener_direct_u8(
    bit_depth: BitDepth,
    frame_type: Option<FrameRestorationType>,
    gdf_active: bool,
    blocks: &[WienerNsLrSourceBlock],
    target: &crate::pipeline::frame_progress::DirectPlaneTarget,
) -> bool {
    bit_depth == BitDepth::Eight
        && !target.is_u16()
        && !gdf_active
        && frame_type.is_some_and(|frame_type| {
            matches!(
                frame_type,
                FrameRestorationType::WienerNonsep | FrameRestorationType::Switchable
            )
        })
        && target.end_y().is_some_and(|end_y| {
            luma_stripe_units_all_wiener_nonsep(blocks, target.origin_y(), end_y)
                && terminal_luma_wiener_covers(
                    blocks,
                    target.width(),
                    target.frame_height(),
                    target.origin_y(),
                    end_y,
                )
        })
}

/// The direct-`u8` luma sink has no `u16` post-LR stripe, so a PC-Wiener unit
/// dispatched into it would fail. Reject the whole stripe unless every unit the
/// LR walk will visit is Wiener-NS.
fn luma_stripe_units_all_wiener_nonsep(
    blocks: &[WienerNsLrSourceBlock],
    start_y: usize,
    end_y: usize,
) -> bool {
    lr_blocks_near_rows(blocks, start_y, end_y)
        .iter()
        .filter(|block| lr_block_in_rows(block, start_y, end_y).is_some())
        .all(|block| block.restoration_type == LrUnitRestorationType::WienerNonsep)
}

pub(crate) fn lr_initializations(
    core: &FrameHeaderCore,
    active_planes: [bool; 3],
    plane_blocks: [&[WienerNsLrSourceBlock]; 3],
    target: &crate::pipeline::frame_progress::DirectStripeTarget,
) -> [StripeInitialization; 3] {
    core::array::from_fn(|index| {
        let plane = [PlaneId::Y, PlaneId::U, PlaneId::V][index];
        let Some(target) = target.get(plane).filter(|target| target.is_u16()) else {
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

/// Runs one loop-restoration block filter over its own destination rows.
///
/// No LR block reads the post-LR stripe, so `u16` storage hands the filter the
/// destination rectangle itself and pays no staging copy. Narrower storage
/// still stages, because the stripe always holds `u16`.
#[cfg(test)]
fn filter_lr_block_into<T: ReconSample>(
    plane: &mut StripePlane,
    block: &WienerNsLrSourceBlock,
    filter: impl FnOnce(&mut [T], usize) -> Result<()>,
) -> Result<()> {
    let (destination, stride) = lr_block_destination(plane, block)?;
    if let Some(output) = T::from_u16_slice_mut(destination) {
        return filter(output, stride);
    }
    let sample_count = block
        .width
        .checked_mul(block.height)
        .ok_or_else(super::lr_pipeline_state_error)?;
    with_lr_output_scratch(|staged: &mut Vec<T>| {
        staged.clear();
        staged.resize(sample_count, T::default());
        filter(staged, block.width)?;
        for (row, samples) in staged.chunks_exact(block.width).enumerate() {
            let start = row
                .checked_mul(stride)
                .ok_or_else(super::lr_pipeline_state_error)?;
            let end = start
                .checked_add(block.width)
                .ok_or_else(super::lr_pipeline_state_error)?;
            let row = destination
                .get_mut(start..end)
                .ok_or_else(super::lr_pipeline_state_error)?;
            for (slot, sample) in row.iter_mut().zip(samples) {
                *slot = sample.to_u16();
            }
        }
        Ok(())
    })
}

fn lr_block_destination<'a>(
    plane: &'a mut StripePlane,
    block: &WienerNsLrSourceBlock,
) -> Result<(&'a mut [u16], usize)> {
    if block
        .x
        .checked_add(block.width)
        .is_none_or(|end_x| end_x > plane.width())
    {
        return Err(super::lr_pipeline_state_error());
    }
    let rect = splot_recon::PlaneRect::new(block.x, block.y, block.width, block.height)
        .map_err(|_| super::lr_pipeline_state_error())?;
    let (destination, stride) = plane
        .rect_mut(rect)
        .ok_or_else(super::lr_pipeline_state_error)?;
    Ok((destination, stride))
}

enum LrDestination<'a> {
    U16(&'a mut [u16]),
    U8(&'a mut [u8]),
}

fn chroma_lr_block_destination<'a>(
    plane: &'a mut StripeOutputPlane,
    block: &WienerNsLrSourceBlock,
) -> Result<(LrDestination<'a>, usize)> {
    if block
        .x
        .checked_add(block.width)
        .is_none_or(|end_x| end_x > plane.width())
    {
        return Err(super::lr_pipeline_state_error());
    }
    let rect = PlaneRect::new(block.x, block.y, block.width, block.height)
        .map_err(|_| super::lr_pipeline_state_error())?;
    match plane {
        StripeOutputPlane::U16(plane) => {
            let (destination, stride) = plane
                .rect_mut(rect)
                .ok_or_else(super::lr_pipeline_state_error)?;
            Ok((LrDestination::U16(destination), stride))
        }
        StripeOutputPlane::DirectU8(_) => {
            let (destination, stride) = plane
                .u8_rect_mut(rect)
                .ok_or_else(super::lr_pipeline_state_error)?;
            Ok((LrDestination::U8(destination), stride))
        }
    }
}

/// Post-CCSO rows adjacent to a stripe band, split per plane.
///
/// § 7.20.2 sources every in-stripe sample from `CdefFrame`, and § 7.20.1
/// stripes restart per tile row, so a band abutting a tile-row boundary needs
/// deringed rows the band itself never covers.
#[derive(Default)]
pub(crate) struct CdefOverlap {
    pub(crate) y: Vec<StripePlane>,
    pub(crate) u: Vec<StripePlane>,
    pub(crate) v: Vec<StripePlane>,
}

impl CdefOverlap {
    fn plane(&self, plane_id: PlaneId) -> &[StripePlane] {
        match plane_id {
            PlaneId::Y => &self.y,
            PlaneId::U => &self.u,
            PlaneId::V => &self.v,
        }
    }
}

struct LrSourceWindow<'a, T> {
    samples: &'a [T],
    stride: usize,
    origin_x: isize,
    origin_y: isize,
}

enum LrSourceRow<'a, T> {
    Curr(&'a [T]),
    Cdef(&'a [u16]),
}

impl<T: ReconSample> LrSourceRow<'_, T> {
    fn len(&self) -> usize {
        match self {
            Self::Curr(row) => row.len(),
            Self::Cdef(row) => row.len(),
        }
    }

    fn get(&self, index: usize) -> Option<u16> {
        match self {
            Self::Curr(row) => row.get(index).map(|sample| sample.to_u16()),
            Self::Cdef(row) => row.get(index).copied(),
        }
    }
}

impl<'a, T: ReconSample> LrSourceWindow<'a, T> {
    /// Resolves one padded § 7.17 source window into `samples`.
    ///
    /// Every row is written across the whole stride, so a buffer that already
    /// holds enough samples keeps its previous contents instead of being
    /// cleared and refilled.
    #[allow(clippy::too_many_arguments)]
    fn materialize(
        samples: &'a mut Vec<T>,
        plane: PlaneId,
        curr_plane: FramePlane<'_, T>,
        cdef_plane: &StripePlane,
        cdef_overlap: &[StripePlane],
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
        let sample_count = stride.checked_mul(rows).ok_or(OVERFLOW_WINDOW)?;
        let radius_x = isize::try_from(radius_x).map_err(|_| OVERFLOW_WINDOW)?;
        let radius_y = isize::try_from(radius_y).map_err(|_| OVERFLOW_WINDOW)?;
        if let Some(missing) = sample_count.checked_sub(samples.len()).filter(|n| *n > 0) {
            samples.try_reserve_exact(missing).map_err(|_| {
                ReconError::WorkspaceAllocationFailed {
                    plane,
                    context: "LR source window",
                }
            })?;
            samples.resize(sample_count, T::default());
        }
        for row_index in 0..rows {
            let y = block_y
                .checked_sub(radius_y)
                .and_then(|top| top.checked_add(isize::try_from(row_index).ok()?))
                .ok_or(OVERFLOW_WINDOW)?;
            let left = loop_restoration_source_sample(plane, isize::MIN, y, bounds)?;
            let right = loop_restoration_source_sample(plane, isize::MAX, y, bounds)?;
            if right.x >= plane_width || left.y >= plane_height {
                return Err(ReconError::PcWienerInvalidBounds {
                    field: "LR source frame bounds",
                });
            }
            let source_row = match left.source {
                LoopRestorationSource::CurrFrame => {
                    LrSourceRow::Curr(curr_plane.row(left.y).ok_or(
                        ReconError::BufferLengthMismatch {
                            expected: left.y.saturating_add(1),
                            actual: curr_plane.frame_height(),
                        },
                    )?)
                }
                LoopRestorationSource::CdefFrame => LrSourceRow::Cdef(
                    cdef_plane
                        .row(left.y)
                        .or_else(|| cdef_overlap.iter().find_map(|plane| plane.row(left.y)))
                        .ok_or(ReconError::BufferLengthMismatch {
                            expected: left.y.saturating_add(1),
                            actual: cdef_plane.end_y().unwrap_or(cdef_plane.origin_y()),
                        })?,
                ),
            };
            let min_x = isize::try_from(left.x).map_err(|_| OVERFLOW_WINDOW)?;
            let max_x = isize::try_from(right.x).map_err(|_| OVERFLOW_WINDOW)?;
            let x0 = block_x.checked_sub(radius_x).ok_or(OVERFLOW_WINDOW)?;
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
            let mid = stride - pre - post;
            let left_value = T::try_from_u16(source_row.get(left.x).ok_or(
                ReconError::BufferLengthMismatch {
                    expected: left.x.saturating_add(1),
                    actual: source_row.len(),
                },
            )?)?;
            let row_start = row_index * stride;
            samples[row_start..row_start + pre].fill(left_value);
            if mid > 0 {
                let mid_start = (x0 + pre as isize) as usize;
                let mid_end = mid_start.saturating_add(mid);
                let missing = ReconError::BufferLengthMismatch {
                    expected: mid_end,
                    actual: source_row.len(),
                };
                let output = &mut samples[row_start + pre..row_start + pre + mid];
                match &source_row {
                    LrSourceRow::Curr(row) => {
                        output.copy_from_slice(row.get(mid_start..mid_end).ok_or(missing)?);
                    }
                    LrSourceRow::Cdef(row) => {
                        let source = row.get(mid_start..mid_end).ok_or(missing)?;
                        for (output, &value) in output.iter_mut().zip(source) {
                            *output = T::try_from_u16(value)?;
                        }
                    }
                }
            }
            let right_value = T::try_from_u16(source_row.get(right.x).ok_or(
                ReconError::BufferLengthMismatch {
                    expected: right.x.saturating_add(1),
                    actual: source_row.len(),
                },
            )?)?;
            samples[row_start + pre + mid..row_start + stride].fill(right_value);
        }
        Ok(Self {
            samples: samples.get(..sample_count).ok_or(OVERFLOW_WINDOW)?,
            stride,
            origin_x: block_x.checked_sub(radius_x).ok_or(OVERFLOW_WINDOW)?,
            origin_y: block_y.checked_sub(radius_y).ok_or(OVERFLOW_WINDOW)?,
        })
    }

    fn tail_from(&self, x: isize, y: isize) -> Option<(&[T], usize)> {
        let col = usize::try_from(x.checked_sub(self.origin_x)?).ok()?;
        let row = usize::try_from(y.checked_sub(self.origin_y)?).ok()?;
        if col >= self.stride {
            return None;
        }
        let start = row.checked_mul(self.stride)?.checked_add(col)?;
        self.samples.get(start..).map(|tail| (tail, self.stride))
    }

    #[cfg(test)]
    fn get_abs(&self, x: isize, y: isize) -> T {
        let col = x.saturating_sub(self.origin_x);
        let row = y.saturating_sub(self.origin_y);
        if col < 0 || row < 0 || col as usize >= self.stride {
            return T::default();
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
        cdef_overlap: &CdefOverlap,
        plane_blocks: [&[WienerNsLrSourceBlock]; 3],
        lr_unit_filters: &[WienerNsLrUnitFilter],
        output: LrStripeOutput,
    ) -> Result<LrFrame<'a, T>> {
        let mut frame = LrFrame::from_cdef(
            cdef,
            output.active_planes,
            output.direct_u8_planes,
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
                        let post_lr_y = post_lr_y
                            .as_u16_mut()
                            .ok_or_else(super::lr_pipeline_state_error)?;
                        self.compute_pc_wiener_block(
                            &block,
                            frame.deblocked_y,
                            &frame.cdef_y,
                            &cdef_overlap.y,
                            qindex,
                            filter_set_index,
                            post_lr_y,
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
        cdef_luma: &StripePlane,
        cdef_luma_overlap: &[StripePlane],
        qindex: u32,
        filter_set_index: usize,
        post_lr: &mut StripePlane,
    ) -> Result<()> {
        let block = clipped_lr_source_block(
            block,
            self.luma_width,
            self.luma_height,
            self.luma_width,
            self.luma_height,
        )?;
        let sample_count = block
            .width
            .checked_mul(block.height)
            .ok_or_else(super::lr_pipeline_state_error)?;
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
            let window = LrSourceWindow::<T>::materialize(
                primary,
                PlaneId::Y,
                curr_luma,
                cdef_luma,
                cdef_luma_overlap,
                &bounds,
                block_x,
                block_y,
                block.width,
                block.height,
                {
                    let radius = PC_WIENER_CLASSIFY_READ_RADIUS.max(PC_WIENER_FILTER_TAP_RADIUS);
                    (radius, radius)
                },
            )
            .map_err(lr_window_error)?;
            let subclass_map = self.luma_lr_cell_subclasses(
                &block,
                &window,
                qindex,
                PC_WIENER_FULL_CLASSES,
                filter_set_index,
                sample_count,
                cell_subclasses,
            )?;
            let params = PcWienerFilter {
                width: block.width,
                height: block.height,
                output_stride,
                bit_depth: self.bit_depth,
                filter_set_index,
                subclass_block_size: MI_SIZE,
                subclasses: subclass_map,
            };
            let tap_radius = isize::try_from(PC_WIENER_FILTER_TAP_RADIUS)
                .map_err(|_| super::lr_pipeline_state_error())?;
            let (padded, padded_stride) = window
                .tail_from(
                    block_x.saturating_sub(tap_radius),
                    block_y.saturating_sub(tap_radius),
                )
                .ok_or_else(super::lr_pipeline_state_error)?;
            let padded_source =
                PcWienerPaddedSource::new(padded, padded_stride, block.width, block.height)
                    .map_err(lr_window_error)?;
            if let Some(output) = T::from_u16_slice_mut(output) {
                pc_wiener_filter_block_padded(output, &params, &padded_source)
                    .map_err(lr_window_error)?;
            } else {
                pc_wiener_filter_block_padded_u16_into(output, &params, &padded_source)
                    .map_err(lr_window_error)?;
            }
            self.preserve_lossless_lr_samples(
                PlaneId::Y,
                &block,
                curr_luma,
                output,
                output_stride,
                |slot, sample| *slot = sample.to_u16(),
            )
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
        cdef_chroma: &StripePlane,
        cdef_chroma_overlap: &[StripePlane],
        curr_luma: FramePlane<'_, T>,
        cdef_luma: &StripePlane,
        cdef_luma_overlap: &[StripePlane],
        post_lr: &mut StripeOutputPlane,
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
        let (mut output, output_stride) = chroma_lr_block_destination(post_lr, &block)?;
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
                let chroma_window = LrSourceWindow::<T>::materialize(
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
                let luma_radius_x = WIENER_NS_CHROMA_TAP_RADIUS << sub_x;
                let luma_radius_y = WIENER_NS_CHROMA_TAP_RADIUS << sub_y;
                let luma_block_x = block_x
                    .checked_mul(1 << sub_x)
                    .ok_or_else(super::lr_pipeline_state_error)?;
                let luma_block_y = block_y
                    .checked_mul(1 << sub_y)
                    .ok_or_else(super::lr_pipeline_state_error)?;
                let luma_window = LrSourceWindow::<T>::materialize(
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
                let radius = WIENER_NS_CHROMA_TAP_RADIUS as isize;
                let (chroma_padded, chroma_stride) = chroma_window
                    .tail_from(
                        block_x.saturating_sub(radius),
                        block_y.saturating_sub(radius),
                    )
                    .ok_or_else(super::lr_pipeline_state_error)?;
                let (luma_padded, luma_stride) = luma_window
                    .tail_from(
                        luma_block_x.saturating_sub(luma_radius_x as isize),
                        luma_block_y.saturating_sub(luma_radius_y as isize),
                    )
                    .ok_or_else(super::lr_pipeline_state_error)?;
                let padded_source = WienerNsChromaPaddedSource::new(
                    chroma_padded,
                    chroma_stride,
                    luma_padded,
                    luma_stride,
                    block.width,
                    block.height,
                    (sub_x as u8, sub_y as u8),
                )
                .map_err(|error| lr_plane_window_error(&error, plane_id))?;
                with_wiener_ns_chroma_scratch(|scratch| match &mut output {
                    LrDestination::U16(output) => wiener_ns_filter_chroma_block_padded_u16_into(
                        output,
                        &params,
                        &padded_source,
                        scratch,
                    ),
                    LrDestination::U8(output) => wiener_ns_filter_chroma_block_padded_u8_into(
                        output,
                        &params,
                        &padded_source,
                        scratch,
                    ),
                })
                .map_err(|error| lr_plane_window_error(&error, plane_id))
            })?;
            match output {
                LrDestination::U16(output) => self.preserve_lossless_lr_samples(
                    plane_id,
                    &block,
                    curr_chroma,
                    output,
                    output_stride,
                    |slot, sample| *slot = sample.to_u16(),
                ),
                LrDestination::U8(output) => self.preserve_lossless_lr_samples(
                    plane_id,
                    &block,
                    curr_chroma,
                    output,
                    output_stride,
                    |slot, sample| *slot = sample.to_u16() as u8,
                ),
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn compute_luma_lr_block<T: ReconSample>(
        &self,
        block: &WienerNsLrSourceBlock,
        curr_luma: FramePlane<'_, T>,
        cdef_luma: &StripePlane,
        cdef_luma_overlap: &[StripePlane],
        qindex: u32,
        num_classes: usize,
        filter_set_index: usize,
        coeffs: &[[i16; WIENER_NS_LUMA_COEFFS]],
        post_lr: &mut StripeOutputPlane,
    ) -> Result<()> {
        let block = clipped_lr_source_block(
            block,
            self.luma_width,
            self.luma_height,
            self.luma_width,
            self.luma_height,
        )?;
        let sample_count = block
            .width
            .checked_mul(block.height)
            .ok_or_else(super::lr_pipeline_state_error)?;
        let bounds = crate::filters::wienerns_lr::wienerns_lr_source_block_bounds(&block, 0, 0);
        let block_x = usize_to_isize_recon(block.x, "luma LR block x")
            .map_err(|_| super::lr_pipeline_state_error())?;
        let block_y = usize_to_isize_recon(block.y, "luma LR block y")
            .map_err(|_| super::lr_pipeline_state_error())?;
        let (mut output, output_stride) = chroma_lr_block_destination(post_lr, &block)?;
        with_lr_source_scratch(|scratch| -> Result<()> {
            let LrSourceScratch {
                primary,
                cell_subclasses,
                ..
            } = scratch;
            let window = LrSourceWindow::<T>::materialize(
                primary,
                PlaneId::Y,
                curr_luma,
                cdef_luma,
                cdef_luma_overlap,
                &bounds,
                block_x,
                block_y,
                block.width,
                block.height,
                {
                    let radius = WIENER_NS_LUMA_TAP_RADIUS.max(PC_WIENER_CLASSIFY_READ_RADIUS);
                    (radius, radius)
                },
            )
            .map_err(lr_window_error)?;
            let cell_subclass_map = if num_classes > 1 {
                Some(self.luma_lr_cell_subclasses(
                    &block,
                    &window,
                    qindex,
                    num_classes,
                    filter_set_index,
                    sample_count,
                    cell_subclasses,
                )?)
            } else {
                None
            };
            let params = WienerNsLumaFilter {
                width: block.width,
                height: block.height,
                output_stride,
                bit_depth: self.bit_depth,
                coeffs_by_class: coeffs,
                subclasses: None,
            };
            let tap_radius = isize::try_from(WIENER_NS_LUMA_TAP_RADIUS)
                .map_err(|_| super::lr_pipeline_state_error())?;
            let (padded, padded_stride) = window
                .tail_from(
                    block_x.saturating_sub(tap_radius),
                    block_y.saturating_sub(tap_radius),
                )
                .ok_or_else(super::lr_pipeline_state_error)?;
            let padded_source =
                WienerNsLumaPaddedSource::new(padded, padded_stride, block.width, block.height)
                    .map_err(lr_window_error)?;
            with_wiener_ns_luma_scratch(sample_count, |scratch| match &mut output {
                LrDestination::U16(output) => {
                    if let Some(output) = T::from_u16_slice_mut(output) {
                        if let Some(cell_subclasses) = cell_subclass_map {
                            wiener_ns_filter_luma_block_padded_cells_into(
                                output,
                                &params,
                                &padded_source,
                                cell_subclasses,
                                scratch,
                            )
                        } else {
                            wiener_ns_filter_luma_block_padded_into(
                                output,
                                &params,
                                &padded_source,
                                scratch,
                            )
                        }
                    } else if let Some(cell_subclasses) = cell_subclass_map {
                        wiener_ns_filter_luma_block_padded_cells_u16_into(
                            output,
                            &params,
                            &padded_source,
                            cell_subclasses,
                            scratch,
                        )
                    } else {
                        wiener_ns_filter_luma_block_padded_u16_into(
                            output,
                            &params,
                            &padded_source,
                            scratch,
                        )
                    }
                }
                LrDestination::U8(output) => {
                    if let Some(cell_subclasses) = cell_subclass_map {
                        wiener_ns_filter_luma_block_padded_cells_u8_into(
                            output,
                            &params,
                            &padded_source,
                            cell_subclasses,
                            scratch,
                        )
                    } else {
                        wiener_ns_filter_luma_block_padded_u8_into(
                            output,
                            &params,
                            &padded_source,
                            scratch,
                        )
                    }
                }
            })
            .map_err(lr_window_error)?;
            Ok(())
        })?;
        match output {
            LrDestination::U16(output) => self.preserve_lossless_lr_samples(
                PlaneId::Y,
                &block,
                curr_luma,
                output,
                output_stride,
                |slot, sample| *slot = sample.to_u16(),
            ),
            LrDestination::U8(output) => self.preserve_lossless_lr_samples(
                PlaneId::Y,
                &block,
                curr_luma,
                output,
                output_stride,
                |slot, sample| *slot = sample.to_u16() as u8,
            ),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn preserve_lossless_lr_samples<T: ReconSample, O>(
        &self,
        plane_id: PlaneId,
        block: &WienerNsLrSourceBlock,
        curr_plane: FramePlane<'_, T>,
        output: &mut [O],
        output_stride: usize,
        mut write_sample: impl FnMut(&mut O, T),
    ) -> Result<()> {
        let Some(lossless_grid) = self.lossless_grid else {
            return Ok(());
        };
        let (sub_x, sub_y) = self.plane_subsampling(plane_id);
        for row in 0..block.height {
            for col in 0..block.width {
                let x = block
                    .x
                    .checked_add(col)
                    .ok_or_else(super::lr_pipeline_state_error)?;
                let y = block
                    .y
                    .checked_add(row)
                    .ok_or_else(super::lr_pipeline_state_error)?;
                if !lossless_grid.plane_sample_lossless(plane_id, x, y, sub_x, sub_y) {
                    continue;
                }
                let output_index = row
                    .checked_mul(output_stride)
                    .and_then(|start| start.checked_add(col))
                    .ok_or_else(super::lr_pipeline_state_error)?;
                let sample = *curr_plane
                    .row(y)
                    .and_then(|row| row.get(x))
                    .ok_or_else(super::lr_pipeline_state_error)?;
                let Some(slot) = output.get_mut(output_index) else {
                    return Err(super::lr_pipeline_state_error());
                };
                write_sample(slot, sample);
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
    fn luma_lr_cell_subclasses<'a, T: ReconSample>(
        &self,
        block: &WienerNsLrSourceBlock,
        window: &LrSourceWindow<'_, T>,
        qindex: u32,
        num_classes: usize,
        filter_set_index: usize,
        sample_count: usize,
        cell_subclasses: &'a mut Vec<usize>,
    ) -> Result<&'a [usize]> {
        if sample_count
            != block
                .width
                .checked_mul(block.height)
                .ok_or_else(super::lr_pipeline_state_error)?
        {
            return Err(super::lr_pipeline_state_error());
        }
        let cell_cols = block.width.div_ceil(MI_SIZE).max(1);
        let cell_rows = block.height.div_ceil(MI_SIZE).max(1);
        let Some(tx_skip_grid) = self.tx_skip_grid else {
            return Err(super::lr_pipeline_state_error());
        };
        let tile_start_y = mi_to_luma_start_recon(block.tile_mi_row_start, "luma LR tile start y")
            .map_err(|_| super::lr_pipeline_state_error())?;
        let tile_end_y = mi_to_luma_end_recon(block.tile_mi_row_end, "luma LR tile end y")
            .map_err(|_| super::lr_pipeline_state_error())?;
        let cell_count = cell_cols
            .checked_mul(cell_rows)
            .ok_or_else(super::lr_pipeline_state_error)?;
        if cell_count > cell_subclasses.len() {
            let additional = cell_count - cell_subclasses.len();
            if cell_subclasses.try_reserve_exact(additional).is_err() {
                cell_subclasses.clear();
                return Err(ReconError::WorkspaceAllocationFailed {
                    plane: PlaneId::Y,
                    context: "PC-Wiener cell subclasses",
                }
                .into());
            }
            cell_subclasses.resize(cell_count, 0);
        } else {
            cell_subclasses.truncate(cell_count);
        }
        let subclass_table =
            pc_wiener_subclass_table(num_classes, filter_set_index).map_err(lr_window_error)?;
        let padded_source = PcWienerClassifyPaddedSource::new_prevalidated(
            window.samples,
            window.stride,
            window.origin_x,
            window.origin_y,
            self.bit_depth,
        )
        .map_err(lr_window_error)?;
        let mut group_start = 0;
        while group_start < cell_cols {
            let class_x = block
                .x
                .checked_add(group_start.saturating_mul(MI_SIZE))
                .ok_or_else(super::lr_pipeline_state_error)?;
            let block_start_x = (class_x >> 6) << 6;
            let mut group_end = group_start + 1;
            while group_end < cell_cols {
                let next_x = block
                    .x
                    .checked_add(group_end.saturating_mul(MI_SIZE))
                    .ok_or_else(super::lr_pipeline_state_error)?;
                if ((next_x >> 6) << 6) != block_start_x {
                    break;
                }
                group_end += 1;
            }
            let block_end_x = super::super::pc_wiener_block_end_x(block, block_start_x)
                .map_err(|_| super::lr_pipeline_state_error())?;
            let params = PcWienerClassifyParams {
                x: usize_to_isize_recon(class_x, "luma LR PC-Wiener x")
                    .map_err(|_| super::lr_pipeline_state_error())?,
                y: usize_to_isize_recon(block.y, "luma LR PC-Wiener y")
                    .map_err(|_| super::lr_pipeline_state_error())?,
                bit_depth: self.bit_depth,
                base_q_idx: qindex,
                block_start_x,
                block_end_x,
                luma_stripe_start_y: block.luma_stripe_start_y,
                luma_stripe_end_y: block.luma_stripe_end_y,
                tile_start_y,
                tile_end_y,
            };
            let group_cols = group_end - group_start;
            with_reusable_scratch(&PC_WIENER_CLASSIFY_SCRATCH, |scratch| {
                let classes = pc_wiener_classify_grid_padded_classes_into::<T, _>(
                    &params,
                    group_cols,
                    cell_rows,
                    &padded_source,
                    |lookup| {
                        tx_skip_grid.lookup(
                            crate::filters::wienerns_lr::wienerns_lr_tx_skip_lookup_from_pc(lookup),
                        )
                    },
                    scratch,
                )
                .map_err(lr_window_error)?;
                for cell_row in 0..cell_rows {
                    let class_start = cell_row
                        .checked_mul(group_cols)
                        .ok_or_else(super::lr_pipeline_state_error)?;
                    let class_end = class_start
                        .checked_add(group_cols)
                        .ok_or_else(super::lr_pipeline_state_error)?;
                    let cell_start = cell_row
                        .checked_mul(cell_cols)
                        .and_then(|start| start.checked_add(group_start))
                        .ok_or_else(super::lr_pipeline_state_error)?;
                    let cell_end = cell_start
                        .checked_add(group_cols)
                        .ok_or_else(super::lr_pipeline_state_error)?;
                    let Some(classes) = classes.get(class_start..class_end) else {
                        return Err(super::lr_pipeline_state_error());
                    };
                    let Some(cells) = cell_subclasses.get_mut(cell_start..cell_end) else {
                        return Err(super::lr_pipeline_state_error());
                    };
                    for (cell, &class) in cells.iter_mut().zip(classes) {
                        *cell = usize::from(subclass_table[usize::from(class)]);
                    }
                }
                Ok(())
            })?;
            group_start = group_end;
        }

        Ok(cell_subclasses)
    }
}

#[cfg(test)]
#[path = "final_filters_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "final_filters_direct_tests.rs"]
mod direct_tests;
