// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#[cfg(test)]
use splot_recon::BitDepth;
use splot_recon::{
    CurrentFrameWorkspace, DecodedFrame, DecodedFrameInfo, InterpolationFilter, PixelFormat,
    PlaneId, PlaneRect, ReconError, ReconSample, ReferencePlaneView, SubpelPredictParams,
    WARPED_BLOCK_SIZE, WarpPredictBlockParams, blend_compound_average_weighted_sample,
    ext_warp_predict_unit, subpel_predict_block, subpel_predict_block_compound_intermediate,
    subpel_predict_block_compound_intermediate_into, subpel_predict_block_into,
    warp_predict_block_into, wedge_mask_plane_sample,
};

use super::mv_scaling::{PlaneScaling, derive_plane_scaling};
use super::{Mv, SPEC_MC, unsupported_at};
use crate::Result;
use splot_core::span::ByteOffset;
use splot_recon::math::{clip3, round2_i32};

mod optflow;
mod refinemv;
pub(crate) mod sink;
pub(crate) use optflow::CompoundMotionGrid;
use optflow::MotionCell;
pub(crate) use sink::{BlockReconWindow, WorkspaceSink};

pub(crate) const fn mc_planes(pixel_format: PixelFormat) -> [(PlaneId, u32, u32); 3] {
    let sub_x = pixel_format.subsampling_x() as u32;
    let sub_y = pixel_format.subsampling_y() as u32;
    [
        (PlaneId::Y, 0, 0),
        (PlaneId::U, sub_x, sub_y),
        (PlaneId::V, sub_x, sub_y),
    ]
}
pub(crate) const CWP_EQUAL: i16 = 8;

fn copy_u16_samples<T: ReconSample>(samples: &[u16], output: &mut [T]) -> splot_recon::Result<()> {
    if samples.len() != output.len() {
        return Err(ReconError::BufferLengthMismatch {
            expected: output.len(),
            actual: samples.len(),
        });
    }
    for &sample in samples {
        T::try_from_u16(sample)?;
    }
    for (output, &sample) in output.iter_mut().zip(samples) {
        *output = T::try_from_u16(sample)?;
    }
    Ok(())
}

fn pack_samples<T: ReconSample>(samples: &[u16]) -> splot_recon::Result<Vec<T>> {
    let mut packed = Vec::with_capacity(samples.len());
    for &sample in samples {
        packed.push(T::try_from_u16(sample)?);
    }
    Ok(packed)
}

fn clip_and_pack_warp_samples<T: ReconSample, const N: usize>(
    samples: &[i32; N],
    max_sample: i32,
) -> splot_recon::Result<[T; N]> {
    let mut packed = [T::default(); N];
    for (dst, &sample) in packed.iter_mut().zip(samples) {
        let clipped = sample.clamp(0, max_sample) as u16;
        *dst = T::try_from_u16(clipped)?;
    }
    Ok(packed)
}

pub(super) const fn optflow_unit_size(luma_w: usize, luma_h: usize) -> usize {
    if luma_w <= 8 && luma_h <= 8 { 4 } else { 8 }
}

#[allow(clippy::struct_field_names)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct McBlockRect {
    pub(crate) luma_x: usize,
    pub(crate) luma_y: usize,
    pub(crate) luma_w: usize,
    pub(crate) luma_h: usize,
    pub(crate) chroma_luma_x: usize,
    pub(crate) chroma_luma_y: usize,
    pub(crate) chroma_luma_w: usize,
    pub(crate) chroma_luma_h: usize,
}

impl McBlockRect {
    pub(crate) const fn from_luma_rect(
        luma_x: usize,
        luma_y: usize,
        luma_w: usize,
        luma_h: usize,
    ) -> Self {
        Self {
            luma_x,
            luma_y,
            luma_w,
            luma_h,
            chroma_luma_x: luma_x,
            chroma_luma_y: luma_y,
            chroma_luma_w: luma_w,
            chroma_luma_h: luma_h,
        }
    }

    const fn plane_luma_rect(self, plane: PlaneId) -> (usize, usize, usize, usize) {
        match plane {
            PlaneId::Y => (self.luma_x, self.luma_y, self.luma_w, self.luma_h),
            PlaneId::U | PlaneId::V => (
                self.chroma_luma_x,
                self.chroma_luma_y,
                self.chroma_luma_w,
                self.chroma_luma_h,
            ),
        }
    }

    fn plane_rect(self, plane: PlaneId, sub_x: u32, sub_y: u32) -> (usize, usize, usize, usize) {
        let (x, y, width, height) = self.plane_luma_rect(plane);
        let scale_x = 1usize << sub_x;
        let scale_y = 1usize << sub_y;
        let plane_x = x >> sub_x;
        let plane_y = y >> sub_y;
        (
            plane_x,
            plane_y,
            x.saturating_add(width)
                .div_ceil(scale_x)
                .saturating_sub(plane_x),
            y.saturating_add(height)
                .div_ceil(scale_y)
                .saturating_sub(plane_y),
        )
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CompoundBlend {
    Average {
        implicit_mask: bool,
        cwp_weight: i16,
    },
    DiffWeighted {
        inverse: bool,
    },
    Wedge {
        index: u8,
        sign: bool,
    },
}

impl CompoundBlend {
    pub(crate) const fn average_with_implicit_mask(implicit_mask: bool) -> Self {
        Self::Average {
            implicit_mask,
            cwp_weight: CWP_EQUAL,
        }
    }

    pub(crate) const fn average_with_cwp_weight(self, cwp_weight: i16) -> Self {
        match self {
            Self::Average { implicit_mask, .. } => Self::Average {
                implicit_mask,
                cwp_weight,
            },
            other => other,
        }
    }

    pub(crate) const fn cwp_weight(self) -> i16 {
        match self {
            Self::Average { cwp_weight, .. } => cwp_weight,
            Self::DiffWeighted { .. } | Self::Wedge { .. } => CWP_EQUAL,
        }
    }
}

impl Default for CompoundBlend {
    fn default() -> Self {
        Self::average_with_implicit_mask(false)
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct InterBlockParams<'a, T: ReconSample> {
    rect: McBlockRect,
    prediction: InterPrediction<'a, T>,
    interp: InterpolationFilter,
    has_chroma: bool,
    sub8x8_chroma: bool,
    use_refinemv: bool,
    search_refinemv: bool,
    refinemv_switchable: bool,
    optflow_sad_threshold: Option<u32>,
}

impl<'a, T: ReconSample> InterBlockParams<'a, T> {
    pub(crate) const fn single(
        reference: &'a DecodedFrame<T>,
        rect: McBlockRect,
        mv: Mv,
        interp: InterpolationFilter,
    ) -> Self {
        Self {
            rect,
            prediction: InterPrediction::Single { reference, mv },
            interp,
            has_chroma: true,
            sub8x8_chroma: false,
            use_refinemv: false,
            search_refinemv: false,
            refinemv_switchable: false,
            optflow_sad_threshold: None,
        }
    }
    pub(crate) const fn compound_average(
        reference0: &'a DecodedFrame<T>,
        reference1: &'a DecodedFrame<T>,
        rect: McBlockRect,
        mv0: Mv,
        mv1: Mv,
        interp: InterpolationFilter,
        blend: CompoundBlend,
    ) -> Self {
        Self {
            rect,
            prediction: InterPrediction::CompoundAverage {
                reference0,
                reference1,
                mv0,
                mv1,
                blend,
                optflow_distances: None,
                warp_params: [None, None],
            },
            interp,
            has_chroma: true,
            sub8x8_chroma: false,
            use_refinemv: false,
            search_refinemv: false,
            refinemv_switchable: false,
            optflow_sad_threshold: None,
        }
    }
    pub(crate) const fn single_warp(
        reference: &'a DecodedFrame<T>,
        rect: McBlockRect,
        mv: Mv,
        interp: InterpolationFilter,
        warp_params: [i32; 6],
    ) -> Self {
        Self {
            rect,
            prediction: InterPrediction::SingleWarp {
                reference,
                mv,
                warp_params,
            },
            interp,
            has_chroma: true,
            sub8x8_chroma: false,
            use_refinemv: false,
            search_refinemv: false,
            refinemv_switchable: false,
            optflow_sad_threshold: None,
        }
    }
    pub(crate) const fn with_chroma(mut self, has_chroma: bool) -> Self {
        self.has_chroma = has_chroma;
        self
    }

    pub(crate) const fn with_refinemv(mut self, use_refinemv: bool) -> Self {
        self.use_refinemv = use_refinemv;
        self.search_refinemv = use_refinemv;
        self
    }

    pub(crate) const fn with_refinemv_search(mut self, enabled: bool) -> Self {
        self.search_refinemv = self.use_refinemv && enabled;
        self
    }

    pub(crate) const fn with_switchable_refinemv(mut self, switchable: bool) -> Self {
        self.refinemv_switchable = switchable;
        self
    }

    pub(crate) const fn with_sub8x8_chroma(mut self, enabled: bool) -> Self {
        self.sub8x8_chroma = enabled;
        self
    }

    pub(crate) fn with_optflow_distances(mut self, distances: Option<[i32; 2]>) -> Self {
        if let InterPrediction::CompoundAverage {
            optflow_distances, ..
        } = &mut self.prediction
        {
            *optflow_distances = distances;
        }
        self
    }

    pub(crate) const fn with_optflow_sad_threshold(mut self, threshold: Option<u32>) -> Self {
        self.optflow_sad_threshold = threshold;
        self
    }

    pub(crate) fn with_compound_warp(mut self, models: [Option<[i32; 6]>; 2]) -> Self {
        if let InterPrediction::CompoundAverage { warp_params, .. } = &mut self.prediction {
            *warp_params = models;
        }
        self
    }

    pub(crate) fn into_compound(self) -> Option<CompoundMcBlock<'a, T>> {
        let InterPrediction::CompoundAverage {
            reference0,
            reference1,
            mv0,
            mv1,
            blend,
            optflow_distances,
            warp_params,
        } = self.prediction
        else {
            return None;
        };
        Some(CompoundMcBlock {
            reference0,
            reference1,
            rect: self.rect,
            mv0,
            mv1,
            interp: self.interp,
            blend,
            optflow_distances,
            warp_params,
            has_chroma: self.has_chroma,
            sub8x8_chroma: self.sub8x8_chroma,
            use_refinemv: self.use_refinemv,
            search_refinemv: self.search_refinemv,
            refinemv_switchable: self.refinemv_switchable,
            optflow_sad_threshold: self.optflow_sad_threshold,
        })
    }
}
#[derive(Clone, Copy, Debug)]
enum InterPrediction<'a, T: ReconSample> {
    Single {
        reference: &'a DecodedFrame<T>,
        mv: Mv,
    },
    SingleWarp {
        reference: &'a DecodedFrame<T>,
        mv: Mv,
        warp_params: [i32; 6],
    },
    CompoundAverage {
        reference0: &'a DecodedFrame<T>,
        reference1: &'a DecodedFrame<T>,
        mv0: Mv,
        mv1: Mv,
        blend: CompoundBlend,
        optflow_distances: Option<[i32; 2]>,
        warp_params: [Option<[i32; 6]>; 2],
    },
}
#[derive(Clone, Copy, Debug)]
pub(crate) struct CompoundMcBlock<'a, T: ReconSample> {
    reference0: &'a DecodedFrame<T>,
    reference1: &'a DecodedFrame<T>,
    rect: McBlockRect,
    mv0: Mv,
    mv1: Mv,
    interp: InterpolationFilter,
    blend: CompoundBlend,
    optflow_distances: Option<[i32; 2]>,
    warp_params: [Option<[i32; 6]>; 2],
    has_chroma: bool,
    sub8x8_chroma: bool,
    use_refinemv: bool,
    search_refinemv: bool,
    refinemv_switchable: bool,
    optflow_sad_threshold: Option<u32>,
}

#[derive(Debug)]
pub(super) struct CompoundBlockMetadata {
    rect: McBlockRect,
    has_chroma: bool,
    motion: Option<CompoundMotionGrid>,
}

impl CompoundBlockMetadata {
    pub(super) fn stored_mvs_at_origin(&self) -> splot_recon::Result<Option<[Mv; 2]>> {
        self.motion
            .as_ref()
            .map(|motion| motion.stored_mvs_at_luma_offset(0, 0))
            .transpose()
    }

    pub(super) fn publish<T: ReconSample>(
        &self,
        samples: &[T],
        sink: &mut WorkspaceSink<'_, T>,
    ) -> Result<()> {
        let mut sample_start = 0usize;
        for (plane, sub_x, sub_y) in mc_planes(sink.info().pixel_format()) {
            if plane != PlaneId::Y && !self.has_chroma {
                continue;
            }
            let (plane_x, plane_y, block_w, block_h) = self.rect.plane_rect(plane, sub_x, sub_y);
            let sample_count =
                block_w
                    .checked_mul(block_h)
                    .ok_or(ReconError::ArithmeticOverflow {
                        context: "compound output plane sample count",
                    })?;
            let sample_end =
                sample_start
                    .checked_add(sample_count)
                    .ok_or(ReconError::ArithmeticOverflow {
                        context: "compound output plane sample range",
                    })?;
            let plane_samples =
                samples
                    .get(sample_start..sample_end)
                    .ok_or(ReconError::BufferLengthMismatch {
                        expected: sample_end,
                        actual: samples.len(),
                    })?;
            let plane_rect = PlaneRect::new(plane_x, plane_y, block_w, block_h)?;
            sink.write_rect(plane, plane_rect, plane_samples, block_w)?;
            sample_start = sample_end;
        }
        Ok(())
    }
}

pub(crate) struct CompoundBlockOutput<T> {
    metadata: CompoundBlockMetadata,
    samples: Vec<T>,
}

impl<T: ReconSample> CompoundBlockOutput<T> {
    pub(crate) fn publish(
        self,
        sink: &mut WorkspaceSink<'_, T>,
    ) -> Result<Option<CompoundMotionGrid>> {
        self.metadata.publish(&self.samples, sink)?;
        Ok(self.metadata.motion)
    }
}
pub(crate) fn motion_compensate_inter_block_into<T: ReconSample>(
    sink: &mut WorkspaceSink<'_, T>,
    block: InterBlockParams<'_, T>,
    offset: ByteOffset,
) -> Result<()> {
    motion_compensate_inter_block(sink, block, None, offset).map(drop)
}

pub(crate) fn motion_compensate_inter_block_with_optflow_mvs_into<T: ReconSample>(
    sink: &mut WorkspaceSink<'_, T>,
    block: InterBlockParams<'_, T>,
    optflow_unit_size: usize,
    offset: ByteOffset,
) -> Result<Option<[Mv; 2]>> {
    let Some(grid) = motion_compensate_inter_block(sink, block, Some(optflow_unit_size), offset)?
    else {
        return Ok(None);
    };
    Ok(Some(grid.stored_mvs_at_luma_offset(0, 0)?))
}

pub(crate) fn motion_compensate_inter_block_with_motion_grid_into<T: ReconSample>(
    sink: &mut WorkspaceSink<'_, T>,
    block: InterBlockParams<'_, T>,
    optflow_unit_size: Option<usize>,
    offset: ByteOffset,
) -> Result<Option<CompoundMotionGrid>> {
    motion_compensate_inter_block(sink, block, optflow_unit_size, offset)
}

fn motion_compensate_inter_block<T: ReconSample>(
    sink: &mut WorkspaceSink<'_, T>,
    block: InterBlockParams<'_, T>,
    optflow_unit_size: Option<usize>,
    offset: ByteOffset,
) -> Result<Option<CompoundMotionGrid>> {
    match block.prediction {
        InterPrediction::Single { reference, mv } => {
            motion_compensate_single_block_into(
                sink,
                reference,
                block.rect,
                mv,
                block.interp,
                block.has_chroma,
                offset,
            )?;
            Ok(None)
        }
        InterPrediction::SingleWarp {
            reference,
            mv,
            warp_params,
        } => {
            motion_compensate_single_warp_block_into(
                sink,
                reference,
                block,
                mv,
                warp_params,
                offset,
            )?;
            Ok(None)
        }
        InterPrediction::CompoundAverage {
            reference0,
            reference1,
            mv0,
            mv1,
            blend,
            optflow_distances,
            warp_params,
        } => motion_compensate_compound_average_block_into(
            sink,
            CompoundMcBlock {
                reference0,
                reference1,
                rect: block.rect,
                mv0,
                mv1,
                interp: block.interp,
                blend,
                optflow_distances,
                warp_params,
                has_chroma: block.has_chroma,
                sub8x8_chroma: block.sub8x8_chroma,
                use_refinemv: block.use_refinemv,
                search_refinemv: block.search_refinemv,
                refinemv_switchable: block.refinemv_switchable,
                optflow_sad_threshold: block.optflow_sad_threshold,
            },
            optflow_unit_size,
            offset,
        ),
    }
}

fn motion_compensate_single_block_into<T: ReconSample>(
    sink: &mut WorkspaceSink<'_, T>,
    reference: &DecodedFrame<T>,
    rect: McBlockRect,
    mv: Mv,
    interp: InterpolationFilter,
    has_chroma: bool,
    offset: ByteOffset,
) -> Result<()> {
    motion_compensate_planes(sink, has_chroma, |sink, plane, sub_x, sub_y| {
        predict_plane(
            sink, reference, plane, rect, mv, interp, sub_x, sub_y, offset,
        )
    })
}

fn motion_compensate_planes<T: ReconSample>(
    sink: &mut WorkspaceSink<'_, T>,
    has_chroma: bool,
    mut predict: impl FnMut(&mut WorkspaceSink<'_, T>, PlaneId, u32, u32) -> Result<()>,
) -> Result<()> {
    for (plane, sub_x, sub_y) in mc_planes(sink.info().pixel_format()) {
        if plane != PlaneId::Y && !has_chroma {
            continue;
        }
        predict(sink, plane, sub_x, sub_y)?;
    }
    Ok(())
}

fn motion_compensate_compound_average_block_into<T: ReconSample>(
    sink: &mut WorkspaceSink<'_, T>,
    block: CompoundMcBlock<'_, T>,
    optflow_unit_size: Option<usize>,
    offset: ByteOffset,
) -> Result<Option<CompoundMotionGrid>> {
    predict_compound_average_block(sink, block, optflow_unit_size, offset)?.publish(sink)
}

pub(crate) fn predict_compound_average_block<T: ReconSample>(
    sink: &WorkspaceSink<'_, T>,
    block: CompoundMcBlock<'_, T>,
    optflow_unit_size: Option<usize>,
    offset: ByteOffset,
) -> Result<CompoundBlockOutput<T>> {
    let sample_count =
        compound_output_sample_count(block.rect, block.has_chroma, sink.info().pixel_format())?;
    let mut samples = vec![T::default(); sample_count];
    let metadata =
        predict_compound_average_block_into(sink, block, optflow_unit_size, offset, &mut samples)?;
    Ok(CompoundBlockOutput { metadata, samples })
}

pub(super) fn predict_compound_average_block_into<T: ReconSample>(
    sink: &WorkspaceSink<'_, T>,
    block: CompoundMcBlock<'_, T>,
    optflow_unit_size: Option<usize>,
    offset: ByteOffset,
    samples: &mut [T],
) -> Result<CompoundBlockMetadata> {
    let sample_count =
        compound_output_sample_count(block.rect, block.has_chroma, sink.info().pixel_format())?;
    let available_samples = samples.len();
    let mut samples = samples
        .get_mut(..sample_count)
        .ok_or(ReconError::BufferLengthMismatch {
            expected: sample_count,
            actual: available_samples,
        })?;
    let refinemv = block
        .use_refinemv
        .then(|| refinemv::compound_default_refinemv_motion_grid(sink, block, offset))
        .transpose()?;
    let motion = optflow::compound_motion_grid(sink, block, optflow_unit_size, refinemv, offset)?;
    let luma_diff_weighted_mask =
        compound_luma_diff_weighted_mask(sink, block, motion.as_ref(), offset)?;
    for (plane, sub_x, sub_y) in mc_planes(sink.info().pixel_format()) {
        if plane != PlaneId::Y && !block.has_chroma {
            continue;
        }
        let (_, _, block_w, block_h) = block.rect.plane_rect(plane, sub_x, sub_y);
        let plane_sample_count =
            block_w
                .checked_mul(block_h)
                .ok_or(ReconError::ArithmeticOverflow {
                    context: "compound output plane sample count",
                })?;
        let (plane_samples, remaining_samples) = samples.split_at_mut(plane_sample_count);
        samples = remaining_samples;
        if plane != PlaneId::Y && block.sub8x8_chroma {
            with_plane_prediction(
                sink.info(),
                block.reference0,
                plane,
                block.rect,
                block.mv0,
                block.interp,
                sub_x,
                sub_y,
                offset,
                |_, predicted, _| {
                    copy_u16_samples(predicted, plane_samples)?;
                    Ok(())
                },
            )?;
        } else {
            predict_compound_plane_output(
                sink,
                block.reference0,
                block.reference1,
                plane,
                block.rect,
                block.mv0,
                block.mv1,
                block.interp,
                block.blend,
                block.warp_params,
                sub_x,
                sub_y,
                luma_diff_weighted_mask.as_deref(),
                motion.as_ref(),
                offset,
                plane_samples,
            )?;
        }
    }
    Ok(CompoundBlockMetadata {
        rect: block.rect,
        has_chroma: block.has_chroma,
        motion,
    })
}

fn compound_output_sample_count(
    rect: McBlockRect,
    has_chroma: bool,
    pixel_format: PixelFormat,
) -> Result<usize> {
    let mut sample_count = 0usize;
    for (plane, sub_x, sub_y) in mc_planes(pixel_format) {
        if plane != PlaneId::Y && !has_chroma {
            continue;
        }
        let (_, _, block_w, block_h) = rect.plane_rect(plane, sub_x, sub_y);
        let plane_samples = block_w
            .checked_mul(block_h)
            .ok_or(ReconError::ArithmeticOverflow {
                context: "compound output plane sample count",
            })?;
        sample_count =
            sample_count
                .checked_add(plane_samples)
                .ok_or(ReconError::ArithmeticOverflow {
                    context: "compound output sample count",
                })?;
    }
    Ok(sample_count)
}

fn compound_luma_diff_weighted_mask<T: ReconSample>(
    sink: &WorkspaceSink<'_, T>,
    block: CompoundMcBlock<'_, T>,
    motion: Option<&CompoundMotionGrid>,
    offset: ByteOffset,
) -> Result<Option<Vec<u16>>> {
    let CompoundBlend::DiffWeighted { inverse } = block.blend else {
        return Ok(None);
    };
    let prediction =
        compound_plane_prediction_for_block(sink, block, PlaneId::Y, 0, 0, motion, offset)?;
    Ok(Some(diff_weighted_mask(
        &prediction.pred0,
        &prediction.pred1,
        sink.info().bit_depth(),
        prediction.block_w,
        prediction.block_h,
        inverse,
    )?))
}

fn motion_compensate_single_warp_block_into<T: ReconSample>(
    sink: &mut WorkspaceSink<'_, T>,
    reference: &DecodedFrame<T>,
    block: InterBlockParams<'_, T>,
    mv: Mv,
    warp_params: [i32; 6],
    offset: ByteOffset,
) -> Result<()> {
    motion_compensate_planes(sink, block.has_chroma, |sink, plane, sub_x, sub_y| {
        if plane != PlaneId::Y && block.sub8x8_chroma {
            predict_plane(
                sink,
                reference,
                plane,
                block.rect,
                mv,
                block.interp,
                sub_x,
                sub_y,
                offset,
            )
        } else {
            predict_warp_plane(
                sink,
                reference,
                plane,
                block.rect,
                warp_params,
                sub_x,
                sub_y,
                offset,
            )
        }
    })
}

#[allow(clippy::too_many_arguments)]
fn predict_plane<T: ReconSample>(
    sink: &mut WorkspaceSink<'_, T>,
    reference: &DecodedFrame<T>,
    plane: PlaneId,
    rect: McBlockRect,
    mv: Mv,
    interp: InterpolationFilter,
    sub_x: u32,
    sub_y: u32,
    offset: ByteOffset,
) -> Result<()> {
    with_plane_prediction(
        sink.info(),
        reference,
        plane,
        rect,
        mv,
        interp,
        sub_x,
        sub_y,
        offset,
        |rect, predicted, stride| {
            sink.write_u16_rect(plane, rect, predicted, stride)?;
            Ok(())
        },
    )
}

#[allow(clippy::too_many_arguments)]
fn with_plane_prediction<T: ReconSample>(
    info: DecodedFrameInfo,
    reference: &DecodedFrame<T>,
    plane: PlaneId,
    rect: McBlockRect,
    mv: Mv,
    interp: InterpolationFilter,
    sub_x: u32,
    sub_y: u32,
    offset: ByteOffset,
    consume: impl FnOnce(PlaneRect, &[u16], usize) -> Result<()>,
) -> Result<()> {
    let (view, _, _) = reference_plane_view(reference, plane, offset)?;

    let (plane_x, plane_y, block_w, block_h) = rect.plane_rect(plane, sub_x, sub_y);
    let reference_size = reference.info().coded_luma_size();
    let frame_size = info.coded_luma_size();

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

    let params = SubpelPredictParams {
        interp,
        w: block_w,
        h: block_h,
        start_x: scaling.start_x,
        start_y: scaling.start_y,
        step_x: scaling.step_x,
        step_y: scaling.step_y,
        first_x: scaling.first_x,
        first_y: scaling.first_y,
        last_x: scaling.last_x,
        last_y: scaling.last_y,
        bit_depth: info.bit_depth(),
    };
    let rect = PlaneRect::new(plane_x, plane_y, block_w, block_h)?;
    let len = block_w
        .checked_mul(block_h)
        .ok_or(ReconError::ArithmeticOverflow {
            context: "single-reference prediction sample count",
        })?;
    SUBPEL_PREDICTION_BUFFER.with(|slot| {
        let mut predicted = slot.take().unwrap_or_default();
        predicted.resize(len, 0);
        let result: Result<()> = (|| {
            subpel_predict_block_into(&view, &params, &mut predicted)?;
            consume(rect, &predicted, block_w)
        })();
        slot.set(Some(predicted));
        result
    })
}

#[allow(clippy::too_many_arguments)]
fn predict_warp_plane<T: ReconSample>(
    sink: &mut WorkspaceSink<'_, T>,
    reference: &DecodedFrame<T>,
    plane: PlaneId,
    rect: McBlockRect,
    warp_params: [i32; 6],
    sub_x: u32,
    sub_y: u32,
    offset: ByteOffset,
) -> Result<()> {
    let (view, ref_mi_cols, ref_mi_rows) = reference_plane_view(reference, plane, offset)?;
    let (ref_width, ref_height) = (view.width(), view.height());
    let destination_size = sink.plane_storage_size(plane)?;
    let (destination_width, destination_height) =
        (destination_size.width(), destination_size.height());

    let (plane_x, plane_y, block_w, block_h) = rect.plane_rect(plane, sub_x, sub_y);
    let bit_depth = sink.info().bit_depth();
    let reference_size = reference.info().coded_luma_size();
    let frame_size = sink.info().coded_luma_size();
    let scaling = derive_plane_scaling(
        plane_x as i32,
        plane_y as i32,
        0,
        0,
        sub_x,
        sub_y,
        reference_size.width() as i32,
        reference_size.height() as i32,
        frame_size.width() as i32,
        frame_size.height() as i32,
    );
    let skip_pred = !splot_recon::warp_shear_is_valid(warp_params)
        || block_w < WARPED_BLOCK_SIZE
        || block_h < WARPED_BLOCK_SIZE
        || scaling.is_scaled();
    if skip_pred {
        for i4 in 0..block_h.div_euclid(4) {
            for j4 in 0..block_w.div_euclid(4) {
                let write_x = plane_x + j4 * 4;
                let write_y = plane_y + i4 * 4;
                if write_x >= destination_width || write_y >= destination_height {
                    continue;
                }
                let unit_x = (plane_x + (j4 & !1) * 4) as i32;
                let unit_y = (plane_y + (i4 & !1) * 4) as i32;
                let (first_x, first_y, last_x, last_y) = ext_warp_unit_bounds(
                    rect,
                    plane,
                    warp_params,
                    unit_x,
                    unit_y,
                    block_w.min(8) as i32,
                    block_h.min(8) as i32,
                    sub_x,
                    sub_y,
                    ref_mi_cols,
                    ref_mi_rows,
                    scaling,
                );
                let params = WarpPredictBlockParams {
                    warp_params,
                    block_x: plane_x as i32,
                    block_y: plane_y as i32,
                    subsampling_x: sub_x as u8,
                    subsampling_y: sub_y as u8,
                    reference_scale_x: scaling.scale_x,
                    reference_scale_y: scaling.scale_y,
                    first_x,
                    first_y,
                    last_x,
                    last_y,
                    bit_depth,
                };
                let predicted = ext_warp_predict_unit(&view, &params, i4, j4, false)?;
                let packed =
                    clip_and_pack_warp_samples(&predicted, i32::from(bit_depth.max_sample()))?;
                let rect = PlaneRect::new(write_x, write_y, 4, 4)?;
                sink.write_rect(plane, rect, &packed, 4)?;
            }
        }
        return Ok(());
    }

    for local_y in (0..block_h).step_by(WARPED_BLOCK_SIZE) {
        for local_x in (0..block_w).step_by(WARPED_BLOCK_SIZE) {
            let write_x = plane_x + local_x;
            let write_y = plane_y + local_y;
            if write_x >= destination_width || write_y >= destination_height {
                continue;
            }
            let params = WarpPredictBlockParams {
                warp_params,
                block_x: write_x as i32,
                block_y: write_y as i32,
                subsampling_x: sub_x as u8,
                subsampling_y: sub_y as u8,
                reference_scale_x: scaling.scale_x,
                reference_scale_y: scaling.scale_y,
                first_x: 0,
                first_y: 0,
                last_x: ref_width as i32 - 1,
                last_y: ref_height as i32 - 1,
                bit_depth,
            };
            let mut predicted = [0i32; WARPED_BLOCK_SIZE * WARPED_BLOCK_SIZE];
            warp_predict_block_into(&view, &params, false, &mut predicted)?;
            let write_w = (block_w - local_x).min(WARPED_BLOCK_SIZE);
            let write_h = (block_h - local_y).min(WARPED_BLOCK_SIZE);
            let packed = clip_and_pack_warp_samples(&predicted, i32::from(bit_depth.max_sample()))?;
            let rect = PlaneRect::new(write_x, write_y, write_w, write_h)?;
            sink.write_rect(plane, rect, &packed, WARPED_BLOCK_SIZE)?;
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn predict_compound_plane_output<T: ReconSample>(
    sink: &WorkspaceSink<'_, T>,
    reference0: &DecodedFrame<T>,
    reference1: &DecodedFrame<T>,
    plane: PlaneId,
    rect: McBlockRect,
    mv0: Mv,
    mv1: Mv,
    interp: InterpolationFilter,
    blend: CompoundBlend,
    warp_params: [Option<[i32; 6]>; 2],
    sub_x: u32,
    sub_y: u32,
    luma_diff_weighted_mask: Option<&[u16]>,
    motion: Option<&CompoundMotionGrid>,
    offset: ByteOffset,
    samples: &mut [T],
) -> Result<()> {
    let block = CompoundMcBlock {
        reference0,
        reference1,
        rect,
        mv0,
        mv1,
        interp,
        blend,
        optflow_distances: None,
        warp_params,
        has_chroma: true,
        sub8x8_chroma: false,
        use_refinemv: false,
        search_refinemv: false,
        refinemv_switchable: false,
        optflow_sad_threshold: None,
    };
    let prediction =
        compound_plane_prediction_for_block(sink, block, plane, sub_x, sub_y, motion, offset)?;
    let coded_luma_size = sink.info().coded_luma_size();
    let frame_w = (coded_luma_size.width().div_ceil(4) * 4) >> sub_x;
    let frame_h = (coded_luma_size.height().div_ceil(4) * 4) >> sub_y;
    blend_compound_average::<T>(
        &prediction.pred0,
        &prediction.pred1,
        sink.info().bit_depth(),
        prediction.block_w,
        prediction.block_h,
        blend,
        rect.luma_w,
        rect.luma_h,
        motion,
        prediction.plane_x,
        prediction.plane_y,
        prediction.scaling0,
        prediction.scaling1,
        frame_w,
        frame_h,
        luma_diff_weighted_mask,
        sub_x,
        sub_y,
        samples,
    )?;

    Ok(())
}

struct CompoundPlanePrediction {
    pred0: Vec<i32>,
    pred1: Vec<i32>,
    plane_x: usize,
    plane_y: usize,
    block_w: usize,
    block_h: usize,
    scaling0: PlaneScaling,
    scaling1: PlaneScaling,
    recycle_buffers: bool,
}

std::thread_local! {
    static COMPOUND_PREDICTION_BUFFERS: std::cell::Cell<Option<[Vec<i32>; 2]>> =
        const { std::cell::Cell::new(None) };
    static INITIAL_LUMA_PREDICTIONS: std::cell::Cell<Option<Vec<u16>>> =
        const { std::cell::Cell::new(None) };
    static SUBPEL_PREDICTION_BUFFER: std::cell::Cell<Option<Vec<u16>>> =
        const { std::cell::Cell::new(None) };
}

fn take_compound_prediction_buffers(len: usize) -> [Vec<i32>; 2] {
    COMPOUND_PREDICTION_BUFFERS.with(|slot| {
        let mut buffers = slot.take().unwrap_or_default();
        for buffer in &mut buffers {
            buffer.resize(len, 0);
        }
        buffers
    })
}

fn with_initial_luma_predictions<R>(
    width: usize,
    height: usize,
    predict: impl FnOnce(&mut [u16], &mut [u16]) -> Result<R>,
) -> Result<R> {
    let len = width
        .checked_mul(height)
        .ok_or(ReconError::ArithmeticOverflow {
            context: "initial luma prediction sample count",
        })?;
    let storage_len = len.checked_mul(2).ok_or(ReconError::ArithmeticOverflow {
        context: "paired initial luma prediction sample count",
    })?;
    INITIAL_LUMA_PREDICTIONS.with(|slot| {
        let mut storage = slot.take().unwrap_or_default();
        storage.resize(storage_len, 0);
        let (pred0, pred1) = storage.split_at_mut(len);
        let result = predict(pred0, pred1);
        slot.set(Some(storage));
        result
    })
}

impl Drop for CompoundPlanePrediction {
    fn drop(&mut self) {
        if self.recycle_buffers {
            let buffers = [
                core::mem::take(&mut self.pred0),
                core::mem::take(&mut self.pred1),
            ];
            COMPOUND_PREDICTION_BUFFERS.with(|slot| slot.set(Some(buffers)));
        }
    }
}

fn compound_plane_prediction_for_block<T: ReconSample>(
    sink: &WorkspaceSink<'_, T>,
    block: CompoundMcBlock<'_, T>,
    plane: PlaneId,
    sub_x: u32,
    sub_y: u32,
    motion: Option<&CompoundMotionGrid>,
    offset: ByteOffset,
) -> Result<CompoundPlanePrediction> {
    if let Some(motion) = motion {
        return optflow::compound_optflow_plane_prediction(
            sink, block, plane, sub_x, sub_y, motion, offset,
        );
    }
    if block.warp_params[0].is_some() || block.warp_params[1].is_some() {
        return compound_warp_plane_prediction(sink, block, plane, sub_x, sub_y, offset);
    }
    compound_plane_prediction(
        sink,
        block.reference0,
        block.reference1,
        plane,
        block.rect,
        block.mv0,
        block.mv1,
        block.interp,
        sub_x,
        sub_y,
        offset,
    )
}

#[allow(clippy::too_many_arguments)]
fn compound_plane_prediction<T: ReconSample>(
    sink: &WorkspaceSink<'_, T>,
    reference0: &DecodedFrame<T>,
    reference1: &DecodedFrame<T>,
    plane: PlaneId,
    rect: McBlockRect,
    mv0: Mv,
    mv1: Mv,
    interp: InterpolationFilter,
    sub_x: u32,
    sub_y: u32,
    offset: ByteOffset,
) -> Result<CompoundPlanePrediction> {
    let (view0, _, _) = reference_plane_view(reference0, plane, offset)?;
    let (view1, _, _) = reference_plane_view(reference1, plane, offset)?;

    let (plane_x, plane_y, block_w, block_h) = rect.plane_rect(plane, sub_x, sub_y);
    let reference_size0 = reference0.info().coded_luma_size();
    let reference_size1 = reference1.info().coded_luma_size();
    let frame_size = sink.info().coded_luma_size();

    let scaling0 = derive_plane_scaling(
        plane_x as i32,
        plane_y as i32,
        mv0.row,
        mv0.col,
        sub_x,
        sub_y,
        reference_size0.width() as i32,
        reference_size0.height() as i32,
        frame_size.width() as i32,
        frame_size.height() as i32,
    );
    let scaling1 = derive_plane_scaling(
        plane_x as i32,
        plane_y as i32,
        mv1.row,
        mv1.col,
        sub_x,
        sub_y,
        reference_size1.width() as i32,
        reference_size1.height() as i32,
        frame_size.width() as i32,
        frame_size.height() as i32,
    );

    let params0 = SubpelPredictParams {
        interp,
        w: block_w,
        h: block_h,
        start_x: scaling0.start_x,
        start_y: scaling0.start_y,
        step_x: scaling0.step_x,
        step_y: scaling0.step_y,
        first_x: scaling0.first_x,
        first_y: scaling0.first_y,
        last_x: scaling0.last_x,
        last_y: scaling0.last_y,
        bit_depth: sink.info().bit_depth(),
    };
    let params1 = SubpelPredictParams {
        interp,
        w: block_w,
        h: block_h,
        start_x: scaling1.start_x,
        start_y: scaling1.start_y,
        step_x: scaling1.step_x,
        step_y: scaling1.step_y,
        first_x: scaling1.first_x,
        first_y: scaling1.first_y,
        last_x: scaling1.last_x,
        last_y: scaling1.last_y,
        bit_depth: sink.info().bit_depth(),
    };
    let sample_count = block_w
        .checked_mul(block_h)
        .ok_or(ReconError::ArithmeticOverflow {
            context: "compound prediction sample count",
        })?;
    let [pred0, pred1] = take_compound_prediction_buffers(sample_count);
    let mut prediction = CompoundPlanePrediction {
        pred0,
        pred1,
        plane_x,
        plane_y,
        block_w,
        block_h,
        scaling0,
        scaling1,
        recycle_buffers: true,
    };
    subpel_predict_block_compound_intermediate_into(
        &view0,
        &params0,
        &mut prediction.pred0,
        block_w,
    )?;
    subpel_predict_block_compound_intermediate_into(
        &view1,
        &params1,
        &mut prediction.pred1,
        block_w,
    )?;
    Ok(prediction)
}

/// AV2 § 7.13.3.14 compound LOCALWARP plane prediction: warp each list with its
/// own § 7.13.3.23 model (§ 7.13.3.19 block warp, § 7.13.3.20 extended warp for
/// invalid shear / sub-8x8, or translational when the list has no samples), then
/// feed the two § 7.13.3.16 `Preds[refList]` intermediates to the compound blend.
fn compound_warp_plane_prediction<T: ReconSample>(
    sink: &WorkspaceSink<'_, T>,
    block: CompoundMcBlock<'_, T>,
    plane: PlaneId,
    sub_x: u32,
    sub_y: u32,
    offset: ByteOffset,
) -> Result<CompoundPlanePrediction> {
    let (plane_x, plane_y, block_w, block_h) = block.rect.plane_rect(plane, sub_x, sub_y);
    let pred0 = compound_ref_intermediate(
        sink,
        block.reference0,
        plane,
        block.rect,
        block.warp_params[0],
        block.mv0,
        block.interp,
        sub_x,
        sub_y,
        offset,
    )?;
    let pred1 = compound_ref_intermediate(
        sink,
        block.reference1,
        plane,
        block.rect,
        block.warp_params[1],
        block.mv1,
        block.interp,
        sub_x,
        sub_y,
        offset,
    )?;
    Ok(CompoundPlanePrediction {
        pred0: pred0.samples,
        pred1: pred1.samples,
        plane_x,
        plane_y,
        block_w,
        block_h,
        scaling0: pred0.scaling,
        scaling1: pred1.scaling,
        recycle_buffers: false,
    })
}

struct CompoundRefIntermediate {
    samples: Vec<i32>,
    scaling: PlaneScaling,
}

#[allow(clippy::too_many_arguments)]
fn compound_ref_intermediate<T: ReconSample>(
    sink: &WorkspaceSink<'_, T>,
    reference: &DecodedFrame<T>,
    plane: PlaneId,
    rect: McBlockRect,
    warp_params: Option<[i32; 6]>,
    mv: Mv,
    interp: InterpolationFilter,
    sub_x: u32,
    sub_y: u32,
    offset: ByteOffset,
) -> Result<CompoundRefIntermediate> {
    let (view, ref_mi_cols, ref_mi_rows) = reference_plane_view(reference, plane, offset)?;
    let (plane_x, plane_y, block_w, block_h) = rect.plane_rect(plane, sub_x, sub_y);
    let bit_depth = sink.info().bit_depth();
    let reference_size = reference.info().coded_luma_size();
    let frame_size = sink.info().coded_luma_size();
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
    let Some(warp_params) = warp_params else {
        let params = SubpelPredictParams {
            interp,
            w: block_w,
            h: block_h,
            start_x: scaling.start_x,
            start_y: scaling.start_y,
            step_x: scaling.step_x,
            step_y: scaling.step_y,
            first_x: scaling.first_x,
            first_y: scaling.first_y,
            last_x: scaling.last_x,
            last_y: scaling.last_y,
            bit_depth,
        };
        return Ok(CompoundRefIntermediate {
            samples: subpel_predict_block_compound_intermediate(&view, &params)?,
            scaling,
        });
    };
    let (ref_width, ref_height) = (view.width(), view.height());
    let mut samples = vec![0i32; block_w.saturating_mul(block_h)];
    let skip_pred = !splot_recon::warp_shear_is_valid(warp_params)
        || block_w < WARPED_BLOCK_SIZE
        || block_h < WARPED_BLOCK_SIZE
        || scaling.is_scaled();
    if skip_pred {
        for i4 in 0..block_h.div_euclid(4) {
            for j4 in 0..block_w.div_euclid(4) {
                let (first_x, first_y, last_x, last_y) = ext_warp_unit_bounds(
                    rect,
                    plane,
                    warp_params,
                    (plane_x + (j4 & !1) * 4) as i32,
                    (plane_y + (i4 & !1) * 4) as i32,
                    block_w.min(8) as i32,
                    block_h.min(8) as i32,
                    sub_x,
                    sub_y,
                    ref_mi_cols,
                    ref_mi_rows,
                    scaling,
                );
                let params = WarpPredictBlockParams {
                    warp_params,
                    block_x: plane_x as i32,
                    block_y: plane_y as i32,
                    subsampling_x: sub_x as u8,
                    subsampling_y: sub_y as u8,
                    reference_scale_x: scaling.scale_x,
                    reference_scale_y: scaling.scale_y,
                    first_x,
                    first_y,
                    last_x,
                    last_y,
                    bit_depth,
                };
                let predicted = ext_warp_predict_unit(&view, &params, i4, j4, true)?;
                write_compound_section(&mut samples, block_w, j4 * 4, i4 * 4, &predicted, 4, 4, 4);
            }
        }
    } else {
        for local_y in (0..block_h).step_by(WARPED_BLOCK_SIZE) {
            for local_x in (0..block_w).step_by(WARPED_BLOCK_SIZE) {
                let params = WarpPredictBlockParams {
                    warp_params,
                    block_x: (plane_x + local_x) as i32,
                    block_y: (plane_y + local_y) as i32,
                    subsampling_x: sub_x as u8,
                    subsampling_y: sub_y as u8,
                    reference_scale_x: scaling.scale_x,
                    reference_scale_y: scaling.scale_y,
                    first_x: 0,
                    first_y: 0,
                    last_x: ref_width as i32 - 1,
                    last_y: ref_height as i32 - 1,
                    bit_depth,
                };
                let mut predicted = [0i32; WARPED_BLOCK_SIZE * WARPED_BLOCK_SIZE];
                warp_predict_block_into(&view, &params, true, &mut predicted)?;
                let write_w = (block_w - local_x).min(WARPED_BLOCK_SIZE);
                let write_h = (block_h - local_y).min(WARPED_BLOCK_SIZE);
                write_compound_section(
                    &mut samples,
                    block_w,
                    local_x,
                    local_y,
                    &predicted,
                    WARPED_BLOCK_SIZE,
                    write_w,
                    write_h,
                );
            }
        }
    }
    Ok(CompoundRefIntermediate { samples, scaling })
}

#[allow(clippy::too_many_arguments)]
fn write_compound_section(
    dst: &mut [i32],
    dst_w: usize,
    x: usize,
    y: usize,
    src: &[i32],
    src_stride: usize,
    w: usize,
    h: usize,
) {
    for row in 0..h {
        for col in 0..w {
            if let (Some(&value), Some(slot)) = (
                src.get(row * src_stride + col),
                dst.get_mut((y + row) * dst_w + (x + col)),
            ) {
                *slot = value;
            }
        }
    }
}

fn blend_compound_average_weighted_samples<T: ReconSample>(
    pred0: &[i32],
    pred1: &[i32],
    bit_depth: splot_recon::BitDepth,
    cwp_weight: i16,
    output: &mut [T],
) -> splot_recon::Result<()> {
    for (slot, (&left, &right)) in output.iter_mut().zip(pred0.iter().zip(pred1)) {
        *slot = T::try_from_u16(blend_compound_average_weighted_sample(
            left, right, bit_depth, cwp_weight,
        ))?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn blend_compound_average<T: ReconSample>(
    pred0: &[i32],
    pred1: &[i32],
    bit_depth: splot_recon::BitDepth,
    w: usize,
    h: usize,
    blend: CompoundBlend,
    luma_w: usize,
    luma_h: usize,
    motion: Option<&CompoundMotionGrid>,
    plane_x: usize,
    plane_y: usize,
    scaling0: PlaneScaling,
    scaling1: PlaneScaling,
    frame_w: usize,
    frame_h: usize,
    luma_diff_weighted_mask: Option<&[u16]>,
    sub_x: u32,
    sub_y: u32,
    output: &mut [T],
) -> splot_recon::Result<()> {
    let sample_count = w.checked_mul(h).ok_or(ReconError::ArithmeticOverflow {
        context: "compound blend sample count",
    })?;
    if output.len() != sample_count {
        return Err(ReconError::BufferLengthMismatch {
            expected: sample_count,
            actual: output.len(),
        });
    }
    if pred0.len() != pred1.len() {
        return Err(ReconError::CompoundBlendLengthMismatch {
            left_len: pred0.len(),
            right_len: pred1.len(),
        });
    }
    if pred0.len() != sample_count {
        return Err(ReconError::BufferLengthMismatch {
            expected: sample_count,
            actual: pred0.len(),
        });
    }
    let CompoundBlend::Average {
        implicit_mask,
        cwp_weight,
    } = blend
    else {
        return blend_compound_diff_weighted::<T>(
            pred0,
            pred1,
            bit_depth,
            w,
            h,
            blend,
            luma_w,
            luma_h,
            luma_diff_weighted_mask,
            sub_x,
            sub_y,
            output,
        );
    };
    if !implicit_mask {
        return blend_compound_average_weighted_samples(
            pred0, pred1, bit_depth, cwp_weight, output,
        );
    }
    if cwp_weight != CWP_EQUAL {
        return blend_compound_average_weighted_samples(
            pred0, pred1, bit_depth, cwp_weight, output,
        );
    }
    if scaling0.is_scaled() || scaling1.is_scaled() {
        return blend_compound_average_weighted_samples(
            pred0, pred1, bit_depth, cwp_weight, output,
        );
    }

    let last_x = frame_w as i32 - 1;
    let last_y = frame_h as i32 - 1;
    let ref_start_x0 = scaling0.start_x >> 10;
    let ref_start_y0 = scaling0.start_y >> 10;
    let ref_start_x1 = scaling1.start_x >> 10;
    let ref_start_y1 = scaling1.start_y >> 10;
    let scaling_templates = [scaling0, scaling1];
    let extent = |scaling: PlaneScaling| {
        let end_x = scaling.start_x + scaling.step_x * w.saturating_sub(1) as i32;
        let end_y = scaling.start_y + scaling.step_y * h.saturating_sub(1) as i32;
        (
            scaling.start_x >> 10,
            scaling.start_y >> 10,
            end_x >> 10,
            end_y >> 10,
        )
    };
    let uniform_scaling = match motion {
        None => Some([scaling0, scaling1]),
        Some(motion) => motion.uniform_mvs().map(|mvs| {
            core::array::from_fn(|reference| {
                scaling_templates[reference].with_prescaled_mv(
                    plane_x as i32,
                    plane_y as i32,
                    mvs[reference][0],
                    mvs[reference][1],
                    sub_x,
                    sub_y,
                )
            })
        }),
    };
    let onscreen = |extent: (i32, i32, i32, i32)| {
        extent.0 >= 0 && extent.1 >= 0 && extent.2 <= last_x && extent.3 <= last_y
    };
    if uniform_scaling.is_some_and(|scaling| scaling.into_iter().map(extent).all(onscreen)) {
        return blend_compound_average_weighted_samples(pred0, pred1, bit_depth, CWP_EQUAL, output);
    }
    let max_sample = i32::from(bit_depth.max_sample());
    let shift = 1 + compound_inter_post_round();
    for (idx, (slot, (&left, &right))) in output.iter_mut().zip(pred0.iter().zip(pred1)).enumerate()
    {
        let row = idx / w;
        let col = idx % w;
        let starts = if let Some(motion) = motion {
            let mvs = motion.at_luma_offset(col << sub_x, row << sub_y)?;
            core::array::from_fn(|reference| {
                let scaling = scaling_templates[reference].with_prescaled_mv(
                    (plane_x + col) as i32,
                    (plane_y + row) as i32,
                    mvs[reference][0],
                    mvs[reference][1],
                    sub_x,
                    sub_y,
                );
                (scaling.start_x >> 10, scaling.start_y >> 10)
            })
        } else {
            [
                (ref_start_x0 + col as i32, ref_start_y0 + row as i32),
                (ref_start_x1 + col as i32, ref_start_y1 + row as i32),
            ]
        };
        let ref0_onscreen =
            (0..=last_x).contains(&starts[0].0) && (0..=last_y).contains(&starts[0].1);
        let ref1_onscreen =
            (0..=last_x).contains(&starts[1].0) && (0..=last_y).contains(&starts[1].1);
        let mask = match (ref0_onscreen, ref1_onscreen) {
            (true, false) => 2,
            (false, true) => 0,
            _ => 1,
        };
        let sample = round2_i32(mask * left + (2 - mask) * right, shift);
        *slot = T::try_from_u16(sample.clamp(0, max_sample) as u16)?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn blend_compound_diff_weighted<T: ReconSample>(
    pred0: &[i32],
    pred1: &[i32],
    bit_depth: splot_recon::BitDepth,
    w: usize,
    h: usize,
    blend: CompoundBlend,
    luma_w: usize,
    luma_h: usize,
    luma_diff_weighted_mask: Option<&[u16]>,
    sub_x: u32,
    sub_y: u32,
    output: &mut [T],
) -> splot_recon::Result<()> {
    if let CompoundBlend::Wedge { index, sign } = blend {
        return blend_compound_wedge::<T>(
            pred0, pred1, bit_depth, w, h, luma_w, luma_h, index, sign, sub_x, sub_y, output,
        );
    }
    let CompoundBlend::DiffWeighted { inverse } = blend else {
        return blend_compound_average_weighted_samples(pred0, pred1, bit_depth, CWP_EQUAL, output);
    };
    let max_sample = i32::from(bit_depth.max_sample());
    let blend_shift = 6 + compound_inter_post_round();
    let mask = if let Some(luma_mask) = luma_diff_weighted_mask {
        subsample_diff_weighted_luma_mask(luma_mask, w, h, sub_x, sub_y)?
    } else {
        diff_weighted_mask(pred0, pred1, bit_depth, w, h, inverse)?
    };
    if mask.len() < output.len() {
        return Err(ReconError::BufferLengthMismatch {
            expected: output.len(),
            actual: mask.len(),
        });
    }
    for (slot, ((&left, &right), &mask)) in
        output.iter_mut().zip(pred0.iter().zip(pred1).zip(&mask))
    {
        let blended = round2_i32(
            i32::from(mask) * left + i32::from(64 - mask) * right,
            blend_shift,
        );
        *slot = T::try_from_u16(blended.clamp(0, max_sample) as u16)?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn blend_compound_wedge<T: ReconSample>(
    pred0: &[i32],
    pred1: &[i32],
    bit_depth: splot_recon::BitDepth,
    w: usize,
    h: usize,
    luma_w: usize,
    luma_h: usize,
    wedge_index: u8,
    sign: bool,
    sub_x: u32,
    sub_y: u32,
    output: &mut [T],
) -> splot_recon::Result<()> {
    let max_sample = i32::from(bit_depth.max_sample());
    let shift = 6 + compound_inter_post_round();
    for y in 0..h {
        for x in 0..w {
            let idx = y * w + x;
            let mask = wedge_mask_plane_sample(
                luma_w,
                luma_h,
                usize::from(wedge_index),
                sign,
                sub_x,
                sub_y,
                x,
                y,
            )?;
            let blended = round2_i32(
                i32::from(mask) * pred0[idx] + i32::from(64 - mask) * pred1[idx],
                shift,
            );
            output[idx] = T::try_from_u16(blended.clamp(0, max_sample) as u16)?;
        }
    }
    Ok(())
}

fn diff_weighted_mask(
    pred0: &[i32],
    pred1: &[i32],
    bit_depth: splot_recon::BitDepth,
    w: usize,
    h: usize,
    inverse: bool,
) -> splot_recon::Result<Vec<u16>> {
    if pred0.len() != pred1.len() {
        return Err(ReconError::CompoundBlendLengthMismatch {
            left_len: pred0.len(),
            right_len: pred1.len(),
        });
    }
    let diff_round = u32::from(bit_depth.bits().saturating_sub(8)) + compound_inter_post_round();
    let mut mask = Vec::with_capacity(w.saturating_mul(h));
    for (&left, &right) in pred0.iter().zip(pred1.iter()).take(w.saturating_mul(h)) {
        let diff = round2_i32(
            i32::try_from(left.abs_diff(right)).unwrap_or(i32::MAX),
            diff_round,
        );
        let base_mask = u16::try_from((38 + diff / 16).clamp(0, 64)).map_err(|_| {
            ReconError::ArithmeticOverflow {
                context: "diff-weighted compound mask",
            }
        })?;
        mask.push(if inverse { 64 - base_mask } else { base_mask });
    }
    Ok(mask)
}

fn subsample_diff_weighted_luma_mask(
    luma_mask: &[u16],
    w: usize,
    h: usize,
    sub_x: u32,
    sub_y: u32,
) -> splot_recon::Result<Vec<u16>> {
    let scale_x = 1usize
        .checked_shl(sub_x)
        .ok_or(ReconError::ArithmeticOverflow {
            context: "diff-weighted luma mask horizontal subsampling",
        })?;
    let scale_y = 1usize
        .checked_shl(sub_y)
        .ok_or(ReconError::ArithmeticOverflow {
            context: "diff-weighted luma mask vertical subsampling",
        })?;
    let luma_w = w
        .checked_mul(scale_x)
        .ok_or(ReconError::ArithmeticOverflow {
            context: "diff-weighted luma mask width",
        })?;
    let luma_h = h
        .checked_mul(scale_y)
        .ok_or(ReconError::ArithmeticOverflow {
            context: "diff-weighted luma mask height",
        })?;
    let expected = luma_w
        .checked_mul(luma_h)
        .ok_or(ReconError::ArithmeticOverflow {
            context: "diff-weighted luma mask sample count",
        })?;
    if luma_mask.len() < expected {
        return Err(ReconError::BufferLengthMismatch {
            expected,
            actual: luma_mask.len(),
        });
    }

    let mut out = Vec::with_capacity(w.saturating_mul(h));
    for y in 0..h {
        for x in 0..w {
            let mut sum = 0i32;
            for dy in 0..scale_y {
                for dx in 0..scale_x {
                    let mask_x = x * scale_x + dx;
                    let mask_y = y * scale_y + dy;
                    sum += i32::from(luma_mask[mask_y * luma_w + mask_x]);
                }
            }
            let averaged = round2_i32(sum, sub_x + sub_y);
            out.push(
                u16::try_from(averaged).map_err(|_| ReconError::ArithmeticOverflow {
                    context: "diff-weighted chroma mask average",
                })?,
            );
        }
    }
    Ok(out)
}

const fn compound_inter_post_round() -> u32 {
    4
}

#[allow(clippy::too_many_arguments)]
fn ext_warp_unit_bounds(
    rect: McBlockRect,
    plane: PlaneId,
    warp_params: [i32; 6],
    unit_x: i32,
    unit_y: i32,
    bbox_w: i32,
    bbox_h: i32,
    sub_x: u32,
    sub_y: u32,
    ref_mi_cols: i32,
    ref_mi_rows: i32,
    scaling: PlaneScaling,
) -> (i32, i32, i32, i32) {
    const WARPEDMODEL_PREC_BITS: u32 = 16;
    const MV_BOUND: i64 = 1 << 16;
    const MV_BORDER: i32 = 128;
    let src_x = (unit_x + (bbox_w >> 1)) << sub_x;
    let src_y = (unit_y + (bbox_h >> 1)) << sub_y;
    let dst_x = i64::from(warp_params[2]) * i64::from(src_x)
        + i64::from(warp_params[3]) * i64::from(src_y)
        + i64::from(warp_params[0]);
    let dst_y = i64::from(warp_params[4]) * i64::from(src_x)
        + i64::from(warp_params[5]) * i64::from(src_y)
        + i64::from(warp_params[1]);
    let mv_row = clip3(
        -MV_BOUND + 1,
        MV_BOUND - 1,
        (dst_y - (i64::from(src_y) << WARPEDMODEL_PREC_BITS)) >> (WARPEDMODEL_PREC_BITS - 3),
    ) as i32;
    let mv_col = clip3(
        -MV_BOUND + 1,
        MV_BOUND - 1,
        (dst_x - (i64::from(src_x) << WARPEDMODEL_PREC_BITS)) >> (WARPEDMODEL_PREC_BITS - 3),
    ) as i32;
    let (luma_x, luma_y, luma_w, luma_h) = rect.plane_luma_rect(plane);
    let mi_row = (luma_y / 4) as i32;
    let mi_col = (luma_x / 4) as i32;
    let bh4 = (luma_h / 4) as i32;
    let bw4 = (luma_w / 4) as i32;
    let mv_row = mv_row.clamp(
        -(mi_row + bh4) * 32 - MV_BORDER,
        (ref_mi_rows - mi_row) * 32 + MV_BORDER,
    );
    let mv_col = mv_col.clamp(
        -(mi_col + bw4) * 32 - MV_BORDER,
        (ref_mi_cols - mi_col) * 32 + MV_BORDER,
    );
    let scaling = scaling.with_mv(unit_x, unit_y, mv_row, mv_col, sub_x, sub_y);
    let first_x = ((scaling.start_x >> 10) - 3).clamp(0, scaling.last_x);
    let first_y = ((scaling.start_y >> 10) - 3).clamp(0, scaling.last_y);
    let end_x = scaling.start_x + scaling.step_x * (bbox_w - 1);
    let end_y = scaling.start_y + scaling.step_y * (bbox_h - 1);
    let last_x = ((end_x >> 10) + 4).clamp(0, scaling.last_x);
    let last_y = ((end_y >> 10) + 4).clamp(0, scaling.last_y);
    (first_x, first_y, last_x, last_y)
}

pub(crate) fn intrabc_predict_fractional_luma_into<T: ReconSample>(
    workspace: &mut CurrentFrameWorkspace<T>,
    target: PlaneRect,
    scaling: super::mv_scaling::PlaneScaling,
) -> Result<()> {
    intrabc_predict_subpel_plane_into(workspace, PlaneId::Y, target, scaling)
}

pub(crate) fn intrabc_predict_subpel_plane_into<T: ReconSample>(
    workspace: &mut CurrentFrameWorkspace<T>,
    plane: PlaneId,
    target: PlaneRect,
    scaling: super::mv_scaling::PlaneScaling,
) -> Result<()> {
    let storage = workspace.plane(plane)?.storage_size();
    let full = PlaneRect::new(0, 0, storage.width(), storage.height())?;
    let mut samples: Vec<T> = Vec::with_capacity(storage.width().saturating_mul(storage.height()));
    for row in workspace.rect_rows(plane, full)? {
        samples.extend_from_slice(row);
    }
    let view = ReferencePlaneView::new(&samples, storage.width(), storage.height())?;
    let params = crate::filters::wienerns_lr::recon::full_recon::intrabc_bilinear_params(
        scaling,
        target.width(),
        target.height(),
        workspace.info().bit_depth(),
    );
    let predicted = subpel_predict_block(&view, &params)?;
    let packed = pack_samples(&predicted)?;
    workspace.write_rect(plane, target, &packed, target.width())?;
    Ok(())
}

pub(crate) fn reference_plane_view<T: ReconSample>(
    reference: &DecodedFrame<T>,
    plane: PlaneId,
    offset: ByteOffset,
) -> Result<(ReferencePlaneView<'_, T>, i32, i32)> {
    let Some(ref_plane) = reference.plane(plane) else {
        return Err(unsupported_at(
            "inter_reference_missing_plane",
            offset,
            "minimal inter motion compensation requires the reference frame to carry every plane",
            SPEC_MC,
        ));
    };
    let visible = ref_plane.visible_rect();
    let stride = ref_plane.stride_samples();
    let origin = visible
        .y()
        .checked_mul(stride)
        .and_then(|row| row.checked_add(visible.x()));
    let view = origin
        .and_then(|start| ref_plane.samples().get(start..))
        .ok_or(())
        .and_then(|samples| {
            ReferencePlaneView::from_strided(samples, stride, visible.width(), visible.height())
                .map_err(|_| ())
        })
        .map_err(|()| {
            unsupported_at(
                "inter_reference_plane_geometry",
                offset,
                "minimal inter motion compensation requires a reference plane whose storage covers its visible rectangle",
                SPEC_MC,
            )
        })?;

    let luma_visible = reference.y().visible_size();
    let ref_mi_cols = luma_visible.width().div_ceil(4) as i32;
    let ref_mi_rows = luma_visible.height().div_ceil(4) as i32;

    Ok((view, ref_mi_cols, ref_mi_rows))
}

#[cfg(test)]
#[path = "mc_tests.rs"]
mod tests;
