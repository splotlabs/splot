// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#[cfg(test)]
use splot_recon::BitDepth;
use splot_recon::{
    CurrentFrameSurface, CurrentFrameWorkspace, DecodedFrameInfo, InterpolationFilter,
    OptflowScratch, PixelFormat, PlaneId, PlaneRect, PreparedWarpPrediction, ReconError,
    ReconSample, ReferencePlaneView, SubpelPredictParams, WARPED_BLOCK_SIZE,
    WarpPredictBlockParams, blend_compound_average_weighted_sample, ext_warp_predict_unit,
    subpel_predict_16x16_bilinear_horizontal_overlap_into,
    subpel_predict_block_compound_average_fast_validated_strided_into,
    subpel_predict_block_compound_average_strided_into,
    subpel_predict_block_compound_average_strided_into_u8,
    subpel_predict_block_compound_intermediate_into, subpel_predict_block_into,
    subpel_predict_block_strided_into, subpel_predict_block_strided_into_u8,
    wedge_mask_plane_sample,
};

use super::Mv;
use super::mv_scaling::{PlaneScaling, derive_plane_scaling};
use super::reference::{
    ReferenceSamples, compound_last_row, subpel_last_reference_row, warp_plane_last_row,
};
use crate::Result;
use splot_core::span::ByteOffset;
use splot_recon::math::{clip3, round2_i32};

mod compound_average;
mod optflow;
mod refinemv;
pub(crate) mod sink;
pub(crate) use optflow::CompoundMotionGrid;
use optflow::{CompoundAverageOutput, MotionCell};
pub(crate) use sink::WorkspaceSink;

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
const MAX_MC_BLOCK_DIM: usize = 128;
const MAX_MC_BLOCK_SAMPLES: usize = MAX_MC_BLOCK_DIM * MAX_MC_BLOCK_DIM;
const MAX_OPTICAL_FLOW_CELLS: usize = (MAX_MC_BLOCK_DIM / 4) * (MAX_MC_BLOCK_DIM / 4);
const MAX_REFINEMV_PREDICTION_DIM: usize = MAX_MC_BLOCK_DIM + 8;

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

    pub(in crate::prediction::inter) fn plane_rect(
        self,
        plane: PlaneId,
        sub_x: u32,
        sub_y: u32,
    ) -> (usize, usize, usize, usize) {
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
        reference: ReferenceSamples<'a, T>,
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
        reference0: ReferenceSamples<'a, T>,
        reference1: ReferenceSamples<'a, T>,
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
        reference: ReferenceSamples<'a, T>,
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
        reference: ReferenceSamples<'a, T>,
        mv: Mv,
    },
    SingleWarp {
        reference: ReferenceSamples<'a, T>,
        mv: Mv,
        warp_params: [i32; 6],
    },
    CompoundAverage {
        reference0: ReferenceSamples<'a, T>,
        reference1: ReferenceSamples<'a, T>,
        mv0: Mv,
        mv1: Mv,
        blend: CompoundBlend,
        optflow_distances: Option<[i32; 2]>,
        warp_params: [Option<[i32; 6]>; 2],
    },
}
#[derive(Clone, Copy, Debug)]
pub(crate) struct CompoundMcBlock<'a, T: ReconSample> {
    reference0: ReferenceSamples<'a, T>,
    reference1: ReferenceSamples<'a, T>,
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
    /// The block's luma rectangle, which publication scatters into.
    pub(super) const fn luma_rect(&self) -> (usize, usize, usize, usize) {
        (
            self.rect.luma_x,
            self.rect.luma_y,
            self.rect.luma_w,
            self.rect.luma_h,
        )
    }

    pub(super) fn stored_mvs_at_origin(&self) -> splot_recon::Result<Option<[Mv; 2]>> {
        self.motion
            .as_ref()
            .map(|motion| motion.stored_mvs_at_luma_offset(0, 0))
            .transpose()
    }

    pub(super) fn publish<T: ReconSample>(
        &self,
        samples: &[T],
        sink: &mut WorkspaceSink<'_, '_, T>,
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

struct RecycledMcSamples<T: Send + 'static>(Vec<T>);

impl<T: Send + 'static> RecycledMcSamples<T> {
    fn take() -> Self {
        Self(take_mc_samples())
    }
}

impl<T: Send + 'static> std::ops::Deref for RecycledMcSamples<T> {
    type Target = Vec<T>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T: Send + 'static> std::ops::DerefMut for RecycledMcSamples<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl<T: Send + 'static> Drop for RecycledMcSamples<T> {
    fn drop(&mut self) {
        recycle_mc_samples(&mut self.0);
    }
}

pub(crate) struct CompoundBlockOutput<T: Send + 'static> {
    metadata: CompoundBlockMetadata,
    samples: RecycledMcSamples<T>,
}

impl<T: ReconSample> CompoundBlockOutput<T> {
    pub(crate) fn publish(
        mut self,
        sink: &mut WorkspaceSink<'_, '_, T>,
    ) -> Result<Option<CompoundMotionGrid>> {
        self.metadata.publish(&self.samples, sink)?;
        Ok(self.metadata.motion.take())
    }
}
pub(crate) fn motion_compensate_inter_block_into<T: ReconSample>(
    sink: &mut WorkspaceSink<'_, '_, T>,
    block: InterBlockParams<'_, T>,
    offset: ByteOffset,
) -> Result<()> {
    motion_compensate_inter_block(sink, block, None, offset).map(drop)
}

pub(crate) fn inter_block_motion_grid<T: ReconSample>(
    sink: &WorkspaceSink<'_, '_, T>,
    block: InterBlockParams<'_, T>,
    optflow_unit_size: Option<usize>,
    offset: ByteOffset,
) -> Result<Option<CompoundMotionGrid>> {
    match block.into_compound() {
        Some(compound) => compound_block_motion_grid(sink, compound, optflow_unit_size, offset),
        None => Ok(None),
    }
}

pub(crate) fn predict_inter_block_from_grid<T: ReconSample>(
    sink: &mut WorkspaceSink<'_, '_, T>,
    block: InterBlockParams<'_, T>,
    motion: Option<CompoundMotionGrid>,
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
            Ok(motion)
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
            Ok(motion)
        }
        InterPrediction::CompoundAverage { .. } => match block.into_compound() {
            Some(compound)
                if motion.is_none()
                    && compound_average::predict_translational_direct(sink, compound, offset)? =>
            {
                Ok(None)
            }
            Some(compound) => {
                predict_compound_average_block(sink, compound, motion, offset)?.publish(sink)
            }
            None => Ok(motion),
        },
    }
}

fn motion_compensate_inter_block<T: ReconSample>(
    sink: &mut WorkspaceSink<'_, '_, T>,
    block: InterBlockParams<'_, T>,
    optflow_unit_size: Option<usize>,
    offset: ByteOffset,
) -> Result<Option<CompoundMotionGrid>> {
    let motion = inter_block_motion_grid(sink, block, optflow_unit_size, offset)?;
    predict_inter_block_from_grid(sink, block, motion, offset)
}

fn motion_compensate_single_block_into<T: ReconSample>(
    sink: &mut WorkspaceSink<'_, '_, T>,
    reference: ReferenceSamples<'_, T>,
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
    sink: &mut WorkspaceSink<'_, '_, T>,
    has_chroma: bool,
    mut predict: impl FnMut(&mut WorkspaceSink<'_, '_, T>, PlaneId, u32, u32) -> Result<()>,
) -> Result<()> {
    for (plane, sub_x, sub_y) in mc_planes(sink.info().pixel_format()) {
        if plane != PlaneId::Y && !has_chroma {
            continue;
        }
        predict(sink, plane, sub_x, sub_y)?;
    }
    Ok(())
}

pub(super) fn compound_block_motion_grid<T: ReconSample>(
    sink: &WorkspaceSink<'_, '_, T>,
    block: CompoundMcBlock<'_, T>,
    optflow_unit_size: Option<usize>,
    offset: ByteOffset,
) -> Result<Option<CompoundMotionGrid>> {
    let refinemv = block
        .use_refinemv
        .then(|| refinemv::compound_default_refinemv_motion_grid(sink, block, offset))
        .transpose()?;
    optflow::compound_motion_grid(sink, block, optflow_unit_size, refinemv, offset)
}

pub(crate) fn predict_compound_average_block<T: ReconSample>(
    sink: &WorkspaceSink<'_, '_, T>,
    block: CompoundMcBlock<'_, T>,
    motion: Option<CompoundMotionGrid>,
    offset: ByteOffset,
) -> Result<CompoundBlockOutput<T>> {
    let sample_count =
        compound_output_sample_count(block.rect, block.has_chroma, sink.info().pixel_format())?;
    let mut samples = RecycledMcSamples::take();
    samples.clear();
    samples.resize(sample_count, T::default());
    let metadata = predict_compound_from_grid(sink, block, motion, offset, &mut samples)?;
    Ok(CompoundBlockOutput { metadata, samples })
}

pub(super) fn tip_batch_motion_grid<T: ReconSample>(
    sink: &WorkspaceSink<'_, '_, T>,
    block: CompoundMcBlock<'_, T>,
    columns: usize,
    unit_count: usize,
    unit_at: impl Fn(usize) -> (McBlockRect, [Mv; 2]) + Sync,
    offset: ByteOffset,
) -> Result<CompoundMotionGrid> {
    optflow::tip_motion_grid(sink, block, 8, columns, unit_count, unit_at, offset)
}

pub(super) fn predict_tip_batch_from_grid<T: ReconSample>(
    sink: &WorkspaceSink<'_, '_, T>,
    mut block: CompoundMcBlock<'_, T>,
    batch_rect: McBlockRect,
    batch_has_chroma: bool,
    motion: CompoundMotionGrid,
    offset: ByteOffset,
    samples: &mut [T],
) -> Result<CompoundBlockMetadata> {
    block.rect = batch_rect;
    block.has_chroma = batch_has_chroma;
    block.sub8x8_chroma = false;
    block.optflow_distances = None;
    block.use_refinemv = false;
    block.search_refinemv = false;
    predict_compound_from_grid(sink, block, Some(motion), offset, samples)
}

fn compound_output_samples<'a, T: ReconSample>(
    sink: &WorkspaceSink<'_, '_, T>,
    block: CompoundMcBlock<'_, T>,
    samples: &'a mut [T],
) -> Result<&'a mut [T]> {
    let sample_count =
        compound_output_sample_count(block.rect, block.has_chroma, sink.info().pixel_format())?;
    let available_samples = samples.len();
    samples.get_mut(..sample_count).ok_or(
        ReconError::BufferLengthMismatch {
            expected: sample_count,
            actual: available_samples,
        }
        .into(),
    )
}

pub(super) fn predict_compound_from_grid<T: ReconSample>(
    sink: &WorkspaceSink<'_, '_, T>,
    block: CompoundMcBlock<'_, T>,
    motion: Option<CompoundMotionGrid>,
    offset: ByteOffset,
    samples: &mut [T],
) -> Result<CompoundBlockMetadata> {
    let mut samples = compound_output_samples(sink, block, samples)?;
    let luma_diff_weighted_mask =
        compound_luma_diff_weighted_mask(sink, block, motion.as_ref(), offset)?;
    for (plane, sub_x, sub_y) in mc_planes(sink.info().pixel_format()) {
        if plane != PlaneId::Y && !block.has_chroma {
            continue;
        }
        let plane_sample_count = compound_plane_sample_count(block.rect, plane, sub_x, sub_y)?;
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
                luma_diff_weighted_mask.as_ref().map(|mask| mask.as_slice()),
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

/// The samples one plane of a compound block's packed output holds.
fn compound_plane_sample_count(
    rect: McBlockRect,
    plane: PlaneId,
    sub_x: u32,
    sub_y: u32,
) -> Result<usize> {
    let (_, _, block_w, block_h) = rect.plane_rect(plane, sub_x, sub_y);
    block_w.checked_mul(block_h).ok_or(
        ReconError::ArithmeticOverflow {
            context: "compound output plane sample count",
        }
        .into(),
    )
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
        let plane_samples = compound_plane_sample_count(rect, plane, sub_x, sub_y)?;
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
    sink: &WorkspaceSink<'_, '_, T>,
    block: CompoundMcBlock<'_, T>,
    motion: Option<&CompoundMotionGrid>,
    offset: ByteOffset,
) -> Result<Option<RecycledMcSamples<u16>>> {
    let CompoundBlend::DiffWeighted { inverse } = block.blend else {
        return Ok(None);
    };
    let prediction =
        compound_plane_prediction_for_block(sink, block, PlaneId::Y, 0, 0, motion, offset)?;
    let mut mask = RecycledMcSamples::take();
    diff_weighted_mask_into(
        &prediction.pred0,
        &prediction.pred1,
        sink.info().bit_depth(),
        prediction.block_w,
        prediction.block_h,
        inverse,
        &mut mask,
    )?;
    Ok(Some(mask))
}

fn motion_compensate_single_warp_block_into<T: ReconSample>(
    sink: &mut WorkspaceSink<'_, '_, T>,
    reference: ReferenceSamples<'_, T>,
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
    sink: &mut WorkspaceSink<'_, '_, T>,
    reference: ReferenceSamples<'_, T>,
    plane: PlaneId,
    rect: McBlockRect,
    mv: Mv,
    interp: InterpolationFilter,
    sub_x: u32,
    sub_y: u32,
    offset: ByteOffset,
) -> Result<()> {
    let (view, target, params) = plane_prediction(
        sink.info(),
        reference,
        plane,
        rect,
        mv,
        interp,
        sub_x,
        sub_y,
        offset,
    )?;
    let direct = if T::u16_slice(&[]).is_some() {
        sink.with_contiguous_u16_rect_mut(plane, target, |output, stride| {
            subpel_predict_block_strided_into(&view, &params, output, stride)
        })?
    } else if T::u8_slice(&[]).is_some() {
        sink.with_contiguous_u8_rect_mut(plane, target, |output, stride| {
            subpel_predict_block_strided_into_u8(&view, &params, output, stride)
        })?
    } else {
        None
    };
    if direct.is_some() {
        return Ok(());
    }
    let len = params
        .w
        .checked_mul(params.h)
        .ok_or(ReconError::ArithmeticOverflow {
            context: "single-reference prediction sample count",
        })?;
    SUBPEL_PREDICTION_BUFFER.with(|slot| {
        let mut predicted = slot.take().unwrap_or_default();
        predicted.resize(len, 0);
        let result: Result<()> = (|| {
            subpel_predict_block_into(&view, &params, &mut predicted)?;
            sink::write_u16_rect(sink, plane, target, &predicted, params.w)?;
            Ok(())
        })();
        slot.set(Some(predicted));
        result
    })
}

#[allow(clippy::too_many_arguments)]
fn with_plane_prediction<T: ReconSample>(
    info: DecodedFrameInfo,
    reference: ReferenceSamples<'_, T>,
    plane: PlaneId,
    rect: McBlockRect,
    mv: Mv,
    interp: InterpolationFilter,
    sub_x: u32,
    sub_y: u32,
    offset: ByteOffset,
    consume: impl FnOnce(PlaneRect, &[u16], usize) -> Result<()>,
) -> Result<()> {
    let (view, rect, params) = plane_prediction(
        info, reference, plane, rect, mv, interp, sub_x, sub_y, offset,
    )?;
    let len = params
        .w
        .checked_mul(params.h)
        .ok_or(ReconError::ArithmeticOverflow {
            context: "single-reference prediction sample count",
        })?;
    SUBPEL_PREDICTION_BUFFER.with(|slot| {
        let mut predicted = slot.take().unwrap_or_default();
        predicted.resize(len, 0);
        let result: Result<()> = (|| {
            subpel_predict_block_into(&view, &params, &mut predicted)?;
            consume(rect, &predicted, params.w)
        })();
        slot.set(Some(predicted));
        result
    })
}

#[allow(clippy::too_many_arguments)]
fn plane_prediction<T: ReconSample>(
    info: DecodedFrameInfo,
    reference: ReferenceSamples<'_, T>,
    plane: PlaneId,
    rect: McBlockRect,
    mv: Mv,
    interp: InterpolationFilter,
    sub_x: u32,
    sub_y: u32,
    offset: ByteOffset,
) -> Result<(ReferencePlaneView<'_, T>, PlaneRect, SubpelPredictParams)> {
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
    let (view, _, _) = reference.plane_view(plane, subpel_last_reference_row(&params), offset)?;
    let rect = PlaneRect::new(plane_x, plane_y, block_w, block_h)?;
    Ok((view, rect, params))
}

#[allow(clippy::too_many_arguments)]
fn predict_warp_plane<T: ReconSample>(
    sink: &mut WorkspaceSink<'_, '_, T>,
    reference: ReferenceSamples<'_, T>,
    plane: PlaneId,
    rect: McBlockRect,
    warp_params: [i32; 6],
    sub_x: u32,
    sub_y: u32,
    offset: ByteOffset,
) -> Result<()> {
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
    let last_row = warp_plane_last_row(
        warp_params,
        (plane_x, plane_y, block_w, block_h),
        sub_x,
        sub_y,
        scaling,
    );
    let (view, ref_mi_cols, ref_mi_rows) = reference.plane_view(plane, last_row, offset)?;
    let (ref_width, ref_height) = (view.width(), view.height());
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

    let params = WarpPredictBlockParams {
        warp_params,
        block_x: plane_x as i32,
        block_y: plane_y as i32,
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
    let prepared = PreparedWarpPrediction::new(&params)?;
    for local_y in (0..block_h).step_by(WARPED_BLOCK_SIZE) {
        for local_x in (0..block_w).step_by(WARPED_BLOCK_SIZE) {
            let write_x = plane_x + local_x;
            let write_y = plane_y + local_y;
            if write_x >= destination_width || write_y >= destination_height {
                continue;
            }
            let mut predicted = [0i32; WARPED_BLOCK_SIZE * WARPED_BLOCK_SIZE];
            prepared.predict_block_into(
                &view,
                write_x as i32,
                write_y as i32,
                false,
                &mut predicted,
            )?;
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
    sink: &WorkspaceSink<'_, '_, T>,
    reference0: ReferenceSamples<'_, T>,
    reference1: ReferenceSamples<'_, T>,
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
    let coded_luma_size = sink.info().coded_luma_size();
    let frame_w = (coded_luma_size.width().div_ceil(4) * 4) >> sub_x;
    let frame_h = (coded_luma_size.height().div_ceil(4) * 4) >> sub_y;
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
    let has_warp = warp_params.iter().any(Option::is_some);
    if !has_warp
        && let (
            Some(motion),
            CompoundBlend::Average {
                implicit_mask,
                cwp_weight,
            },
        ) = (motion, blend)
    {
        if let Some(output) = T::u8_slice_mut(samples)
            && predict_motion_compound_average_into(
                sink,
                block,
                plane,
                sub_x,
                sub_y,
                motion,
                implicit_mask,
                cwp_weight,
                offset,
                output,
            )?
        {
            return Ok(());
        }
        if let Some(output) = T::u16_slice_mut(samples)
            && predict_motion_compound_average_into(
                sink,
                block,
                plane,
                sub_x,
                sub_y,
                motion,
                implicit_mask,
                cwp_weight,
                offset,
                output,
            )?
        {
            return Ok(());
        }
    }
    let translation = if motion.is_none() && !has_warp {
        Some(translational_compound_plane(
            sink, block, plane, sub_x, sub_y, offset,
        )?)
    } else {
        None
    };
    if let (
        Some(translation),
        CompoundBlend::Average {
            implicit_mask,
            cwp_weight,
        },
        Some(output),
    ) = (translation.as_ref(), blend, T::u16_slice_mut(samples))
        && compound_average_weights_are_uniform(
            implicit_mask,
            cwp_weight,
            translation.plane.block_w,
            translation.plane.block_h,
            translation.plane.scalings,
            Some(translation.plane.scalings),
            (frame_w, frame_h),
        )
    {
        predict_compound_average_into(
            &translation.plane,
            &translation.params,
            cwp_weight,
            None,
            None,
            output,
            translation.plane.block_w,
        )?;
        return Ok(());
    }
    let prediction = match translation {
        Some(translation) => compound_plane_prediction_from_translation(translation)?,
        None => {
            compound_plane_prediction_for_block(sink, block, plane, sub_x, sub_y, motion, offset)?
        }
    };
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

#[allow(clippy::too_many_arguments)]
fn predict_motion_compound_average_into<T: ReconSample, O: CompoundAverageOutput + Send>(
    sink: &WorkspaceSink<'_, '_, T>,
    block: CompoundMcBlock<'_, T>,
    plane: PlaneId,
    sub_x: u32,
    sub_y: u32,
    motion: &CompoundMotionGrid,
    implicit_mask: bool,
    cwp_weight: i16,
    offset: ByteOffset,
    output: &mut [O],
) -> Result<bool> {
    if optflow::predict_uniform_motion_compound_average_into(
        sink,
        block,
        plane,
        sub_x,
        sub_y,
        motion,
        implicit_mask,
        cwp_weight,
        offset,
        output,
    )? {
        return Ok(true);
    }
    optflow::predict_motion_grid_compound_average_into(
        sink,
        block,
        plane,
        sub_x,
        sub_y,
        motion,
        implicit_mask,
        cwp_weight,
        offset,
        output,
    )
}

fn predict_compound_average_into<T: ReconSample, O: CompoundAverageOutput>(
    plane: &CompoundSubpelPlane<'_, T>,
    params: &[SubpelPredictParams; 2],
    cwp_weight: i16,
    pred0_scratch: Option<&mut [i32]>,
    intermediate_scratch: Option<&mut [i16]>,
    output: &mut [O],
    output_stride: usize,
) -> splot_recon::Result<()> {
    let sample_count =
        plane
            .block_w
            .checked_mul(plane.block_h)
            .ok_or(ReconError::ArithmeticOverflow {
                context: "compound prediction sample count",
            })?;
    match pred0_scratch.and_then(|scratch| scratch.get_mut(..sample_count)) {
        Some(pred0) => predict_compound_average_pred0_into(
            plane,
            params,
            cwp_weight,
            pred0,
            intermediate_scratch,
            output,
            output_stride,
        ),
        None => with_compound_primary_prediction(sample_count, |pred0| {
            predict_compound_average_pred0_into(
                plane,
                params,
                cwp_weight,
                pred0,
                intermediate_scratch,
                output,
                output_stride,
            )
        }),
    }
}

fn predict_compound_average_pred0_into<T: ReconSample, O: CompoundAverageOutput>(
    plane: &CompoundSubpelPlane<'_, T>,
    params: &[SubpelPredictParams; 2],
    cwp_weight: i16,
    pred0: &mut [i32],
    mut intermediate_scratch: Option<&mut [i16]>,
    output: &mut [O],
    output_stride: usize,
) -> splot_recon::Result<()> {
    subpel_predict_block_compound_intermediate_into(
        &plane.views[0],
        &params[0],
        intermediate_scratch.as_deref_mut(),
        pred0,
        plane.block_w,
    )?;
    O::predict_second(
        &plane.views[1],
        &params[1],
        pred0,
        cwp_weight,
        intermediate_scratch,
        output,
        output_stride,
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

pub(crate) struct McScratch {
    compound_predictions: [Vec<i32>; 2],
    initial_luma_predictions: Vec<u16>,
    subpel_prediction: Vec<u16>,
    samples: [Option<Box<dyn std::any::Any + Send>>; 2],
    optflow: Option<OptflowScratch>,
    motion_cells: [Option<Vec<MotionCell>>; 2],
}

impl Default for McScratch {
    fn default() -> Self {
        Self {
            compound_predictions: core::array::from_fn(|_| {
                Vec::with_capacity(MAX_MC_BLOCK_SAMPLES)
            }),
            initial_luma_predictions: Vec::with_capacity(
                2 * MAX_REFINEMV_PREDICTION_DIM * MAX_REFINEMV_PREDICTION_DIM,
            ),
            subpel_prediction: Vec::with_capacity(MAX_MC_BLOCK_SAMPLES),
            samples: [None, None],
            optflow: Some(OptflowScratch::with_capacity(
                4 * MAX_MC_BLOCK_SAMPLES,
                MAX_OPTICAL_FLOW_CELLS,
            )),
            motion_cells: core::array::from_fn(|_| {
                Some(Vec::with_capacity(MAX_OPTICAL_FLOW_CELLS))
            }),
        }
    }
}

struct McInstallGuard<'a> {
    scratch: &'a mut McScratch,
    swapped: bool,
}

impl Drop for McInstallGuard<'_> {
    fn drop(&mut self) {
        if self.swapped {
            self.scratch.swap_with_thread_locals();
        }
        MC_INSTALL_DEPTH.with(|depth| depth.set(depth.get().saturating_sub(1)));
    }
}

impl McScratch {
    pub(crate) const fn empty() -> Self {
        Self {
            compound_predictions: [Vec::new(), Vec::new()],
            initial_luma_predictions: Vec::new(),
            subpel_prediction: Vec::new(),
            samples: [None, None],
            optflow: None,
            motion_cells: [None, None],
        }
    }

    pub(crate) fn with_installed<R>(&mut self, f: impl FnOnce() -> R) -> R {
        let nested = MC_INSTALL_DEPTH.with(|depth| {
            let nested = depth.get() != 0;
            depth.set(depth.get().saturating_add(1));
            nested
        });
        let mut guard = McInstallGuard {
            scratch: self,
            swapped: false,
        };
        if !nested {
            guard.scratch.swap_with_thread_locals();
            guard.swapped = true;
        }
        f()
    }

    fn swap_with_thread_locals(&mut self) {
        COMPOUND_PREDICTION_BUFFERS.with(|slot| {
            let mut active = slot.take().unwrap_or_default();
            std::mem::swap(&mut active, &mut self.compound_predictions);
            slot.set(Some(active));
        });
        INITIAL_LUMA_PREDICTIONS.with(|slot| {
            let mut active = slot.take().unwrap_or_default();
            std::mem::swap(&mut active, &mut self.initial_luma_predictions);
            slot.set(Some(active));
        });
        SUBPEL_PREDICTION_BUFFER.with(|slot| {
            let mut active = slot.take().unwrap_or_default();
            std::mem::swap(&mut active, &mut self.subpel_prediction);
            slot.set(Some(active));
        });
        MC_SAMPLES_RECYCLER.with(|slot| {
            std::mem::swap(&mut *slot.borrow_mut(), &mut self.samples);
        });
        optflow::swap_thread_locals(&mut self.optflow, &mut self.motion_cells);
    }
}

std::thread_local! {
    static MC_INSTALL_DEPTH: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static COMPOUND_PREDICTION_BUFFERS: std::cell::Cell<Option<[Vec<i32>; 2]>> =
        const { std::cell::Cell::new(None) };
    static INITIAL_LUMA_PREDICTIONS: std::cell::Cell<Option<Vec<u16>>> =
        const { std::cell::Cell::new(None) };
    static SUBPEL_PREDICTION_BUFFER: std::cell::Cell<Option<Vec<u16>>> =
        const { std::cell::Cell::new(None) };
    static MC_SAMPLES_RECYCLER: std::cell::RefCell<[Option<Box<dyn std::any::Any + Send>>; 2]> =
        std::cell::RefCell::new([None, None]);
}

fn take_mc_samples<T: Send + 'static>() -> Vec<T> {
    MC_SAMPLES_RECYCLER.with(crate::support::reusable_scratch::take_reusable_vec)
}

fn recycle_mc_samples<T: Send + 'static>(samples: &mut Vec<T>) {
    MC_SAMPLES_RECYCLER.with(|cell| {
        crate::support::reusable_scratch::recycle_reusable_vec(cell, samples);
    });
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

fn with_compound_primary_prediction<R>(
    len: usize,
    predict: impl FnOnce(&mut [i32]) -> splot_recon::Result<R>,
) -> splot_recon::Result<R> {
    COMPOUND_PREDICTION_BUFFERS.with(|slot| {
        let mut buffers = slot.take().unwrap_or_default();
        buffers[0].resize(len, 0);
        let result = predict(&mut buffers[0]);
        slot.set(Some(buffers));
        result
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
    sink: &WorkspaceSink<'_, '_, T>,
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
    compound_plane_prediction_from_translation(translational_compound_plane(
        sink, block, plane, sub_x, sub_y, offset,
    )?)
}

struct CompoundSubpelPlane<'a, T: ReconSample> {
    views: [ReferencePlaneView<'a, T>; 2],
    plane_x: usize,
    plane_y: usize,
    block_w: usize,
    block_h: usize,
    scalings: [PlaneScaling; 2],
}

struct TranslationalCompoundPlane<'a, T: ReconSample> {
    plane: CompoundSubpelPlane<'a, T>,
    params: [SubpelPredictParams; 2],
}

fn compound_subpel_plane<'a, T: ReconSample>(
    sink: &WorkspaceSink<'_, '_, T>,
    block: CompoundMcBlock<'a, T>,
    plane: PlaneId,
    sub_x: u32,
    sub_y: u32,
    offset: ByteOffset,
) -> Result<CompoundSubpelPlane<'a, T>> {
    let (plane_x, plane_y, block_w, block_h) = block.rect.plane_rect(plane, sub_x, sub_y);
    let reference_size0 = block.reference0.info().coded_luma_size();
    let reference_size1 = block.reference1.info().coded_luma_size();
    let frame_size = sink.info().coded_luma_size();

    let scaling0 = derive_plane_scaling(
        plane_x as i32,
        plane_y as i32,
        block.mv0.row,
        block.mv0.col,
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
        block.mv1.row,
        block.mv1.col,
        sub_x,
        sub_y,
        reference_size1.width() as i32,
        reference_size1.height() as i32,
        frame_size.width() as i32,
        frame_size.height() as i32,
    );
    let last_row = |scaling: PlaneScaling| {
        compound_last_row(scaling.start_y, scaling.step_y, block_h, scaling.last_y)
    };
    let (view0, _, _) = block
        .reference0
        .plane_view(plane, last_row(scaling0), offset)?;
    let (view1, _, _) = block
        .reference1
        .plane_view(plane, last_row(scaling1), offset)?;

    Ok(CompoundSubpelPlane {
        views: [view0, view1],
        plane_x,
        plane_y,
        block_w,
        block_h,
        scalings: [scaling0, scaling1],
    })
}

fn translational_compound_plane<'a, T: ReconSample>(
    sink: &WorkspaceSink<'_, '_, T>,
    block: CompoundMcBlock<'a, T>,
    plane: PlaneId,
    sub_x: u32,
    sub_y: u32,
    offset: ByteOffset,
) -> Result<TranslationalCompoundPlane<'a, T>> {
    let plane = compound_subpel_plane(sink, block, plane, sub_x, sub_y, offset)?;
    let params = plane.scalings.map(|scaling| SubpelPredictParams {
        interp: block.interp,
        w: plane.block_w,
        h: plane.block_h,
        start_x: scaling.start_x,
        start_y: scaling.start_y,
        step_x: scaling.step_x,
        step_y: scaling.step_y,
        first_x: scaling.first_x,
        first_y: scaling.first_y,
        last_x: scaling.last_x,
        last_y: scaling.last_y,
        bit_depth: sink.info().bit_depth(),
    });
    Ok(TranslationalCompoundPlane { plane, params })
}

fn compound_plane_prediction_from_translation<T: ReconSample>(
    translation: TranslationalCompoundPlane<'_, T>,
) -> Result<CompoundPlanePrediction> {
    let TranslationalCompoundPlane { plane, params } = translation;
    let CompoundSubpelPlane {
        views,
        plane_x,
        plane_y,
        block_w,
        block_h,
        scalings,
    } = plane;
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
        scaling0: scalings[0],
        scaling1: scalings[1],
        recycle_buffers: true,
    };
    subpel_predict_block_compound_intermediate_into(
        &views[0],
        &params[0],
        None,
        &mut prediction.pred0,
        block_w,
    )?;
    subpel_predict_block_compound_intermediate_into(
        &views[1],
        &params[1],
        None,
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
    sink: &WorkspaceSink<'_, '_, T>,
    block: CompoundMcBlock<'_, T>,
    plane: PlaneId,
    sub_x: u32,
    sub_y: u32,
    offset: ByteOffset,
) -> Result<CompoundPlanePrediction> {
    let (plane_x, plane_y, block_w, block_h) = block.rect.plane_rect(plane, sub_x, sub_y);
    let sample_count = block_w
        .checked_mul(block_h)
        .ok_or(ReconError::ArithmeticOverflow {
            context: "compound prediction sample count",
        })?;
    let [mut pred0, mut pred1] = take_compound_prediction_buffers(sample_count);
    let scaling0 = compound_ref_intermediate(
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
        &mut pred0,
    )?;
    let scaling1 = compound_ref_intermediate(
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
        &mut pred1,
    )?;
    Ok(CompoundPlanePrediction {
        pred0,
        pred1,
        plane_x,
        plane_y,
        block_w,
        block_h,
        scaling0,
        scaling1,
        recycle_buffers: true,
    })
}

#[allow(clippy::too_many_arguments)]
fn compound_ref_intermediate<T: ReconSample>(
    sink: &WorkspaceSink<'_, '_, T>,
    reference: ReferenceSamples<'_, T>,
    plane: PlaneId,
    rect: McBlockRect,
    warp_params: Option<[i32; 6]>,
    mv: Mv,
    interp: InterpolationFilter,
    sub_x: u32,
    sub_y: u32,
    offset: ByteOffset,
    samples: &mut [i32],
) -> Result<PlaneScaling> {
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
    let last_row = warp_params.map_or_else(
        || compound_last_row(scaling.start_y, scaling.step_y, block_h, scaling.last_y),
        |warp_params| {
            warp_plane_last_row(
                warp_params,
                (plane_x, plane_y, block_w, block_h),
                sub_x,
                sub_y,
                scaling,
            )
        },
    );
    let (view, ref_mi_cols, ref_mi_rows) = reference.plane_view(plane, last_row, offset)?;
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
        subpel_predict_block_compound_intermediate_into(&view, &params, None, samples, block_w)?;
        return Ok(scaling);
    };
    let (ref_width, ref_height) = (view.width(), view.height());
    samples.fill(0);
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
                write_compound_section(samples, block_w, j4 * 4, i4 * 4, &predicted, 4, 4, 4);
            }
        }
    } else {
        let params = WarpPredictBlockParams {
            warp_params,
            block_x: plane_x as i32,
            block_y: plane_y as i32,
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
        let prepared = PreparedWarpPrediction::new(&params)?;
        for local_y in (0..block_h).step_by(WARPED_BLOCK_SIZE) {
            for local_x in (0..block_w).step_by(WARPED_BLOCK_SIZE) {
                let mut predicted = [0i32; WARPED_BLOCK_SIZE * WARPED_BLOCK_SIZE];
                prepared.predict_block_into(
                    &view,
                    (plane_x + local_x) as i32,
                    (plane_y + local_y) as i32,
                    true,
                    &mut predicted,
                )?;
                let write_w = (block_w - local_x).min(WARPED_BLOCK_SIZE);
                let write_h = (block_h - local_y).min(WARPED_BLOCK_SIZE);
                write_compound_section(
                    samples,
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
    Ok(scaling)
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

fn compound_uniform_scalings(
    motion: Option<&CompoundMotionGrid>,
    plane_x: usize,
    plane_y: usize,
    scalings: [PlaneScaling; 2],
    sub_x: u32,
    sub_y: u32,
) -> Option<[PlaneScaling; 2]> {
    match motion {
        None => Some(scalings),
        Some(motion) => motion.uniform_mvs().map(|mvs| {
            core::array::from_fn(|reference| {
                scalings[reference].with_prescaled_mv(
                    plane_x as i32,
                    plane_y as i32,
                    mvs[reference][0],
                    mvs[reference][1],
                    sub_x,
                    sub_y,
                )
            })
        }),
    }
}

fn compound_average_weights_are_uniform(
    implicit_mask: bool,
    cwp_weight: i16,
    w: usize,
    h: usize,
    base_scalings: [PlaneScaling; 2],
    uniform_scalings: Option<[PlaneScaling; 2]>,
    frame_size: (usize, usize),
) -> bool {
    if !implicit_mask
        || cwp_weight != CWP_EQUAL
        || base_scalings.into_iter().any(PlaneScaling::is_scaled)
    {
        return true;
    }
    let last_x = frame_size.0 as i32 - 1;
    let last_y = frame_size.1 as i32 - 1;
    uniform_scalings.is_some_and(|scaling| {
        scaling.into_iter().all(|scaling| {
            let end_x = scaling.start_x + scaling.step_x * w.saturating_sub(1) as i32;
            let end_y = scaling.start_y + scaling.step_y * h.saturating_sub(1) as i32;
            scaling.start_x >> 10 >= 0
                && scaling.start_y >> 10 >= 0
                && end_x >> 10 <= last_x
                && end_y >> 10 <= last_y
        })
    })
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
    let scaling_templates = [scaling0, scaling1];
    let uniform_scalings =
        compound_uniform_scalings(motion, plane_x, plane_y, scaling_templates, sub_x, sub_y);
    if compound_average_weights_are_uniform(
        implicit_mask,
        cwp_weight,
        w,
        h,
        scaling_templates,
        uniform_scalings,
        (frame_w, frame_h),
    ) {
        return blend_compound_average_weighted_samples(
            pred0, pred1, bit_depth, cwp_weight, output,
        );
    }

    optflow::blend_nonuniform_implicit_mask(
        pred0,
        pred1,
        bit_depth,
        w,
        h,
        motion,
        plane_x,
        plane_y,
        scaling_templates,
        frame_w,
        frame_h,
        sub_x,
        sub_y,
        output,
    )
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
    if pred0.len() != pred1.len() {
        return Err(ReconError::CompoundBlendLengthMismatch {
            left_len: pred0.len(),
            right_len: pred1.len(),
        });
    }
    let sample_count = w.checked_mul(h).ok_or(ReconError::ArithmeticOverflow {
        context: "diff-weighted compound mask sample count",
    })?;
    if pred0.len() < sample_count || output.len() > sample_count {
        return Err(ReconError::BufferLengthMismatch {
            expected: sample_count,
            actual: pred0.len().min(output.len()),
        });
    }
    let scales = luma_diff_weighted_mask
        .map(|mask| diff_weighted_luma_mask_scales(mask, w, h, sub_x, sub_y))
        .transpose()?;
    let max_sample = i32::from(bit_depth.max_sample());
    let blend_shift = 6 + compound_inter_post_round();
    let diff_round = u32::from(bit_depth.bits().saturating_sub(8)) + compound_inter_post_round();
    for (index, (slot, (&left, &right))) in
        output.iter_mut().zip(pred0.iter().zip(pred1)).enumerate()
    {
        let mask = if let (Some(luma_mask), Some((scale_x, scale_y, luma_w))) =
            (luma_diff_weighted_mask, scales)
        {
            let x = index % w;
            let y = index / w;
            let mut sum = 0i32;
            for dy in 0..scale_y {
                for dx in 0..scale_x {
                    sum += i32::from(luma_mask[(y * scale_y + dy) * luma_w + x * scale_x + dx]);
                }
            }
            u16::try_from(round2_i32(sum, sub_x + sub_y)).map_err(|_| {
                ReconError::ArithmeticOverflow {
                    context: "diff-weighted chroma mask average",
                }
            })?
        } else {
            let diff = round2_i32(
                i32::try_from(left.abs_diff(right)).unwrap_or(i32::MAX),
                diff_round,
            );
            let base = u16::try_from((38 + diff / 16).clamp(0, 64)).map_err(|_| {
                ReconError::ArithmeticOverflow {
                    context: "diff-weighted compound mask",
                }
            })?;
            if inverse { 64 - base } else { base }
        };
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

fn diff_weighted_mask_into(
    pred0: &[i32],
    pred1: &[i32],
    bit_depth: splot_recon::BitDepth,
    w: usize,
    h: usize,
    inverse: bool,
    mask: &mut Vec<u16>,
) -> splot_recon::Result<()> {
    if pred0.len() != pred1.len() {
        return Err(ReconError::CompoundBlendLengthMismatch {
            left_len: pred0.len(),
            right_len: pred1.len(),
        });
    }
    let sample_count = w.checked_mul(h).ok_or(ReconError::ArithmeticOverflow {
        context: "diff-weighted compound mask sample count",
    })?;
    if pred0.len() < sample_count {
        return Err(ReconError::BufferLengthMismatch {
            expected: sample_count,
            actual: pred0.len(),
        });
    }
    let diff_round = u32::from(bit_depth.bits().saturating_sub(8)) + compound_inter_post_round();
    mask.clear();
    mask.reserve(sample_count);
    for (&left, &right) in pred0.iter().zip(pred1).take(sample_count) {
        let diff = round2_i32(
            i32::try_from(left.abs_diff(right)).unwrap_or(i32::MAX),
            diff_round,
        );
        let base = u16::try_from((38 + diff / 16).clamp(0, 64)).map_err(|_| {
            ReconError::ArithmeticOverflow {
                context: "diff-weighted compound mask",
            }
        })?;
        mask.push(if inverse { 64 - base } else { base });
    }
    Ok(())
}

fn diff_weighted_luma_mask_scales(
    luma_mask: &[u16],
    w: usize,
    h: usize,
    sub_x: u32,
    sub_y: u32,
) -> splot_recon::Result<(usize, usize, usize)> {
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

    Ok((scale_x, scale_y, luma_w))
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

pub(crate) fn intrabc_copy_plane_into<T: ReconSample>(
    workspace: &mut CurrentFrameWorkspace<T>,
    plane: PlaneId,
    source: PlaneRect,
    target: PlaneRect,
) -> Result<()> {
    let storage = workspace.plane(plane)?.storage_size();
    let visible_width = target
        .width()
        .min(storage.width().saturating_sub(target.x()));
    let visible_height = target
        .height()
        .min(storage.height().saturating_sub(target.y()));
    let source = PlaneRect::new(source.x(), source.y(), visible_width, visible_height)?;
    let target = PlaneRect::new(target.x(), target.y(), visible_width, visible_height)?;
    let mut samples = RecycledMcSamples::take();
    workspace
        .copy_rect_within_plane_into(plane, source, target, &mut samples)
        .map_err(Into::into)
}

pub(crate) fn intrabc_predict_subpel_plane_into<T: ReconSample>(
    workspace: &mut CurrentFrameWorkspace<T>,
    plane: PlaneId,
    target: PlaneRect,
    scaling: super::mv_scaling::PlaneScaling,
) -> Result<()> {
    let storage = workspace.plane(plane)?.storage_size();
    let params = crate::filters::wienerns_lr::recon::full_recon::intrabc_bilinear_params(
        scaling,
        target.width(),
        target.height(),
        workspace.info().bit_depth(),
    );
    let prediction_len =
        target
            .width()
            .checked_mul(target.height())
            .ok_or(ReconError::ArithmeticOverflow {
                context: "IntrABC prediction sample count",
            })?;
    SUBPEL_PREDICTION_BUFFER.with(|slot| {
        let mut predicted = slot.take().unwrap_or_default();
        predicted.resize(prediction_len, 0);
        let result: Result<()> = (|| {
            {
                let view = ReferencePlaneView::new(
                    workspace.samples(plane)?,
                    storage.width(),
                    storage.height(),
                )?;
                subpel_predict_block_into(&view, &params, &mut predicted)?;
            }
            CurrentFrameSurface::Frame(workspace).write_u16_rect(
                plane,
                target,
                &predicted[..prediction_len],
                target.width(),
            )?;
            Ok(())
        })();
        slot.set(Some(predicted));
        result
    })
}

#[cfg(test)]
#[path = "mc_tests.rs"]
mod tests;
