// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#[cfg(test)]
use splot_recon::BitDepth;
use splot_recon::{
    CurrentFrameWorkspace, DecodedFrame, InterpolationFilter, PlaneId, PlaneRect, ReconError,
    ReconSample, ReferencePlaneView, SubpelPredictParams, WARPED_BLOCK_SIZE,
    WarpPredictBlockParams, blend_compound_average_equal, blend_compound_average_weighted,
    subpel_predict_block, subpel_predict_block_compound_intermediate, warp_predict_block,
    wedge_mask_plane_sample,
};

use super::mv_scaling::{PlaneScaling, derive_plane_scaling, derive_plane_scaling_prescaled};
use super::{Mv, SPEC_MC, unsupported_at};
use crate::Result;
use splot_core::span::ByteOffset;
use splot_recon::math::{clip3, round2};

mod optflow;
mod refinemv;
pub(crate) use optflow::CompoundMotionGrid;

pub(crate) const YUV420_MC_PLANES: [(PlaneId, u32, u32); 3] =
    [(PlaneId::Y, 0, 0), (PlaneId::U, 1, 1), (PlaneId::V, 1, 1)];
pub(crate) const CWP_EQUAL: i16 = 8;

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
    optflow_sad_threshold: Option<u64>,
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
        warp_params: [i64; 6],
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

    pub(crate) const fn with_optflow_sad_threshold(mut self, threshold: Option<u64>) -> Self {
        self.optflow_sad_threshold = threshold;
        self
    }

    pub(crate) fn with_compound_warp(mut self, models: [Option<[i64; 6]>; 2]) -> Self {
        if let InterPrediction::CompoundAverage { warp_params, .. } = &mut self.prediction {
            *warp_params = models;
        }
        self
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
        warp_params: [i64; 6],
    },
    CompoundAverage {
        reference0: &'a DecodedFrame<T>,
        reference1: &'a DecodedFrame<T>,
        mv0: Mv,
        mv1: Mv,
        blend: CompoundBlend,
        optflow_distances: Option<[i32; 2]>,
        warp_params: [Option<[i64; 6]>; 2],
    },
}
#[derive(Clone, Copy, Debug)]
struct CompoundMcBlock<'a, T: ReconSample> {
    reference0: &'a DecodedFrame<T>,
    reference1: &'a DecodedFrame<T>,
    rect: McBlockRect,
    mv0: Mv,
    mv1: Mv,
    interp: InterpolationFilter,
    blend: CompoundBlend,
    optflow_distances: Option<[i32; 2]>,
    warp_params: [Option<[i64; 6]>; 2],
    has_chroma: bool,
    sub8x8_chroma: bool,
    use_refinemv: bool,
    search_refinemv: bool,
    refinemv_switchable: bool,
    optflow_sad_threshold: Option<u64>,
}
pub(crate) fn motion_compensate_inter_block_into<T: ReconSample>(
    workspace: &mut CurrentFrameWorkspace<T>,
    block: InterBlockParams<'_, T>,
    offset: ByteOffset,
) -> Result<()> {
    motion_compensate_inter_block(workspace, block, None, offset).map(drop)
}

pub(crate) fn motion_compensate_inter_block_with_optflow_mvs_into<T: ReconSample>(
    workspace: &mut CurrentFrameWorkspace<T>,
    block: InterBlockParams<'_, T>,
    optflow_unit_size: usize,
    offset: ByteOffset,
) -> Result<Option<[Mv; 2]>> {
    let Some(grid) =
        motion_compensate_inter_block(workspace, block, Some(optflow_unit_size), offset)?
    else {
        return Ok(None);
    };
    Ok(Some(grid.stored_mvs_at_luma_offset(0, 0)?))
}

pub(crate) fn motion_compensate_inter_block_with_motion_grid_into<T: ReconSample>(
    workspace: &mut CurrentFrameWorkspace<T>,
    block: InterBlockParams<'_, T>,
    optflow_unit_size: Option<usize>,
    offset: ByteOffset,
) -> Result<Option<CompoundMotionGrid>> {
    motion_compensate_inter_block(workspace, block, optflow_unit_size, offset)
}

fn motion_compensate_inter_block<T: ReconSample>(
    workspace: &mut CurrentFrameWorkspace<T>,
    block: InterBlockParams<'_, T>,
    optflow_unit_size: Option<usize>,
    offset: ByteOffset,
) -> Result<Option<CompoundMotionGrid>> {
    match block.prediction {
        InterPrediction::Single { reference, mv } => {
            motion_compensate_single_block_into(
                workspace,
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
                workspace,
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
            workspace,
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
    workspace: &mut CurrentFrameWorkspace<T>,
    reference: &DecodedFrame<T>,
    rect: McBlockRect,
    mv: Mv,
    interp: InterpolationFilter,
    has_chroma: bool,
    offset: ByteOffset,
) -> Result<()> {
    motion_compensate_planes(workspace, has_chroma, |workspace, plane, sub_x, sub_y| {
        predict_plane(
            workspace, reference, plane, rect, mv, interp, sub_x, sub_y, offset,
        )
    })
}

fn motion_compensate_planes<T: ReconSample>(
    workspace: &mut CurrentFrameWorkspace<T>,
    has_chroma: bool,
    mut predict: impl FnMut(&mut CurrentFrameWorkspace<T>, PlaneId, u32, u32) -> Result<()>,
) -> Result<()> {
    for (plane, sub_x, sub_y) in YUV420_MC_PLANES {
        if plane != PlaneId::Y && !has_chroma {
            continue;
        }
        predict(workspace, plane, sub_x, sub_y)?;
    }
    Ok(())
}

fn motion_compensate_compound_average_block_into<T: ReconSample>(
    workspace: &mut CurrentFrameWorkspace<T>,
    block: CompoundMcBlock<'_, T>,
    optflow_unit_size: Option<usize>,
    offset: ByteOffset,
) -> Result<Option<CompoundMotionGrid>> {
    let refinemv = block
        .use_refinemv
        .then(|| refinemv::compound_default_refinemv_motion_grid(workspace, block, offset))
        .transpose()?;
    let motion = optflow::compound_motion_grid(
        workspace,
        block,
        optflow_unit_size,
        refinemv.as_ref(),
        offset,
    )?;
    let luma_diff_weighted_mask =
        compound_luma_diff_weighted_mask(workspace, block, motion.as_ref(), offset)?;
    for (plane, sub_x, sub_y) in YUV420_MC_PLANES {
        if plane != PlaneId::Y && !block.has_chroma {
            continue;
        }
        if plane != PlaneId::Y && block.sub8x8_chroma {
            predict_plane(
                workspace,
                block.reference0,
                plane,
                block.rect,
                block.mv0,
                block.interp,
                sub_x,
                sub_y,
                offset,
            )?;
        } else {
            predict_compound_plane(
                workspace,
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
            )?;
        }
    }
    Ok(motion)
}

fn compound_luma_diff_weighted_mask<T: ReconSample>(
    workspace: &CurrentFrameWorkspace<T>,
    block: CompoundMcBlock<'_, T>,
    motion: Option<&CompoundMotionGrid>,
    offset: ByteOffset,
) -> Result<Option<Vec<u16>>> {
    let CompoundBlend::DiffWeighted { inverse } = block.blend else {
        return Ok(None);
    };
    let prediction =
        compound_plane_prediction_for_block(workspace, block, PlaneId::Y, 0, 0, motion, offset)?;
    Ok(Some(diff_weighted_mask(
        &prediction.pred0,
        &prediction.pred1,
        workspace.info().bit_depth(),
        prediction.block_w,
        prediction.block_h,
        inverse,
    )?))
}

fn motion_compensate_single_warp_block_into<T: ReconSample>(
    workspace: &mut CurrentFrameWorkspace<T>,
    reference: &DecodedFrame<T>,
    block: InterBlockParams<'_, T>,
    mv: Mv,
    warp_params: [i64; 6],
    offset: ByteOffset,
) -> Result<()> {
    motion_compensate_planes(
        workspace,
        block.has_chroma,
        |workspace, plane, sub_x, sub_y| {
            if plane != PlaneId::Y && block.sub8x8_chroma {
                predict_plane(
                    workspace,
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
                    workspace,
                    reference,
                    plane,
                    block.rect,
                    warp_params,
                    sub_x,
                    sub_y,
                    offset,
                )
            }
        },
    )
}

#[allow(clippy::too_many_arguments)]
fn predict_plane<T: ReconSample>(
    workspace: &mut CurrentFrameWorkspace<T>,
    reference: &DecodedFrame<T>,
    plane: PlaneId,
    rect: McBlockRect,
    mv: Mv,
    interp: InterpolationFilter,
    sub_x: u32,
    sub_y: u32,
    offset: ByteOffset,
) -> Result<()> {
    let (view, ref_mi_cols, ref_mi_rows) = reference_plane_view(reference, plane, offset)?;

    let (plane_x, plane_y, block_w, block_h) = rect.plane_rect(plane, sub_x, sub_y);

    let scaling = derive_plane_scaling(
        plane_x as i64,
        plane_y as i64,
        i64::from(mv.row),
        i64::from(mv.col),
        sub_x,
        sub_y,
        ref_mi_cols,
        ref_mi_rows,
        block_w as i64,
        block_h as i64,
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
        bit_depth: workspace.info().bit_depth(),
    };
    let predicted = subpel_predict_block(&view, &params)?;

    let packed: Vec<T> = predicted
        .iter()
        .map(|&v| T::try_from_u16(v))
        .collect::<splot_recon::Result<Vec<T>>>()?;

    let rect = PlaneRect::new(plane_x, plane_y, block_w, block_h)?;
    workspace.write_rect(plane, rect, &packed, block_w)?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn predict_warp_plane<T: ReconSample>(
    workspace: &mut CurrentFrameWorkspace<T>,
    reference: &DecodedFrame<T>,
    plane: PlaneId,
    rect: McBlockRect,
    warp_params: [i64; 6],
    sub_x: u32,
    sub_y: u32,
    offset: ByteOffset,
) -> Result<()> {
    let (view, ref_mi_cols, ref_mi_rows) = reference_plane_view(reference, plane, offset)?;
    let (ref_width, ref_height) = (view.width(), view.height());
    let destination_size = workspace.plane(plane)?.storage_size();
    let (destination_width, destination_height) =
        (destination_size.width(), destination_size.height());

    let (plane_x, plane_y, block_w, block_h) = rect.plane_rect(plane, sub_x, sub_y);
    let bit_depth = workspace.info().bit_depth();
    let skip_pred = !splot_recon::warp_shear_is_valid(warp_params)
        || block_w < WARPED_BLOCK_SIZE
        || block_h < WARPED_BLOCK_SIZE;
    if skip_pred {
        for i4 in 0..block_h.div_euclid(4) {
            for j4 in 0..block_w.div_euclid(4) {
                let write_x = plane_x + j4 * 4;
                let write_y = plane_y + i4 * 4;
                if write_x >= destination_width || write_y >= destination_height {
                    continue;
                }
                let unit_x = (plane_x + (j4 & !1) * 4) as i64;
                let unit_y = (plane_y + (i4 & !1) * 4) as i64;
                let (first_x, first_y, last_x, last_y) = ext_warp_unit_bounds(
                    rect,
                    plane,
                    warp_params,
                    unit_x,
                    unit_y,
                    block_w.min(8) as i64,
                    block_h.min(8) as i64,
                    sub_x,
                    sub_y,
                    ref_mi_cols,
                    ref_mi_rows,
                );
                let params = WarpPredictBlockParams {
                    warp_params,
                    block_x: plane_x as i64,
                    block_y: plane_y as i64,
                    subsampling_x: sub_x as u8,
                    subsampling_y: sub_y as u8,
                    first_x,
                    first_y,
                    last_x,
                    last_y,
                    bit_depth,
                };
                let predicted = splot_recon::math::clip1_predicted_samples(
                    splot_recon::ext_warp_predict_unit(&view, &params, i4, j4, false)?,
                    i64::from(bit_depth.max_sample()),
                );
                let packed: Vec<T> = predicted
                    .iter()
                    .map(|&v| T::try_from_u16(v))
                    .collect::<splot_recon::Result<Vec<T>>>()?;
                let rect = PlaneRect::new(write_x, write_y, 4, 4)?;
                workspace.write_rect(plane, rect, &packed, 4)?;
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
                block_x: write_x as i64,
                block_y: write_y as i64,
                subsampling_x: sub_x as u8,
                subsampling_y: sub_y as u8,
                first_x: 0,
                first_y: 0,
                last_x: ref_width as i64 - 1,
                last_y: ref_height as i64 - 1,
                bit_depth,
            };
            let predicted = splot_recon::math::clip1_predicted_samples(
                warp_predict_block(&view, &params, false)?,
                i64::from(bit_depth.max_sample()),
            );
            let write_w = (block_w - local_x).min(WARPED_BLOCK_SIZE);
            let write_h = (block_h - local_y).min(WARPED_BLOCK_SIZE);
            let mut packed: Vec<T> = Vec::with_capacity(write_w.saturating_mul(write_h));
            for row in 0..write_h {
                for col in 0..write_w {
                    packed.push(T::try_from_u16(predicted[row * WARPED_BLOCK_SIZE + col])?);
                }
            }
            let rect = PlaneRect::new(write_x, write_y, write_w, write_h)?;
            workspace.write_rect(plane, rect, &packed, write_w)?;
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn predict_compound_plane<T: ReconSample>(
    workspace: &mut CurrentFrameWorkspace<T>,
    reference0: &DecodedFrame<T>,
    reference1: &DecodedFrame<T>,
    plane: PlaneId,
    rect: McBlockRect,
    mv0: Mv,
    mv1: Mv,
    interp: InterpolationFilter,
    blend: CompoundBlend,
    warp_params: [Option<[i64; 6]>; 2],
    sub_x: u32,
    sub_y: u32,
    luma_diff_weighted_mask: Option<&[u16]>,
    motion: Option<&CompoundMotionGrid>,
    offset: ByteOffset,
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
        compound_plane_prediction_for_block(workspace, block, plane, sub_x, sub_y, motion, offset)?;
    let coded_luma_size = workspace.info().coded_luma_size();
    let frame_w = coded_luma_size.width() >> sub_x;
    let frame_h = coded_luma_size.height() >> sub_y;
    let blended = blend_compound_average(
        &prediction.pred0,
        &prediction.pred1,
        workspace.info().bit_depth(),
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
    )?;

    let packed: Vec<T> = blended
        .iter()
        .map(|&v| T::try_from_u16(v))
        .collect::<splot_recon::Result<Vec<T>>>()?;
    let rect = PlaneRect::new(
        prediction.plane_x,
        prediction.plane_y,
        prediction.block_w,
        prediction.block_h,
    )?;
    workspace.write_rect(plane, rect, &packed, prediction.block_w)?;
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
}

fn compound_plane_prediction_for_block<T: ReconSample>(
    workspace: &CurrentFrameWorkspace<T>,
    block: CompoundMcBlock<'_, T>,
    plane: PlaneId,
    sub_x: u32,
    sub_y: u32,
    motion: Option<&CompoundMotionGrid>,
    offset: ByteOffset,
) -> Result<CompoundPlanePrediction> {
    if let Some(motion) = motion {
        return optflow::compound_optflow_plane_prediction(
            workspace, block, plane, sub_x, sub_y, motion, offset,
        );
    }
    if block.warp_params[0].is_some() || block.warp_params[1].is_some() {
        return compound_warp_plane_prediction(workspace, block, plane, sub_x, sub_y, offset);
    }
    compound_plane_prediction(
        workspace,
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
    workspace: &CurrentFrameWorkspace<T>,
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
    let (view0, ref_mi_cols0, ref_mi_rows0) = reference_plane_view(reference0, plane, offset)?;
    let (view1, ref_mi_cols1, ref_mi_rows1) = reference_plane_view(reference1, plane, offset)?;

    let (plane_x, plane_y, block_w, block_h) = rect.plane_rect(plane, sub_x, sub_y);

    let scaling0 = derive_plane_scaling(
        plane_x as i64,
        plane_y as i64,
        i64::from(mv0.row),
        i64::from(mv0.col),
        sub_x,
        sub_y,
        ref_mi_cols0,
        ref_mi_rows0,
        block_w as i64,
        block_h as i64,
    );
    let scaling1 = derive_plane_scaling(
        plane_x as i64,
        plane_y as i64,
        i64::from(mv1.row),
        i64::from(mv1.col),
        sub_x,
        sub_y,
        ref_mi_cols1,
        ref_mi_rows1,
        block_w as i64,
        block_h as i64,
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
        bit_depth: workspace.info().bit_depth(),
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
        bit_depth: workspace.info().bit_depth(),
    };
    let pred0 = subpel_predict_block_compound_intermediate(&view0, &params0)?;
    let pred1 = subpel_predict_block_compound_intermediate(&view1, &params1)?;

    Ok(CompoundPlanePrediction {
        pred0,
        pred1,
        plane_x,
        plane_y,
        block_w,
        block_h,
        scaling0,
        scaling1,
    })
}

/// AV2 § 7.13.3.14 compound LOCALWARP plane prediction: warp each list with its
/// own § 7.13.3.23 model (§ 7.13.3.19 block warp, § 7.13.3.20 extended warp for
/// invalid shear / sub-8x8, or translational when the list has no samples), then
/// feed the two § 7.13.3.16 `Preds[refList]` intermediates to the compound blend.
fn compound_warp_plane_prediction<T: ReconSample>(
    workspace: &CurrentFrameWorkspace<T>,
    block: CompoundMcBlock<'_, T>,
    plane: PlaneId,
    sub_x: u32,
    sub_y: u32,
    offset: ByteOffset,
) -> Result<CompoundPlanePrediction> {
    let (plane_x, plane_y, block_w, block_h) = block.rect.plane_rect(plane, sub_x, sub_y);
    let pred0 = compound_ref_intermediate(
        workspace,
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
        workspace,
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
    })
}

struct CompoundRefIntermediate {
    samples: Vec<i32>,
    scaling: PlaneScaling,
}

#[allow(clippy::too_many_arguments)]
fn compound_ref_intermediate<T: ReconSample>(
    workspace: &CurrentFrameWorkspace<T>,
    reference: &DecodedFrame<T>,
    plane: PlaneId,
    rect: McBlockRect,
    warp_params: Option<[i64; 6]>,
    mv: Mv,
    interp: InterpolationFilter,
    sub_x: u32,
    sub_y: u32,
    offset: ByteOffset,
) -> Result<CompoundRefIntermediate> {
    let (view, ref_mi_cols, ref_mi_rows) = reference_plane_view(reference, plane, offset)?;
    let (plane_x, plane_y, block_w, block_h) = rect.plane_rect(plane, sub_x, sub_y);
    let bit_depth = workspace.info().bit_depth();
    let scaling = derive_plane_scaling(
        plane_x as i64,
        plane_y as i64,
        i64::from(mv.row),
        i64::from(mv.col),
        sub_x,
        sub_y,
        ref_mi_cols,
        ref_mi_rows,
        block_w as i64,
        block_h as i64,
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
        || block_h < WARPED_BLOCK_SIZE;
    if skip_pred {
        for i4 in 0..block_h.div_euclid(4) {
            for j4 in 0..block_w.div_euclid(4) {
                let (first_x, first_y, last_x, last_y) = ext_warp_unit_bounds(
                    rect,
                    plane,
                    warp_params,
                    (plane_x + (j4 & !1) * 4) as i64,
                    (plane_y + (i4 & !1) * 4) as i64,
                    block_w.min(8) as i64,
                    block_h.min(8) as i64,
                    sub_x,
                    sub_y,
                    ref_mi_cols,
                    ref_mi_rows,
                );
                let params = WarpPredictBlockParams {
                    warp_params,
                    block_x: plane_x as i64,
                    block_y: plane_y as i64,
                    subsampling_x: sub_x as u8,
                    subsampling_y: sub_y as u8,
                    first_x,
                    first_y,
                    last_x,
                    last_y,
                    bit_depth,
                };
                let predicted = splot_recon::ext_warp_predict_unit(&view, &params, i4, j4, true)?;
                write_compound_section(&mut samples, block_w, j4 * 4, i4 * 4, &predicted, 4, 4, 4);
            }
        }
    } else {
        for local_y in (0..block_h).step_by(WARPED_BLOCK_SIZE) {
            for local_x in (0..block_w).step_by(WARPED_BLOCK_SIZE) {
                let params = WarpPredictBlockParams {
                    warp_params,
                    block_x: (plane_x + local_x) as i64,
                    block_y: (plane_y + local_y) as i64,
                    subsampling_x: sub_x as u8,
                    subsampling_y: sub_y as u8,
                    first_x: 0,
                    first_y: 0,
                    last_x: ref_width as i64 - 1,
                    last_y: ref_height as i64 - 1,
                    bit_depth,
                };
                let predicted = splot_recon::warp_predict_block(&view, &params, true)?;
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

#[allow(clippy::too_many_arguments)]
fn blend_compound_average(
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
) -> splot_recon::Result<Vec<u16>> {
    let CompoundBlend::Average {
        implicit_mask,
        cwp_weight,
    } = blend
    else {
        return blend_compound_diff_weighted(
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
        );
    };
    if !implicit_mask {
        return blend_compound_average_weighted(pred0, pred1, bit_depth, cwp_weight);
    }
    if pred0.len() != pred1.len() {
        return blend_compound_average_weighted(pred0, pred1, bit_depth, cwp_weight);
    }
    if cwp_weight != CWP_EQUAL {
        return blend_compound_average_weighted(pred0, pred1, bit_depth, cwp_weight);
    }

    let last_x = frame_w as i64 - 1;
    let last_y = frame_h as i64 - 1;
    let ref_start_x0 = scaling0.start_x >> 10;
    let ref_start_y0 = scaling0.start_y >> 10;
    let ref_start_x1 = scaling1.start_x >> 10;
    let ref_start_y1 = scaling1.start_y >> 10;
    let ref_mi_cols = ((frame_w << sub_x).div_ceil(4)) as i64;
    let ref_mi_rows = ((frame_h << sub_y).div_ceil(4)) as i64;
    let extent = |scaling: PlaneScaling| {
        let end_x = scaling.start_x + scaling.step_x * w.saturating_sub(1) as i64;
        let end_y = scaling.start_y + scaling.step_y * h.saturating_sub(1) as i64;
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
                derive_plane_scaling_prescaled(
                    plane_x as i64,
                    plane_y as i64,
                    i64::from(mvs[reference][0]),
                    i64::from(mvs[reference][1]),
                    sub_x,
                    sub_y,
                    ref_mi_cols,
                    ref_mi_rows,
                )
            })
        }),
    };
    let onscreen = |extent: (i64, i64, i64, i64)| {
        extent.0 >= 0 && extent.1 >= 0 && extent.2 <= last_x && extent.3 <= last_y
    };
    if pred0.len() == w.saturating_mul(h)
        && uniform_scaling.is_some_and(|scaling| scaling.into_iter().map(extent).all(onscreen))
    {
        return blend_compound_average_equal(pred0, pred1, bit_depth);
    }
    let max_sample = i64::from(bit_depth.max_sample());
    let shift = 1 + compound_inter_post_round();
    let mut blended = Vec::with_capacity(w.saturating_mul(h));
    for (idx, (&left, &right)) in pred0
        .iter()
        .zip(pred1.iter())
        .take(w.saturating_mul(h))
        .enumerate()
    {
        let row = idx / w;
        let col = idx % w;
        let starts = if let Some(motion) = motion {
            let mvs = motion.at_luma_offset(col << sub_x, row << sub_y)?;
            core::array::from_fn(|reference| {
                let scaling = derive_plane_scaling_prescaled(
                    (plane_x + col) as i64,
                    (plane_y + row) as i64,
                    i64::from(mvs[reference][0]),
                    i64::from(mvs[reference][1]),
                    sub_x,
                    sub_y,
                    ref_mi_cols,
                    ref_mi_rows,
                );
                (scaling.start_x >> 10, scaling.start_y >> 10)
            })
        } else {
            [
                (ref_start_x0 + col as i64, ref_start_y0 + row as i64),
                (ref_start_x1 + col as i64, ref_start_y1 + row as i64),
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
        let sample = round2(i64::from(mask * left + (2 - mask) * right), shift);
        blended.push(clip3(0, max_sample, sample) as u16);
    }
    Ok(blended)
}

#[allow(clippy::too_many_arguments)]
fn blend_compound_diff_weighted(
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
) -> splot_recon::Result<Vec<u16>> {
    if pred0.len() != pred1.len() {
        return blend_compound_average_equal(pred0, pred1, bit_depth);
    }
    if let CompoundBlend::Wedge { index, sign } = blend {
        return blend_compound_wedge(
            pred0, pred1, bit_depth, w, h, luma_w, luma_h, index, sign, sub_x, sub_y,
        );
    }
    let CompoundBlend::DiffWeighted { inverse } = blend else {
        return blend_compound_average_equal(pred0, pred1, bit_depth);
    };
    let max_sample = i64::from(bit_depth.max_sample());
    let blend_shift = 6 + compound_inter_post_round();
    let mask = if let Some(luma_mask) = luma_diff_weighted_mask {
        subsample_diff_weighted_luma_mask(luma_mask, w, h, sub_x, sub_y)?
    } else {
        diff_weighted_mask(pred0, pred1, bit_depth, w, h, inverse)?
    };
    Ok(pred0
        .iter()
        .zip(pred1.iter())
        .zip(mask.iter())
        .take(w.saturating_mul(h))
        .map(|((&left, &right), &mask)| {
            let blended = round2(
                i64::from(mask) * i64::from(left) + i64::from(64 - mask) * i64::from(right),
                blend_shift,
            );
            clip3(0, max_sample, blended) as u16
        })
        .collect())
}

#[allow(clippy::too_many_arguments)]
fn blend_compound_wedge(
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
) -> splot_recon::Result<Vec<u16>> {
    if pred0.len() != pred1.len() {
        return Err(ReconError::CompoundBlendLengthMismatch {
            left_len: pred0.len(),
            right_len: pred1.len(),
        });
    }
    let max_sample = i64::from(bit_depth.max_sample());
    let shift = 6 + compound_inter_post_round();
    let mut out = Vec::with_capacity(w.saturating_mul(h));
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
            let blended = round2(
                i64::from(mask) * i64::from(pred0[idx])
                    + i64::from(64 - mask) * i64::from(pred1[idx]),
                shift,
            );
            out.push(clip3(0, max_sample, blended) as u16);
        }
    }
    Ok(out)
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
        let diff = round2(i64::from((left - right).unsigned_abs()), diff_round);
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
            let mut sum = 0i64;
            for dy in 0..scale_y {
                for dx in 0..scale_x {
                    let mask_x = x * scale_x + dx;
                    let mask_y = y * scale_y + dy;
                    sum += i64::from(luma_mask[mask_y * luma_w + mask_x]);
                }
            }
            let averaged = round2(sum, sub_x + sub_y);
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
    warp_params: [i64; 6],
    unit_x: i64,
    unit_y: i64,
    bbox_w: i64,
    bbox_h: i64,
    sub_x: u32,
    sub_y: u32,
    ref_mi_cols: i64,
    ref_mi_rows: i64,
) -> (i64, i64, i64, i64) {
    const WARPEDMODEL_PREC_BITS: u32 = 16;
    const MV_BOUND: i64 = 1 << 16;
    const MV_BORDER: i64 = 128;
    let src_x = (unit_x + (bbox_w >> 1)) << sub_x;
    let src_y = (unit_y + (bbox_h >> 1)) << sub_y;
    let dst_x = warp_params[2] * src_x + warp_params[3] * src_y + warp_params[0];
    let dst_y = warp_params[4] * src_x + warp_params[5] * src_y + warp_params[1];
    let mv_row = clip3(
        -MV_BOUND + 1,
        MV_BOUND - 1,
        (dst_y - (src_y << WARPEDMODEL_PREC_BITS)) >> (WARPEDMODEL_PREC_BITS - 3),
    );
    let mv_col = clip3(
        -MV_BOUND + 1,
        MV_BOUND - 1,
        (dst_x - (src_x << WARPEDMODEL_PREC_BITS)) >> (WARPEDMODEL_PREC_BITS - 3),
    );
    let (luma_x, luma_y, luma_w, luma_h) = rect.plane_luma_rect(plane);
    let mi_row = (luma_y / 4) as i64;
    let mi_col = (luma_x / 4) as i64;
    let bh4 = (luma_h / 4) as i64;
    let bw4 = (luma_w / 4) as i64;
    let mv_row = clip3(
        -(mi_row + bh4) * 32 - MV_BORDER,
        (ref_mi_rows - mi_row) * 32 + MV_BORDER,
        mv_row,
    );
    let mv_col = clip3(
        -(mi_col + bw4) * 32 - MV_BORDER,
        (ref_mi_cols - mi_col) * 32 + MV_BORDER,
        mv_col,
    );
    let scaling = derive_plane_scaling(
        unit_x,
        unit_y,
        mv_row,
        mv_col,
        sub_x,
        sub_y,
        ref_mi_cols,
        ref_mi_rows,
        bbox_w,
        bbox_h,
    );
    let first_x = clip3(0, scaling.last_x, (scaling.start_x >> 10) - 3);
    let first_y = clip3(0, scaling.last_y, (scaling.start_y >> 10) - 3);
    let last_x = clip3(
        0,
        scaling.last_x,
        ((scaling.start_x + scaling.step_x * (bbox_w - 1)) >> 10) + 4,
    );
    let last_y = clip3(
        0,
        scaling.last_y,
        ((scaling.start_y + scaling.step_y * (bbox_h - 1)) >> 10) + 4,
    );
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
    let packed: Vec<T> = predicted
        .iter()
        .map(|&v| T::try_from_u16(v))
        .collect::<splot_recon::Result<Vec<T>>>()?;
    workspace.write_rect(plane, target, &packed, target.width())?;
    Ok(())
}

pub(crate) fn reference_plane_view<T: ReconSample>(
    reference: &DecodedFrame<T>,
    plane: PlaneId,
    offset: ByteOffset,
) -> Result<(ReferencePlaneView<'_, T>, i64, i64)> {
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
    let ref_mi_cols = luma_visible.width().div_ceil(4) as i64;
    let ref_mi_rows = luma_visible.height().div_ceil(4) as i64;

    Ok((view, ref_mi_cols, ref_mi_rows))
}

#[cfg(test)]
#[path = "mc_tests.rs"]
mod tests;
