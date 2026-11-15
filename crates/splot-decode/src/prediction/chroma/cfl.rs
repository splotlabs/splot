// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 § 7.13.5/§ 7.13.6 chroma-from-luma reconstruction helpers.
//!
//! § 7.13.5 applies 64-sample clamping to `L`; `lumaAvg` and `implicitAlpha` use
//! transform-local reference lines (docs/spec/av2/1.0.0/07-decoding-process.md).
//!
//! A block resolves its luma rows, neighbour sums and per-plane models once,
//! before either chroma plane is written, then predicts each plane straight
//! into the frame when it codes no residual.
//!
//! Feature tracking: `INFRA-DECODE-PARALLEL-STAGES`,
//! `DECODE-GENERAL-INTRA-CFL-NON-420`,
//! `DECODE-MHCCP-FRAME-EDGE-REFERENCE-EXTENT`.

use fearless_simd::{Bytes as _, Select as _, Simd, SimdBase, i16x8, i32x8, u16x8};

use splot_recon::math::{approx_divide, resolve_division};
use splot_recon::{
    BitDepth, CurrentFrameWorkspace, IntraRectBlockSize, PixelFormat, PlaneId, ReconSample,
};
use splot_recon::{MhccpEdge, MhccpPredictor, PredictedSample, derive_mhccp_params};

#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
use super::cfl_luma::{
    Luma420Block, cfl_alpha_fits_i16, cfl_predict_pair_u8, cfl_predict_pair_u16, cfl_predict_u8,
    cfl_predict_u16, luma_420, luma_token,
};
use crate::bitstream::tile_payload::{
    CflMultiDirection, CflParams, CoeffBlock, GeneralIntraResidualError,
    add_general_intra_coeff_block_residual_in_place,
};
use crate::pipeline::reconstruct::{commit_intra_prediction, predict_intra_in_frame};
use crate::tile::block_context::NeighbourAvailability;

const MI_SIZE: usize = 4;
/// `Cfl_Filters_420[filterIdx][dy + 1][dx + 1]` (§ 7.13.5): the luma taps
/// of a 4:2:0 CfL sample, `dy` and `dx` in `-1..=1`, summing to `8`.
pub const CFL_FILTERS_420: [[[i32; 3]; 3]; 3] = [
    [[0, 0, 0], [0, 2, 2], [0, 2, 2]],
    [[0, 0, 0], [1, 2, 1], [1, 2, 1]],
    [[0, 1, 0], [1, 4, 1], [0, 1, 0]],
];
/// `Cfl_Filters_422[filterIdx][dx + 1]` (§ 7.13.5): the luma taps of a 4:2:2
/// CfL sample, summing to `8`.
pub const CFL_FILTERS_422: [[i32; 3]; 3] = [[0, 4, 4], [2, 4, 2], [0, 8, 0]];
const CFL_ALPHA_SHIFT: u32 = 11;
const CFL_ALPHA_SCALE: i32 = 32;
const CFL_DERIVED_ALPHA_SHIFT: u8 = 8;
const NUM_REF_SAM_CFL: usize = 8;
/// The widest CfL block axis.
const MAX_CFL_AXIS: usize = 64;
/// [`CflLuma`] filter selector for weights only known at run time.
const RUNTIME_WEIGHTS: usize = 3;

/// A chroma edge the block's neighbours claim is not stored.
const CHROMA_EDGE_UNSTORED: GeneralIntraResidualError =
    GeneralIntraResidualError::InvalidReconstructionState {
        context: "CfL chroma edge",
    };
/// An MHCCP block whose luma references do not fit its prediction.
const MHCCP_GEOMETRY: GeneralIntraResidualError =
    GeneralIntraResidualError::InvalidReconstructionState {
        context: "MHCCP reference geometry",
    };

/// One chroma plane of a CfL block: its id, coefficients and neighbours.
type CflPlane<'c> = (PlaneId, CoeffBlock<'c>, NeighbourAvailability);

/// The chroma transform block every plane of a CfL block shares.
#[derive(Clone, Copy)]
struct CflBlock {
    x: usize,
    y: usize,
    width: usize,
    height: usize,
    size: IntraRectBlockSize,
    qindex: u32,
    sb_mib: usize,
    bit_depth: BitDepth,
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn reconstruct_general_intra_chroma_cfl_block_into<T: ReconSample>(
    scratch: &mut crate::pipeline::general_intra::GeneralIntraReconScratch<T>,
    workspace: &mut CurrentFrameWorkspace<T>,
    block: CoeffBlock<'_>,
    plane_id: PlaneId,
    x: usize,
    y: usize,
    log2_width: u32,
    log2_height: u32,
    qindex: u32,
    cfl_params: CflParams,
    cfl_ds_filter_index: u8,
    sb_mib: usize,
    neighbours: NeighbourAvailability,
    bit_depth: BitDepth,
) -> core::result::Result<(), GeneralIntraResidualError> {
    let geometry = CflBlock::new(x, y, log2_width, log2_height, qindex, sb_mib, bit_depth)?;
    reconstruct_cfl_planes(
        scratch,
        workspace,
        &[(plane_id, block, neighbours)],
        geometry,
        cfl_params,
        cfl_ds_filter_index,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn reconstruct_general_intra_chroma_cfl_pair_into<T: ReconSample>(
    scratch: &mut crate::pipeline::general_intra::GeneralIntraReconScratch<T>,
    workspace: &mut CurrentFrameWorkspace<T>,
    u_block: CoeffBlock<'_>,
    v_block: CoeffBlock<'_>,
    x: usize,
    y: usize,
    log2_width: u32,
    log2_height: u32,
    qindex: u32,
    cfl_params: CflParams,
    cfl_ds_filter_index: u8,
    sb_mib: usize,
    u_neighbours: NeighbourAvailability,
    v_neighbours: NeighbourAvailability,
    bit_depth: BitDepth,
) -> core::result::Result<(), GeneralIntraResidualError> {
    let geometry = CflBlock::new(x, y, log2_width, log2_height, qindex, sb_mib, bit_depth)?;
    let u = (PlaneId::U, u_block, u_neighbours);
    let v = (PlaneId::V, v_block, v_neighbours);
    if matches!(cfl_params, CflParams::Multi { .. }) && u_neighbours != v_neighbours {
        for plane in [u, v] {
            reconstruct_cfl_planes(
                scratch,
                workspace,
                &[plane],
                geometry,
                cfl_params,
                cfl_ds_filter_index,
            )?;
        }
        return Ok(());
    }
    reconstruct_cfl_planes(
        scratch,
        workspace,
        &[u, v],
        geometry,
        cfl_params,
        cfl_ds_filter_index,
    )
}

impl CflBlock {
    fn new(
        x: usize,
        y: usize,
        log2_width: u32,
        log2_height: u32,
        qindex: u32,
        sb_mib: usize,
        bit_depth: BitDepth,
    ) -> core::result::Result<Self, GeneralIntraResidualError> {
        let size = IntraRectBlockSize::new(
            u8::try_from(log2_width).unwrap_or(u8::MAX),
            u8::try_from(log2_height).unwrap_or(u8::MAX),
        )?;
        Ok(Self {
            x,
            y,
            width: size.width(),
            height: size.height(),
            size,
            qindex,
            sb_mib,
            bit_depth,
        })
    }
}

/// Reconstructs `planes` (one, or U then V with the same geometry): the
/// luma side is analysed once for all planes before any plane is written.
fn reconstruct_cfl_planes<T: ReconSample>(
    scratch: &mut crate::pipeline::general_intra::GeneralIntraReconScratch<T>,
    workspace: &mut CurrentFrameWorkspace<T>,
    planes: &[CflPlane<'_>],
    block: CflBlock,
    cfl_params: CflParams,
    cfl_ds_filter_index: u8,
) -> core::result::Result<(), GeneralIntraResidualError> {
    let multi = match cfl_params {
        CflParams::Multi { direction } => Some(direction),
        _ => None,
    };
    let Some(filter) = cfl_filter_index(cfl_ds_filter_index) else {
        return Err(GeneralIntraResidualError::InvalidReconstructionState {
            context: if multi.is_some() {
                "MHCCP filter"
            } else {
                "CfL filter"
            },
        });
    };
    let crate::pipeline::general_intra::GeneralIntraReconScratch {
        cfl_luma_ac,
        cfl_prediction,
        mhccp_chroma,
        ..
    } = scratch;
    let mut models = [None; 2];
    let Some(models) = models.get_mut(..planes.len()) else {
        return Err(GeneralIntraResidualError::InvalidReconstructionState {
            context: "CfL plane count",
        });
    };
    let mut mhccp_layout = None;
    let mut edges = CflLumaEdges::default();
    let average_q3 = {
        let luma_plane = workspace.plane(PlaneId::Y)?;
        let specialised = if workspace.info().pixel_format() == PixelFormat::Yuv420 {
            filter
        } else {
            RUNTIME_WEIGHTS
        };
        macro_rules! analyse {
            ($samples:expr, $filter:literal) => {
                analyse_cfl_block(
                    workspace,
                    &CflLuma::<_, $filter>::new(workspace, $samples, filter)?,
                    planes,
                    block,
                    multi,
                    &mut edges,
                    cfl_luma_ac,
                    mhccp_chroma,
                    models,
                    &mut mhccp_layout,
                )?
            };
            ($samples:expr) => {
                match specialised {
                    0 => analyse!($samples, 0),
                    1 => analyse!($samples, 1),
                    2 => analyse!($samples, 2),
                    _ => analyse!($samples, 3),
                }
            };
        }
        if let Some(samples) = T::u8_slice(luma_plane.samples()) {
            analyse!(samples)
        } else if let Some(samples) = T::u16_slice(luma_plane.samples()) {
            analyse!(samples)
        } else {
            None
        }
    };
    let (width, height) = (block.width, block.height);
    if let Some((average_q3, start)) = average_q3 {
        let Some(luma) = cfl_luma_ac.get(start..start + width * height) else {
            return Err(GeneralIntraResidualError::InvalidReconstructionState {
                context: "CfL luma length",
            });
        };
        let linear = LinearCfl {
            edges: &edges,
            luma,
            average_q3,
            block,
            cfl_params,
        };
        if let &[u, v] = planes
            && u.2 == v.2
            && block.bit_depth == workspace.info().bit_depth()
            && linear.reconstruct_pair(workspace, [u, v], cfl_prediction)?
        {
            return Ok(());
        }
        for &plane in planes {
            linear.reconstruct(workspace, plane, cfl_prediction)?;
        }
        return Ok(());
    }
    let Some(layout) = mhccp_layout else {
        return Err(GeneralIntraResidualError::InvalidReconstructionState {
            context: "MHCCP references",
        });
    };
    let ([linear, centre], _) = layout.views(cfl_luma_ac);
    if let (&[u, v], &[Some(u_model), Some(v_model)]) = (planes, &*models)
        && block.bit_depth == workspace.info().bit_depth()
    {
        let predicted =
            workspace.with_intra_chroma_frames(block.x, block.y, block.size, |frames| {
                let [u_frame, v_frame] = frames;
                let outs = [u_frame.target(), v_frame.target()];
                MhccpPredictor::predict_planes([&u_model, &v_model], linear, centre, width, outs)
                    .then_some(())
                    .ok_or(MHCCP_GEOMETRY)
            })?;
        if predicted.is_some() {
            for (plane_id, coeffs, _) in [u, v] {
                if coeffs.eob != 0 {
                    add_general_intra_coeff_block_residual_in_place(
                        workspace,
                        coeffs,
                        plane_id,
                        block.x,
                        block.y,
                        block.size,
                        block.qindex,
                        false,
                        None,
                        None,
                        block.bit_depth,
                    )?;
                }
            }
            return Ok(());
        }
    }
    for (&(plane_id, coeffs, _), &model) in planes.iter().zip(models.iter()) {
        let Some(predictor) = model else {
            return Err(GeneralIntraResidualError::InvalidReconstructionState {
                context: "MHCCP model",
            });
        };
        let predict = |out: &mut [T], stride: usize| {
            predictor
                .predict_block(linear, centre, width, out, stride)
                .then_some(())
                .ok_or(MHCCP_GEOMETRY)
        };
        if predict_intra_in_frame(
            workspace,
            coeffs,
            plane_id,
            block.x,
            block.y,
            block.size,
            block.qindex,
            false,
            None,
            None,
            block.bit_depth,
            predict,
        )? {
            continue;
        }
        cfl_prediction.clear();
        cfl_prediction.resize(width * height, T::default());
        predict(cfl_prediction.as_mut_slice(), width)?;
        commit_intra_prediction(
            workspace,
            coeffs,
            cfl_prediction,
            plane_id,
            block.x,
            block.y,
            block.size,
            block.qindex,
            false,
            None,
            None,
            block.bit_depth,
        )?;
    }
    Ok(())
}

/// The § 7.13.5 luma side of a linear CfL block, shared by its planes.
struct LinearCfl<'a> {
    edges: &'a CflLumaEdges,
    luma: &'a [i16],
    average_q3: i32,
    block: CflBlock,
    cfl_params: CflParams,
}

impl LinearCfl<'_> {
    /// The `[dc, alpha, lumaAvg, max]` prediction model of `plane_id` from
    /// its DC and `implicitAlpha`.
    fn model(&self, plane_id: PlaneId, (dc, implicit_alpha): (i32, i32)) -> [i32; 4] {
        let alpha_q3 = match self.cfl_params {
            CflParams::Explicit { alpha_u, alpha_v } => {
                let alpha = if plane_id == PlaneId::U {
                    alpha_u
                } else {
                    alpha_v
                };
                i32::from(alpha) * CFL_ALPHA_SCALE
            }
            CflParams::DerivedAlpha => implicit_alpha,
            CflParams::Multi { .. } => 0,
        };
        [
            dc,
            alpha_q3,
            self.average_q3,
            i32::from(self.block.bit_depth.max_sample()),
        ]
    }

    /// Predicts both chroma planes of a block lying in the stored frame,
    /// with the same neighbours, in place and adds their residuals; `false`,
    /// writing nothing, for a block outside the stored frame.
    fn reconstruct_pair<T: ReconSample>(
        &self,
        workspace: &mut CurrentFrameWorkspace<T>,
        [u, v]: [CflPlane<'_>; 2],
        columns: &mut Vec<T>,
    ) -> core::result::Result<bool, GeneralIntraResidualError> {
        let CflBlock {
            x,
            y,
            width,
            height,
            size,
            qindex,
            bit_depth,
            ..
        } = self.block;
        let neighbours = u.2;
        let predicted = workspace.with_intra_chroma_frames(x, y, size, |frames| {
            let [u_frame, v_frame] = frames;
            if columns.len() < 2 * MAX_CFL_AXIS {
                columns.resize(2 * MAX_CFL_AXIS, T::default());
            }
            let (u_left, v_left) = columns.split_at_mut(MAX_CFL_AXIS);
            let left = if neighbours.has_left() && x > 0 {
                if u_frame.left_column(&mut u_left[..height]) != height
                    || v_frame.left_column(&mut v_left[..height]) != height
                {
                    return Err(CHROMA_EDGE_UNSTORED);
                }
                Some([&u_left[..], &v_left[..]])
            } else {
                None
            };
            let above = if neighbours.has_above() && y > 0 {
                let rows = [u_frame.above_row(), v_frame.above_row()];
                if rows.iter().any(|row| row.len() < width) {
                    return Err(CHROMA_EDGE_UNSTORED);
                }
                Some(rows)
            } else {
                None
            };
            let derive = self.cfl_params == CflParams::DerivedAlpha;
            let [u_model, v_model] = self
                .edges
                .chroma_models(above, left, self.block, neighbours, derive)?;
            let models = [
                self.model(PlaneId::U, u_model),
                self.model(PlaneId::V, v_model),
            ];
            cfl_predict_pair(
                models,
                self.luma,
                width,
                [u_frame.target(), v_frame.target()],
            )
        })?;
        if predicted.is_none() {
            return Ok(false);
        }
        for (plane_id, coeffs, _) in [u, v] {
            if coeffs.eob != 0 {
                add_general_intra_coeff_block_residual_in_place(
                    workspace, coeffs, plane_id, x, y, size, qindex, false, None, None, bit_depth,
                )?;
            }
        }
        Ok(true)
    }

    /// Predicts one chroma plane from its edges and adds its residual: in
    /// place when the block lies in the stored frame, else staged.
    fn reconstruct<T: ReconSample>(
        &self,
        workspace: &mut CurrentFrameWorkspace<T>,
        (plane_id, coeffs, neighbours): CflPlane<'_>,
        staged: &mut Vec<T>,
    ) -> core::result::Result<(), GeneralIntraResidualError> {
        let CflBlock {
            x,
            y,
            width,
            height,
            size,
            qindex,
            bit_depth,
            ..
        } = self.block;
        let derive = self.cfl_params == CflParams::DerivedAlpha;
        if bit_depth == workspace.info().bit_depth() {
            let predicted = workspace.with_intra_block_frame(plane_id, x, y, size, |frame| {
                let mut left = [T::default(); MAX_CFL_AXIS];
                let left = if neighbours.has_left() && x > 0 {
                    if frame.left_column(&mut left[..height]) != height {
                        return Err(CHROMA_EDGE_UNSTORED);
                    }
                    Some([&left[..]])
                } else {
                    None
                };
                let above = match frame.above_row() {
                    _ if !(neighbours.has_above() && y > 0) => None,
                    run if run.len() < width => return Err(CHROMA_EDGE_UNSTORED),
                    run => Some([run]),
                };
                let [edges] = self
                    .edges
                    .chroma_models(above, left, self.block, neighbours, derive)?;
                let (out, stride) = frame.target();
                cfl_predict_block(self.model(plane_id, edges), self.luma, width, out, stride)
            })?;
            if predicted.is_some() {
                if coeffs.eob != 0 {
                    add_general_intra_coeff_block_residual_in_place(
                        workspace, coeffs, plane_id, x, y, size, qindex, false, None, None,
                        bit_depth,
                    )?;
                }
                return Ok(());
            }
        }
        let chroma = PlaneSamples::of(workspace, plane_id)?;
        let edges = self
            .edges
            .plane_chroma_model(&chroma, self.block, neighbours, derive);
        let model = self.model(plane_id, edges?);
        staged.clear();
        staged.resize(width * height, T::default());
        cfl_predict_block(model, self.luma, width, staged, width)?;
        commit_intra_prediction(
            workspace, coeffs, staged, plane_id, x, y, size, qindex, false, None, None, bit_depth,
        )
    }
}

/// Builds the shared luma side of a block: the luma edges, `L` and
/// `lumaAvg` (CfL, returned with where `L` starts in `luma_ac`) or the luma
/// references in `luma_ac` and every plane's predictor (MHCCP).
#[allow(clippy::too_many_arguments)]
fn analyse_cfl_block<T: ReconSample, P: CflLaneSample, const F: usize>(
    workspace: &CurrentFrameWorkspace<T>,
    luma: &CflLuma<'_, P, F>,
    planes: &[CflPlane<'_>],
    block: CflBlock,
    multi: Option<CflMultiDirection>,
    edges: &mut CflLumaEdges,
    luma_ac: &mut Vec<i16>,
    mhccp_chroma: &mut [Vec<u16>; 2],
    models: &mut [Option<MhccpPredictor>],
    mhccp_layout: &mut Option<MhccpLayout>,
) -> core::result::Result<Option<(i32, usize)>, GeneralIntraResidualError> {
    let Some(&(_, _, neighbours)) = planes.first() else {
        return Ok(None);
    };
    let Some(direction) = multi else {
        return baseline_simd!(simd => {
            let start = edges.gather(simd, luma, planes, block, luma_ac)?;
            Ok(Some((edges.average_q3(block, neighbours)?, start)))
        });
    };
    let geometry = MhccpGeometry::new(workspace, planes[0].0, block, neighbours)?;
    let layout = geometry.layout(block, direction.value());
    baseline_simd!(simd => geometry.gather_luma(simd, luma, block, &layout, luma_ac))?;
    for (&(plane_id, _, _), refs) in planes.iter().zip(mhccp_chroma.iter_mut()) {
        geometry.gather_chroma(workspace, plane_id, block, &layout, refs)?;
    }
    let (_, edges) = layout.views(luma_ac);
    let [u, v] = mhccp_chroma
        .each_ref()
        .map(|refs| refs.split_at(layout.above_len.min(refs.len())));
    let multi = |params| Some(MhccpPredictor::new(params, block.bit_depth));
    if let [u_model, v_model] = models {
        let [u_params, v_params] =
            derive_mhccp_params(edges, [[u.0, u.1], [v.0, v.1]], block.bit_depth);
        (*u_model, *v_model) = (multi(u_params), multi(v_params));
    } else {
        let [params] = derive_mhccp_params(edges, [[u.0, u.1]], block.bit_depth);
        models.fill(multi(params));
    }
    *mhccp_layout = Some(layout);
    Ok(None)
}

/// [`cfl_predict_block`] of the U and V `models` of one `L`, in one pass
/// over it when both alphas fit the 16-bit lanes.
fn cfl_predict_pair<T: ReconSample>(
    models: [[i32; 4]; 2],
    luma: &[i16],
    width: usize,
    [(u, u_stride), (v, v_stride)]: [(&mut [T], usize); 2],
) -> core::result::Result<(), GeneralIntraResidualError> {
    let height = luma.len() / width.max(1);
    #[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
    if width > 0
        && luma.len().is_multiple_of(16)
        && models.iter().all(|model| cfl_alpha_fits_i16(model[1]))
        && [(&*u, u_stride), (&*v, v_stride)]
            .iter()
            .all(|(out, stride)| (height.max(1) - 1) * stride + width <= out.len())
        && let Some(token) = luma_token()
    {
        if let (Some(u), Some(v)) = (T::u16_slice_mut(&mut *u), T::u16_slice_mut(&mut *v)) {
            cfl_predict_pair_u16(token, models, luma, width, [u, v], [u_stride, v_stride]);
            return Ok(());
        }
        if models[0][3] == i32::from(u8::MAX)
            && let (Some(u), Some(v)) = (T::u8_slice_mut(&mut *u), T::u8_slice_mut(&mut *v))
        {
            cfl_predict_pair_u8(token, models, luma, width, [u, v], [u_stride, v_stride]);
            return Ok(());
        }
    }
    cfl_predict_block(models[0], luma, width, u, u_stride)?;
    cfl_predict_block(models[1], luma, width, v, v_stride)
}

/// Writes the § 7.13.5 CfL prediction `[dc, alpha, lumaAvg, max]` of a
/// `width`-wide block of luma `L` into `out`, row `r` at `out[r * stride..]`.
fn cfl_predict_block<T: ReconSample>(
    model: [i32; 4],
    luma: &[i16],
    width: usize,
    out: &mut [T],
    stride: usize,
) -> core::result::Result<(), GeneralIntraResidualError> {
    let height = luma.len() / width.max(1);
    if !((width == 4 && height.is_multiple_of(2)) || (width > 0 && width.is_multiple_of(8)))
        || (height.max(1) - 1) * stride + width > out.len()
    {
        return Err(GeneralIntraResidualError::InvalidReconstructionState {
            context: "CfL prediction target",
        });
    }
    #[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
    if cfl_alpha_fits_i16(model[1])
        && luma.len().is_multiple_of(16)
        && let Some(token) = luma_token()
    {
        if let Some(out) = T::u16_slice_mut(out) {
            cfl_predict_u16(token, model, luma, width, out, stride);
            return Ok(());
        }
        if model[3] == i32::from(u8::MAX)
            && let Some(out) = T::u8_slice_mut(out)
        {
            cfl_predict_u8(token, model, luma, width, out, stride);
            return Ok(());
        }
    }
    if let Some(out) = T::u16_slice_mut(out) {
        baseline_simd!(simd => cfl_predict_rows(simd, model, luma, width, out, stride));
    } else if model[3] <= i32::from(u8::MAX)
        && let Some(out) = T::u8_slice_mut(out)
    {
        baseline_simd!(simd => cfl_predict_rows(simd, model, luma, width, out, stride));
    } else {
        return Err(GeneralIntraResidualError::InvalidReconstructionState {
            context: "CfL sample storage",
        });
    }
    Ok(())
}

/// One § 7.13.5 CfL sample: `Clip1(dc + Round2Signed(alpha * (L - lumaAvg), 11))`.
#[cfg(test)]
fn cfl_predict([dc, alpha_q3, average_q3, max]: [i32; 4], luma: i32) -> i32 {
    (dc + splot_recon::math::round2_signed_i32(alpha_q3 * (luma - average_q3), CFL_ALPHA_SHIFT))
        .clamp(0, max)
}

/// The § 7.13.5 CfL samples of `luma` (rows 4 or a multiple of 8 wide) in
/// 8-lane groups.
/// `Round2Signed` rounds the magnitude, so each lane rounds `|alpha * L|` and
/// restores the sign.
#[inline]
fn cfl_predict_rows<S: Simd, P: CflLaneSample>(
    simd: S,
    model: [i32; 4],
    luma: &[i16],
    width: usize,
    out: &mut [P],
    stride: usize,
) {
    const LANES: usize = 8;
    let [dc, alpha_q3, average_q3, max] = model;
    let zero = i32x8::splat(simd, 0);
    let predict = |luma: &[i16]| {
        let (low, high) = simd.widen_i16x8(i16x8::from_slice(simd, &luma[..LANES]));
        let scaled = (simd.combine_i32x4(low, high) - average_q3) * alpha_q3;
        let rounded = (scaled.abs() + (1 << (CFL_ALPHA_SHIFT - 1))) >> CFL_ALPHA_SHIFT;
        let signed = scaled.simd_lt(zero).select(zero - rounded, rounded);
        (signed + dc).max(0).min(max)
    };
    if width == LANES / 2 && luma.len().is_multiple_of(LANES) {
        for (pair, luma) in luma.chunks_exact(LANES).enumerate() {
            let mut rows = [P::from_clipped(0); LANES];
            P::store8(simd, &mut rows, predict(luma));
            let start = 2 * pair * stride;
            out[start..start + width].copy_from_slice(&rows[..width]); // splot-copy-ok: publish a 4-wide CfL row
            out[start + stride..start + stride + width].copy_from_slice(&rows[width..]); // splot-copy-ok: publish a 4-wide CfL row
        }
        return;
    }
    for (row, luma) in luma.chunks_exact(width).enumerate() {
        let out = &mut out[row * stride..row * stride + width];
        for (out, luma) in out.chunks_exact_mut(LANES).zip(luma.chunks_exact(LANES)) {
            P::store8(simd, out, predict(luma));
        }
    }
}

/// The § 7.13.5 downsampled luma above and left of a CfL block (`t`, in
/// Q3), each sample gathered once for `lumaAvg` and `implicitAlpha`.
struct CflLumaEdges {
    above: [i16; MAX_CFL_AXIS],
    left: [i16; MAX_CFL_AXIS],
}

impl Default for CflLumaEdges {
    fn default() -> Self {
        Self {
            above: [0; MAX_CFL_AXIS],
            left: [0; MAX_CFL_AXIS],
        }
    }
}

impl CflLumaEdges {
    /// Gathers the edges and the `L` of a block into `luma`, returning where
    /// `L` starts: in one [`CflLuma::block_luma_420`] pass after the above
    /// row when the block crosses no 64-sample luma column clamp, else
    /// through [`Self::gather_above`] and [`CflLuma::fill_luma`].
    fn gather<S: Simd, P: CflLaneSample, const F: usize>(
        &mut self,
        simd: S,
        luma: &CflLuma<'_, P, F>,
        planes: &[CflPlane<'_>],
        block: CflBlock,
        luma_ac: &mut Vec<i16>,
    ) -> core::result::Result<usize, GeneralIntraResidualError> {
        let CflBlock {
            x,
            y,
            width,
            height,
            ..
        } = block;
        let has_left = x > 0 && planes.iter().any(|plane| plane.2.has_left());
        if y > 0
            && x % 32 + width <= 32
            && height <= MAX_CFL_AXIS
            && planes.iter().any(|plane| plane.2.has_above())
        {
            let min_y = cfl_above_min_luma_ref_y(y, block.sb_mib, luma.pixel_format);
            let rows = [3, 2, 1].map(|dy| (2 * y as isize - dy).max(min_y.unwrap_or(0)));
            let len = width * (height + 1);
            if luma_ac.len() < len {
                luma_ac.resize(len, 0);
            }
            let out = &mut luma_ac[..len];
            let mut left = has_left.then_some(&mut self.left[..height]);
            if luma.block_luma_420(x, y, width, [true; 3], Some(rows), &mut left, out) {
                self.above[..width].copy_from_slice(&out[..width]); // splot-copy-ok: lumaAbove from the block's luma pass
                return Ok(width);
            }
        }
        self.gather_above(simd, luma, planes, block)?;
        let left = has_left.then_some(&mut self.left[..height]);
        luma.fill_luma(simd, block, left, luma_ac)?;
        Ok(0)
    }

    /// Gathers the above edge when any plane in `planes` reads it; the left
    /// edge comes with the block's luma from [`CflLuma::fill_luma`].
    fn gather_above<S: Simd, P: CflLaneSample, const F: usize>(
        &mut self,
        simd: S,
        luma: &CflLuma<'_, P, F>,
        planes: &[CflPlane<'_>],
        block: CflBlock,
    ) -> core::result::Result<(), GeneralIntraResidualError> {
        let CflBlock { x, y, width, .. } = block;
        if width > MAX_CFL_AXIS || block.height > MAX_CFL_AXIS {
            return Err(GeneralIntraResidualError::InvalidReconstructionState {
                context: "CfL block size",
            });
        }
        if y > 0 && planes.iter().any(|plane| plane.2.has_above()) {
            let min_luma_ref_y = cfl_above_min_luma_ref_y(y, block.sb_mib, luma.pixel_format);
            let above = &mut self.above[..width];
            let below_sb_top = min_luma_ref_y.is_none_or(|min_y| min_y <= 2 * y as isize - 3);
            if !(below_sb_top
                && luma.block_luma_420(
                    x,
                    y - 1,
                    width,
                    [false, false, true],
                    None,
                    &mut None,
                    above,
                ))
            {
                let rows = luma.row(y - 1, false, min_luma_ref_y)?;
                luma.q3_run(simd, &rows, x, true, false, above);
            }
        }
        Ok(())
    }

    /// § 7.13.5 `lumaAvg`.
    fn average_q3(
        &self,
        block: CflBlock,
        neighbours: NeighbourAvailability,
    ) -> core::result::Result<i32, GeneralIntraResidualError> {
        let mut sum = 0u32;
        let mut count = 0u32;
        if neighbours.has_above() && block.y > 0 {
            sum += edge_sum(&self.above[..block.width]);
            count += (block.width / dc_step(block.width)) as u32;
        }
        if neighbours.has_left() && block.x > 0 {
            sum += edge_sum(&self.left[..block.height]);
            count += (block.height / dc_step(block.height)) as u32;
        }
        let bits = block.bit_depth.bits();
        if count == 0 {
            return Ok(i32::from(8u16 << (bits - 1)));
        }
        let max = (8u16 << bits).saturating_sub(1);
        Ok(i32::from(approx_divide(sum, count)?.min(max)))
    }

    /// [`Self::chroma_model`] from the edges of `chroma`, each sample past
    /// its right or bottom edge repeating the last.
    fn plane_chroma_model<T: ReconSample>(
        &self,
        chroma: &PlaneSamples<'_, T>,
        block: CflBlock,
        neighbours: NeighbourAvailability,
        derive: bool,
    ) -> core::result::Result<(i32, i32), GeneralIntraResidualError> {
        let CflBlock {
            x,
            y,
            width,
            height,
            ..
        } = block;
        let mut above = [T::default(); MAX_CFL_AXIS];
        let above = if neighbours.has_above() && y > 0 {
            let row = chroma.row_or_err(y - 1)?;
            let stored = row.get(x..).unwrap_or_default();
            if stored.len() >= width {
                Some(stored)
            } else {
                let Some(&last) = row.last() else {
                    return Err(CHROMA_EDGE_UNSTORED);
                };
                above[..stored.len()].copy_from_slice(stored); // splot-copy-ok: a chroma row past the frame edge
                above[stored.len()..width].fill(last);
                Some(&above[..])
            }
        } else {
            None
        };
        let mut left = [T::default(); MAX_CFL_AXIS];
        let left = if neighbours.has_left() && x > 0 {
            chroma.gather_column(x - 1, y, &mut left[..height])?;
            Some(&left[..])
        } else {
            None
        };
        let [model] = self.chroma_models(
            above.map(|above| [above]),
            left.map(|left| [left]),
            block,
            neighbours,
            derive,
        )?;
        Ok(model)
    }

    /// The § 7.13.5 DC prediction each of `N` CfL planes scales its luma
    /// around and, when `derive`, its `implicitAlpha`, from the planes'
    /// available `above` and `left` chroma edges; the luma terms of
    /// `implicitAlpha` are shared. Its edge lengths and sample counts are
    /// powers of two and its sums are non-negative, so its divisions are
    /// shifts: the sample step, and the sums over a count of 4 or 8.
    fn chroma_models<T: ReconSample, const N: usize>(
        &self,
        above: Option<[&[T]; N]>,
        left: Option<[&[T]; N]>,
        block: CflBlock,
        neighbours: NeighbourAvailability,
        derive: bool,
    ) -> core::result::Result<[(i32, i32); N], GeneralIntraResidualError> {
        let CflBlock { width, height, .. } = block;
        let (num_above, num_left) = match (derive, neighbours.has_above(), neighbours.has_left()) {
            (false, ..) => (0, 0),
            (true, true, true) if width > height.saturating_mul(2) => (NUM_REF_SAM_CFL, 0),
            (true, true, true) if height > width.saturating_mul(2) => (0, NUM_REF_SAM_CFL),
            (true, true, true) => (NUM_REF_SAM_CFL >> 1, NUM_REF_SAM_CFL >> 1),
            (true, above, left) => (
                if above { NUM_REF_SAM_CFL } else { 0 },
                if left { NUM_REF_SAM_CFL } else { 0 },
            ),
        };
        let (num_above, num_left) = (num_above.min(width), num_left.min(height));
        if (num_above > 0 && above.is_none()) || (num_left > 0 && left.is_none()) {
            return Err(GeneralIntraResidualError::InvalidReconstructionState {
                context: "CfL implicit alpha edge",
            });
        }
        let mut models = [(0, 0); N];
        for (plane, model) in models.iter_mut().enumerate() {
            let dc: T = splot_recon::predict_intra_dc_value_from_edge_sums(
                block.bit_depth,
                block.size,
                left.map(|left| sample_edge_sum(&left[plane][..height])),
                above.map(|above| sample_edge_sum(&above[plane][..width])),
            )?;
            model.0 = i32::from(dc.to_u16());
        }
        let count = (num_above + num_left) as i32;
        if count == 0 {
            return Ok(models);
        }
        let mut picked = [0i32; NUM_REF_SAM_CFL];
        let mut chroma = [[0i32; NUM_REF_SAM_CFL]; N];
        let mut lane = 0;
        for (luma, samples, len, num) in [
            (&self.above, above, width, num_above),
            (&self.left, left, height, num_left),
        ] {
            let (Some(samples), Some(step)) = (samples, len.checked_div(num)) else {
                continue;
            };
            for at in (step >> 1..len).step_by(step) {
                picked[lane] = i32::from(luma[at] >> 3);
                for (chroma, samples) in chroma.iter_mut().zip(samples) {
                    chroma[lane] = i32::from(samples[at].to_u16());
                }
                lane += 1;
            }
        }
        let alpha = baseline_simd!(simd => {
            let x = i32x8::from_slice(simd, &picked);
            let sums = |lanes: i32x8<_>| {
                let (low, high) = simd.split_i32x8(lanes);
                simd.reduce_sum_i32x4(low + high)
            };
            let y = chroma.map(|chroma| i32x8::from_slice(simd, &chroma));
            ([sums(x), sums(x * x)], y.map(|y| [sums(y), sums(x * y)]))
        });
        let ([sum_x, sum_xx], sums) = alpha;
        let divide = |n: i32| n >> count.trailing_zeros();
        let der = sum_xx - divide(sum_x * sum_x);
        for (model, [sum_y, sum_xy]) in models.iter_mut().zip(sums) {
            let nor = sum_xy - divide(sum_x * sum_y);
            if der != 0 && nor != 0 {
                model.1 = i32::from(resolve_division(nor, der, CFL_DERIVED_ALPHA_SHIFT));
            }
        }
        Ok(models)
    }
}

/// The § 7.13.2.10 edge sampling step: every second sample past 32.
const fn dc_step(len: usize) -> usize {
    if len > 32 { 2 } else { 1 }
}

/// The sum of an edge's samples at its [`dc_step`]; every sample is at most
/// `8 << 10`, so a 64-sample edge cannot overflow.
#[inline]
fn edge_sum<V: splot_recon::SummedSample>(edge: &[V]) -> u32 {
    splot_recon::sum_samples(edge, dc_step(edge.len()) == 2)
}

/// [`edge_sum`] of `u8` or `u16` sample storage.
#[inline]
fn sample_edge_sum<T: ReconSample>(edge: &[T]) -> u32 {
    T::u8_slice(edge)
        .map(edge_sum)
        .or_else(|| T::u16_slice(edge).map(edge_sum))
        .unwrap_or(0)
}

fn cfl_above_min_luma_ref_y(
    chroma_y: usize,
    sb_mib: usize,
    pixel_format: PixelFormat,
) -> Option<isize> {
    let luma_y = chroma_y << pixel_format.subsampling_y();
    let luma_mi_row = luma_y / MI_SIZE;
    let sb_height_luma = sb_mib.saturating_mul(MI_SIZE);
    let sb_start_luma_y = if sb_mib == 0 {
        0
    } else {
        luma_mi_row
            .checked_div(sb_mib)
            .map_or(0, |sb_row| sb_row.saturating_mul(sb_height_luma))
    };
    isize::try_from(sb_start_luma_y)
        .ok()
        .and_then(|sb_y| sb_y.checked_sub(1))
}

/// Where a block's § 7.13.6 `CflRef` buffers sit around it. Both chroma
/// planes of a block with the same neighbours share it.
#[derive(Clone, Copy)]
struct MhccpGeometry {
    reference_width: usize,
    reference_height: usize,
    above: usize,
    left: usize,
    min_chroma_ref_y: usize,
    min_luma_ref_y: Option<isize>,
}

impl MhccpGeometry {
    fn new<T: ReconSample>(
        workspace: &CurrentFrameWorkspace<T>,
        plane_id: PlaneId,
        block: CflBlock,
        neighbours: NeighbourAvailability,
    ) -> core::result::Result<Self, GeneralIntraResidualError> {
        let CflBlock {
            x,
            y,
            width,
            height,
            sb_mib,
            ..
        } = block;
        let pixel_format = workspace.info().pixel_format();
        let sub_x = usize::from(pixel_format.subsampling_x());
        let sub_y = usize::from(pixel_format.subsampling_y());
        let have_above = neighbours.has_above();
        let have_left = neighbours.has_left();
        let above = if have_above { y.min(2) } else { 0 };
        let left = if have_left { x.min(2) } else { 0 };
        let luma_mi_row = (y << sub_y) / MI_SIZE;
        let sb_start_luma_y = if sb_mib == 0 {
            0
        } else {
            luma_mi_row.checked_div(sb_mib).map_or(0, |sb_row| {
                sb_row.saturating_mul(sb_mib.saturating_mul(MI_SIZE))
            })
        };
        let extra_right = if have_above && width << sub_x > MI_SIZE {
            neighbours
                .num_above_right()
                .saturating_mul(MI_SIZE)
                .min(width)
        } else {
            0
        };
        let extra_bottom = if have_left && height << sub_y > MI_SIZE {
            neighbours
                .num_below_left()
                .saturating_mul(MI_SIZE)
                .min(height)
        } else {
            0
        };
        let chroma_size = workspace.plane(plane_id)?.storage_size();
        let frame_right = chroma_size.width().saturating_sub(x);
        let frame_bottom = chroma_size.height().saturating_sub(y);
        if frame_right == 0 || frame_bottom == 0 {
            return Err(GeneralIntraResidualError::InvalidReconstructionState {
                context: "MHCCP reference geometry",
            });
        }
        let reference_width = (left + width + extra_right)
            .min(128 >> sub_x)
            .min(left + frame_right);
        let reference_height = (above + height + extra_bottom)
            .min(128 >> sub_y)
            .min(above + frame_bottom);
        Ok(Self {
            reference_width,
            reference_height,
            above,
            left,
            min_chroma_ref_y: (sb_start_luma_y >> sub_y).saturating_sub(1),
            min_luma_ref_y: isize::try_from(sb_start_luma_y)
                .ok()
                .and_then(|sb_y| sb_y.checked_sub(1)),
        })
    }

    /// Where [`Self::gather_luma`] puts the luma references of `block` for
    /// direction `mh_dir`.
    fn layout(&self, block: CflBlock, mh_dir: u8) -> MhccpLayout {
        let (w, h) = (block.width, block.height);
        let (above, left) = (self.above, self.left);
        let strip_width = match (left, mh_dir) {
            (0, _) => 0,
            (_, 2) => left,
            _ => 1,
        };
        let strip_rows = h.max(self.reference_height.saturating_sub(above));
        let above_len = if above >= 2 {
            self.reference_width.saturating_sub(2)
        } else {
            0
        };
        let first_left_row = above.max(1);
        let left_len = if left >= 2 && self.reference_width >= 3 {
            (self.reference_height - 1).saturating_sub(first_left_row)
        } else {
            0
        };
        let first_col = usize::from(left > 0 && mh_dir != 2);
        let top_stride = self.reference_width.saturating_sub(first_col).max(1);
        let top = (2 * h + 1) * w;
        let strip = top + 2 * top_stride;
        let deinterleaved = strip + 1 + strip_rows * strip_width;
        MhccpLayout {
            mh_dir,
            width: w,
            height: h,
            top,
            top_stride,
            first_col,
            strip,
            strip_width,
            strip_rows,
            deinterleaved,
            above,
            above_len,
            first_left_row,
            left_len,
            len: deinterleaved + 2 * left_len,
        }
    }

    /// Captures the § 7.13.6 luma `CflRef[0]` into `arena` in Q3, as
    /// [`MhccpLayout`] places it. § 7.13.7 reads neither row 0 nor column 0
    /// of the edge as a centre: luma only as `v[0]` of direction 1 (row 0)
    /// or 2 (column 0), or as block samples when the edge is absent. The
    /// block's own rows and the left columns of the rows beside and below
    /// it each take one pass when their taps are stored.
    fn gather_luma<S: Simd, P: CflLaneSample, const F: usize>(
        &self,
        simd: S,
        luma_plane: &CflLuma<'_, P, F>,
        block: CflBlock,
        layout: &MhccpLayout,
        arena: &mut Vec<i16>,
    ) -> core::result::Result<(), GeneralIntraResidualError> {
        let (above, left, reference_width) = (self.above, self.left, self.reference_width);
        let MhccpLayout {
            mh_dir,
            width: w,
            height: h,
            top_stride: ts,
            first_col,
            strip_width: sw,
            ..
        } = *layout;
        if arena.len() < layout.len {
            arena.resize(layout.len, 0);
        }
        let (blocks, rest) = arena.split_at_mut(layout.top);
        let (top, rest) = rest.split_at_mut(2 * ts);
        let (strip, deinterleaved) = rest.split_at_mut(layout.deinterleaved - layout.strip);
        let rows_in = h.min(self.reference_height.saturating_sub(above));
        let row = |r: usize| luma_plane.row(block.y + r, above + r == 0, self.min_luma_ref_y);
        if reference_width > first_col {
            let first_row = usize::from(above > 0 && mh_dir != 1);
            for (r, out) in top
                .chunks_exact_mut(ts)
                .enumerate()
                .take(above)
                .skip(first_row)
            {
                let (y, x) = (block.y + r - above, block.x - left + first_col);
                let free = self
                    .min_luma_ref_y
                    .is_none_or(|min_y| min_y < 2 * y as isize);
                let clamps = [false, r == 0, first_col == 0];
                if !(free && luma_plane.block_luma_420(x, y, ts, clamps, None, &mut None, out)) {
                    let taps = luma_plane.row(y, r == 0, self.min_luma_ref_y)?;
                    luma_plane.q3_run(simd, &taps, x, first_col == 0, false, out);
                }
            }
        }
        let (head, shifted) = blocks.split_at_mut((h + 1) * w);
        let (ext, body) = head.split_at_mut(w);
        let cols_in = (left + w).min(reference_width) - left;
        if !(rows_in == h
            && cols_in == w
            && luma_plane.block_luma_420(
                block.x,
                block.y,
                w,
                [false, above == 0, left == 0],
                None,
                &mut None,
                body,
            ))
        {
            body.fill(0);
            for (r, out) in body.chunks_exact_mut(w).take(rows_in).enumerate() {
                luma_plane.q3_run(
                    simd,
                    &row(r)?,
                    block.x,
                    left == 0,
                    false,
                    &mut out[..cols_in],
                );
            }
        }
        if mh_dir == 1 {
            match above.checked_sub(1) {
                Some(r) => {
                    ext[..cols_in].copy_from_slice(&top[r * ts + left - first_col..][..cols_in]); // splot-copy-ok: v[0] of the block's first row
                    ext[cols_in..].fill(0);
                }
                None => ext.copy_from_slice(&body[..w]), // splot-copy-ok: v[0] of the block's first row
            }
        }
        let (slot, columns) = strip.split_at_mut(1);
        let columns = &mut columns[..layout.strip_rows * sw];
        if sw > 0 {
            let strip_in = self.reference_height.saturating_sub(above);
            let x0 = block.x - sw;
            let stored = &mut columns[..strip_in * sw];
            if !luma_plane.block_luma_420(
                x0,
                block.y,
                sw,
                [false, above == 0, sw == left],
                None,
                &mut None,
                stored,
            ) {
                for (r, out) in stored.chunks_exact_mut(sw).enumerate() {
                    luma_plane.q3_run(simd, &row(r)?, x0, sw == left, false, out);
                }
            }
            columns[strip_in * sw..].fill(0);
            if let Some(r) = above.checked_sub(1).filter(|_| mh_dir == 1) {
                slot[0] = top[r * ts + left - 1 - first_col];
            }
        }
        if mh_dir == 2 {
            for (r, (linear, centre)) in shifted
                .chunks_exact_mut(w)
                .zip(body.chunks_exact(w))
                .enumerate()
            {
                linear[0] = if sw > 0 {
                    columns[r * sw + sw - 1]
                } else {
                    centre[0]
                };
                linear[1..].copy_from_slice(&centre[..w - 1]); // splot-copy-ok: v[0] of a block row, one column left
            }
            let first = layout.first_left_row - above;
            let (linear, centre) = deinterleaved.split_at_mut(layout.left_len);
            let pairs = columns
                .get(first * sw..)
                .unwrap_or_default()
                .chunks_exact(sw.max(1));
            for ((linear, centre), pair) in linear.iter_mut().zip(centre).zip(pairs) {
                (*linear, *centre) = (pair[0], pair[sw - 1]);
            }
        }
        Ok(())
    }

    /// Captures the § 7.13.6 chroma `CflRef[1]` of `plane_id` that § 7.13.7
    /// reads into `chroma`: the above edge, then the left edge, each as
    /// [`MhccpLayout`] counts them.
    fn gather_chroma<T: ReconSample>(
        &self,
        workspace: &CurrentFrameWorkspace<T>,
        plane_id: PlaneId,
        block: CflBlock,
        layout: &MhccpLayout,
        chroma: &mut Vec<u16>,
    ) -> core::result::Result<(), GeneralIntraResidualError> {
        let (above_len, left_len) = (layout.above_len, layout.left_len);
        if chroma.len() < above_len + left_len {
            chroma.resize(above_len + left_len, 0);
        }
        let (above, left) = chroma.split_at_mut(above_len);
        let plane = PlaneSamples::of(workspace, plane_id)?;
        let last_y = plane.height - 1;
        if above_len > 0 {
            let ref_chroma_y = (block.y + 1 - self.above).max(self.min_chroma_ref_y);
            let source_row = plane.row_or_err(ref_chroma_y.min(last_y))?;
            let fill = source_row[source_row.len() - 1].to_u16();
            let source = source_row
                .get(block.x + 1 - self.left..)
                .unwrap_or_default();
            let copied = source.len().min(above_len);
            for (slot, sample) in above.iter_mut().zip(&source[..copied]) {
                *slot = sample.to_u16();
            }
            above[copied..].fill(fill);
        }
        if left_len > 0 {
            let first = layout.first_left_row;
            let start_y = (block.y + first - self.above).min(last_y);
            let end_y = (block.y + first + left_len - 1 - self.above).min(last_y);
            let (column, stride, _) = plane.column(block.x - 1, start_y, end_y - start_y + 1)?;
            for (slot, row) in left[..left_len].iter_mut().zip(first..) {
                let y = (block.y + row - self.above).min(last_y) - start_y;
                *slot = column[y * stride].to_u16();
            }
        }
        Ok(())
    }
}

/// Where the § 7.13.6 luma references of an MHCCP block sit in its arena,
/// all in Q3: `(h + 1) * w` samples of the `v[0]` row of direction 1 above
/// the `h` centre rows of the block, then `h * w` samples of direction 2's
/// `v[0]` rows, the two edge rows `CflRef[0][0..2]`, one free sample and the
/// `strip_rows` rows of the `strip_width` columns left of the block, and for
/// direction 2 the left edge's `v[0]` and centres apart.
#[derive(Clone, Copy)]
struct MhccpLayout {
    mh_dir: u8,
    width: usize,
    height: usize,
    top: usize,
    /// The edge rows hold columns `first_col..reference_width`.
    top_stride: usize,
    first_col: usize,
    strip: usize,
    strip_width: usize,
    strip_rows: usize,
    deinterleaved: usize,
    /// `CflAbove`, and the § 7.13.7 samples of the above edge (row 1 from
    /// column 1) and the left edge (column 1 from row `first_left_row`).
    above: usize,
    above_len: usize,
    first_left_row: usize,
    left_len: usize,
    len: usize,
}

impl MhccpLayout {
    /// The block's `v[0]` and centre luma, and the § 7.13.7 edges.
    fn views<'a>(&self, arena: &'a [i16]) -> ([&'a [i16]; 2], [MhccpEdge<'a>; 2]) {
        let (w, h, ts) = (self.width, self.height, self.top_stride);
        let centre = &arena[w..(h + 1) * w];
        let linear = match self.mh_dir {
            0 => centre,
            1 => &arena[..h * w],
            _ => &arena[(h + 1) * w..(2 * h + 1) * w],
        };
        let top = &arena[self.top..self.top + 2 * ts];
        let (n, fc) = (self.above_len, self.first_col);
        let above = MhccpEdge {
            centre: &top[ts + 1 - fc..][..n],
            linear: match self.mh_dir {
                0 => &top[ts + 1 - fc..][..n],
                1 => &top[1 - fc..][..n],
                _ => &top[ts..][..n],
            },
        };
        let first = self.first_left_row - self.above;
        let left = match self.mh_dir {
            _ if self.left_len == 0 => MhccpEdge::default(),
            2 => {
                let edge = &arena[self.deinterleaved..][..2 * self.left_len];
                let (linear, centre) = edge.split_at(self.left_len);
                MhccpEdge { linear, centre }
            }
            dir => MhccpEdge {
                centre: &arena[self.strip + 1 + first..][..self.left_len],
                linear: &arena[self.strip + first + usize::from(dir == 0)..][..self.left_len],
            },
        };
        ([linear, centre], [above, left])
    }
}

/// One plane of the current frame, resolved once for many row reads.
struct PlaneSamples<'a, T> {
    samples: &'a [T],
    origin_y: usize,
    stored_rows: usize,
    width: usize,
    stride: usize,
    height: usize,
}

impl<'a, T> PlaneSamples<'a, T> {
    fn new<W: ReconSample>(
        workspace: &'a CurrentFrameWorkspace<W>,
        samples: &'a [T],
        plane_id: PlaneId,
    ) -> core::result::Result<Self, GeneralIntraResidualError> {
        let plane = workspace.plane(plane_id)?;
        let size = plane.storage_size();
        let stride = plane.stride_samples().max(1);
        if size.width() == 0 || size.height() == 0 {
            return Err(GeneralIntraResidualError::InvalidReconstructionState {
                context: "CfL plane size",
            });
        }
        Ok(Self {
            samples,
            origin_y: plane.origin_y(),
            stored_rows: samples.len() / stride,
            width: size.width(),
            stride,
            height: size.height(),
        })
    }

    /// Stored row `y`, which must be inside the plane.
    #[inline]
    fn row_or_err(&self, y: usize) -> core::result::Result<&'a [T], GeneralIntraResidualError> {
        const UNSTORED: GeneralIntraResidualError =
            GeneralIntraResidualError::InvalidReconstructionState {
                context: "CfL reference row",
            };
        let Some(local) = y.checked_sub(self.origin_y) else {
            return Err(UNSTORED);
        };
        if y >= self.height || local >= self.stored_rows {
            return Err(UNSTORED);
        }
        let start = local * self.stride;
        match self.samples.get(start..start + self.width) {
            Some(row) => Ok(row),
            None => Err(UNSTORED),
        }
    }

    /// Column `x` of the `rows` stored rows from `y`: the samples from
    /// `(x, y)` on, the row stride, and `rows`, which must be at least one.
    #[inline]
    fn column(
        &self,
        x: usize,
        y: usize,
        rows: usize,
    ) -> core::result::Result<(&'a [T], usize, usize), GeneralIntraResidualError> {
        const UNSTORED: GeneralIntraResidualError =
            GeneralIntraResidualError::InvalidReconstructionState {
                context: "CfL reference column",
            };
        let Some(local) = y.checked_sub(self.origin_y) else {
            return Err(UNSTORED);
        };
        if rows == 0 || x >= self.width || y + rows > self.height || local + rows > self.stored_rows
        {
            return Err(UNSTORED);
        }
        let start = local * self.stride + x;
        match self
            .samples
            .get(start..start + (rows - 1) * self.stride + 1)
        {
            Some(column) => Ok((column, self.stride, rows)),
            None => Err(UNSTORED),
        }
    }
}

impl<'a, T: ReconSample> PlaneSamples<'a, T> {
    /// Column `x` from row `y` into `out`, each sample past the plane's
    /// bottom edge repeating its last.
    #[inline]
    fn gather_column(
        &self,
        x: usize,
        y: usize,
        out: &mut [T],
    ) -> core::result::Result<(), GeneralIntraResidualError> {
        let rows = out.len().min(self.height - y.min(self.height - 1));
        let (column, stride, rows) = self.column(x, y, rows)?;
        let (stored, past) = out.split_at_mut(rows);
        for (i, slot) in stored.iter_mut().enumerate() {
            *slot = column[i * stride];
        }
        past.fill(stored[rows - 1]);
        Ok(())
    }

    fn of(
        workspace: &'a CurrentFrameWorkspace<T>,
        plane_id: PlaneId,
    ) -> core::result::Result<Self, GeneralIntraResidualError> {
        Self::new(workspace, workspace.plane(plane_id)?.samples(), plane_id)
    }
}

/// The luma plane of one CfL block and its § 7.13.5 downsampling filter:
/// the 4:2:0 filter `F`, or [`RUNTIME_WEIGHTS`] for any format.
struct CflLuma<'a, P, const F: usize> {
    plane: PlaneSamples<'a, P>,
    pixel_format: PixelFormat,
    sub_x: usize,
    sub_y: usize,
    /// `weights[dy + 1][dx + 1]`, zero off the subsampled axes.
    weights: [[i32; 3]; 3],
}

impl<'a, P: CflLaneSample, const F: usize> CflLuma<'a, P, F> {
    fn new<T: ReconSample>(
        workspace: &'a CurrentFrameWorkspace<T>,
        samples: &'a [P],
        filter: usize,
    ) -> core::result::Result<Self, GeneralIntraResidualError> {
        let pixel_format = workspace.info().pixel_format();
        let sub_x = usize::from(pixel_format.subsampling_x());
        let sub_y = usize::from(pixel_format.subsampling_y());
        let weights = match (sub_x, sub_y) {
            (1, 1) => CFL_FILTERS_420[filter],
            (1, _) => [[0; 3], CFL_FILTERS_422[filter], [0; 3]],
            _ => [[0; 3], [0, 8, 0], [0; 3]],
        };
        if F != RUNTIME_WEIGHTS && (sub_x, sub_y, F) != (1, 1, filter) {
            return Err(GeneralIntraResidualError::InvalidReconstructionState {
                context: "CfL filter specialisation",
            });
        }
        Ok(Self {
            plane: PlaneSamples::new(workspace, samples, PlaneId::Y)?,
            pixel_format,
            sub_x,
            sub_y,
            weights,
        })
    }

    /// The taps, constant for a 4:2:0 filter.
    #[inline]
    fn weights(&self) -> [[i32; 3]; 3] {
        if F < RUNTIME_WEIGHTS {
            CFL_FILTERS_420[F]
        } else {
            self.weights
        }
    }

    /// The luma rows the taps of chroma row `chroma_y` read; a row with no
    /// tap aliases the middle row.
    #[inline]
    fn row(
        &self,
        chroma_y: usize,
        clamp_y: bool,
        min_luma_ref_y: Option<isize>,
    ) -> core::result::Result<[&'a [P]; 3], GeneralIntraResidualError> {
        let max_y = self.plane.height - 1;
        let luma_y = clamped_cfl_luma_coordinate(chroma_y, self.sub_y, max_y) as isize;
        let tap_row = |dy: isize| {
            let mut sy = luma_y + if clamp_y { dy.max(0) } else { dy };
            if let Some(min_y) = min_luma_ref_y {
                sy = sy.max(min_y);
            }
            self.plane.row_or_err(sy.clamp(0, max_y as isize) as usize)
        };
        let middle = tap_row(0)?;
        let weights = self.weights();
        let edge = |dy: isize, weights: [i32; 3]| {
            if weights == [0; 3] {
                Ok(middle)
            } else {
                tap_row(dy)
            }
        };
        Ok([edge(-1, weights[0])?, middle, edge(1, weights[2])?])
    }

    /// The § 7.13.5 downsampled luma sample at `chroma_x` of `rows`, in Q3.
    #[cfg(test)]
    fn q3(&self, rows: &[&[P]; 3], chroma_x: usize, clamp_x: bool) -> i32 {
        let max_x = self.plane.width - 1;
        let luma_x = clamped_cfl_luma_coordinate(chroma_x, self.sub_x, max_x);
        let columns = [
            if clamp_x {
                luma_x
            } else {
                luma_x.saturating_sub(1)
            },
            luma_x,
            (luma_x + 1).min(max_x),
        ];
        let mut total = 0;
        for (row, weights) in rows.iter().zip(&self.weights()) {
            for (&column, &weight) in columns.iter().zip(weights) {
                if weight != 0 {
                    total += weight * i32::from(row[column].into());
                }
            }
        }
        total
    }

    /// `out[i] = q3(chroma_x0 + i)` in 8-lane groups: the left tap clamps at
    /// the first sample when `clamp_first`, and on every 64th luma column
    /// when `periodic`. Columns past the plane's last chroma column repeat
    /// it, as its clamped luma coordinate does.
    #[inline]
    fn q3_run<S: Simd>(
        &self,
        simd: S,
        rows: &[&[P]; 3],
        chroma_x0: usize,
        clamp_first: bool,
        periodic: bool,
        out: &mut [i16],
    ) {
        let sub_x = if F < RUNTIME_WEIGHTS { 1 } else { self.sub_x };
        let edge = (self.plane.width - 1) >> sub_x;
        let inside = (edge + 1).saturating_sub(chroma_x0).min(out.len());
        let (stored, past) = out.split_at_mut(inside);
        for (g, group) in stored.chunks_mut(8).enumerate() {
            let lanes = self.q3_group(
                simd,
                rows,
                chroma_x0 + 8 * g,
                clamp_first && g == 0,
                periodic,
            );
            if let Ok(group) = <&mut [i16; 8]>::try_from(&mut *group) {
                lanes.store_slice(group);
            } else {
                store_partial(lanes.into(), group);
            }
        }
        if past.is_empty() {
            return;
        }
        let clamp = periodic && (edge << sub_x).is_multiple_of(64);
        let at_edge = |clamp: bool| self.q3_group(simd, rows, edge, clamp, false).as_slice()[0];
        let from = usize::from(inside == 0);
        if inside == 0 {
            past[0] = at_edge(clamp_first || clamp);
        }
        let last = match stored.last() {
            Some(&last) if inside > 1 || !clamp_first || clamp => last,
            _ => at_edge(clamp),
        };
        past[from..].fill(last);
    }

    /// Eight [`Self::q3`] samples of chroma columns `x..x + 8`, the first one
    /// clamped when `clamp_first` and every 64th luma column when `periodic`;
    /// taps past the end of a row read its last sample.
    #[allow(
        clippy::inline_always,
        reason = "measured: out of line it returns through memory"
    )]
    #[inline(always)]
    fn q3_group<S: Simd>(
        &self,
        simd: S,
        rows: &[&[P]; 3],
        x: usize,
        clamp_first: bool,
        periodic: bool,
    ) -> i16x8<S> {
        let sub_x = if F < RUNTIME_WEIGHTS { 1 } else { self.sub_x };
        let start = x << sub_x;
        let lead = usize::from(sub_x == 1);
        let row_taps = |row: &[P], weights: [i32; 3]| {
            let zero = u16x8::splat(simd, 0);
            if weights == [0; 3] {
                return zero;
            }
            let padded;
            let taps = if let Some(taps) = start
                .checked_sub(lead)
                .and_then(|from| row.get(from..))
                .and_then(<[P]>::first_chunk::<17>)
            {
                taps
            } else {
                padded = padded_taps(row, start, lead);
                &padded
            };
            if lead == 0 {
                return P::load_halves(simd, taps).0 * weights[1] as u16;
            }
            let (low, high) = P::load_halves(simd, &taps[1..]);
            let centre = simd.unzip_low_u16x8(low, high);
            let mut acc = centre * weights[1] as u16;
            if weights[2] != 0 {
                acc += simd.unzip_high_u16x8(low, high) * weights[2] as u16;
            }
            if weights[0] != 0 {
                let (low, high) = P::load_halves(simd, taps);
                let left = simd.unzip_low_u16x8(low, high);
                let left = if clamp_first || periodic || x == 0 {
                    let index = u16x8::from_slice(simd, &[0, 1, 2, 3, 4, 5, 6, 7]) + x as u16;
                    let first = if clamp_first || x == 0 {
                        x as u16
                    } else {
                        u16::MAX
                    };
                    let mut clamped = index.simd_eq(u16x8::splat(simd, first));
                    if periodic {
                        clamped |= (index & 31).simd_eq(zero);
                    }
                    clamped.select(centre, left)
                } else {
                    left
                };
                acc += left * weights[0] as u16;
            }
            acc
        };
        let [top, middle, bottom] = self.weights();
        let acc = row_taps(rows[0], top) + row_taps(rows[1], middle) + row_taps(rows[2], bottom);
        acc.bitcast()
    }

    /// The 8-bit 4:2:0 downsampled luma of the chroma rectangle at `(x, y)`,
    /// `width` wide, in one AVX2 pass when every tap lies in the stored
    /// plane: `[periodic, clamp_row0, clamp_col0]` as in [`Luma420Block`].
    /// With `above`, the luma tap rows of a row above the rectangle, `out`
    /// starts with that row. `false` leaves `out` and `left` unwritten.
    #[allow(
        clippy::too_many_arguments,
        clippy::inline_always,
        reason = "measured: out of line it costs more than the call it saves"
    )]
    #[inline(always)]
    fn block_luma_420(
        &self,
        x: usize,
        y: usize,
        width: usize,
        clamps: [bool; 3],
        above: Option<[isize; 3]>,
        left: &mut Option<&mut [i16]>,
        out: &mut [i16],
    ) -> bool {
        let height = (out.len() / width.max(1)).saturating_sub(usize::from(above.is_some()));
        #[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
        if F < RUNTIME_WEIGHTS
            && width > 0
            && height > 0
            && let Some(token) = luma_token()
            && let Some(samples) = P::as_u8(self.plane.samples)
            && 2 * (x + width) <= self.plane.width
            && 2 * (y + height) <= self.plane.height
            && let Some(local_y) = (2 * y).checked_sub(self.plane.origin_y)
        {
            let [periodic, clamp_row0, clamp_col0] = clamps;
            let tap = |row: isize| {
                let local = usize::try_from(row)
                    .ok()?
                    .checked_sub(self.plane.origin_y)?;
                Some(local * self.plane.stride + 2 * x)
            };
            let above = match above {
                Some([top, mid, bottom]) => match (tap(top), tap(mid), tap(bottom)) {
                    (Some(top), Some(mid), Some(bottom)) => Some([top, mid, bottom]),
                    _ => return false,
                },
                None => None,
            };
            let block = Luma420Block {
                origin: local_y * self.plane.stride + 2 * x,
                stride: self.plane.stride,
                luma_x: 2 * x,
                luma_y: 2 * y,
                width,
                periodic,
                clamp_row0,
                clamp_col0,
                above,
            };
            if block.fits(height, samples.len()) {
                return luma_420::<F>(token, samples, block, left.as_deref_mut(), out);
            }
        }
        #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
        let _ = (self, x, y, height, clamps, above, left, out);
        false
    }

    /// The § 7.13.5 `L` of a block into `luma`, and the downsampled luma left
    /// of each block row into `left` when given.
    fn fill_luma<S: Simd>(
        &self,
        simd: S,
        block: CflBlock,
        mut left: Option<&mut [i16]>,
        luma: &mut Vec<i16>,
    ) -> core::result::Result<(), GeneralIntraResidualError> {
        let CflBlock {
            x,
            y,
            width,
            height,
            ..
        } = block;
        if luma.len() < width * height {
            luma.resize(width * height, 0);
        }
        let luma = &mut luma[..width * height];
        if self.block_luma_420(x, y, width, [true; 3], None, &mut left, luma) {
            return Ok(());
        }
        let max_x = self.plane.width - 1;
        let max_y = self.plane.height - 1;
        let column = clamped_cfl_luma_coordinate(x.saturating_sub(1), self.sub_x, max_x);
        let columns = [column.saturating_sub(1), column, (column + 1).min(max_x)];
        let weights = self.weights();
        let mut taps = [[[0u16; 8]; 3]; 3];
        for (row, out) in luma.chunks_exact_mut(width).enumerate() {
            let luma_y = clamped_cfl_luma_coordinate(y + row, self.sub_y, max_y);
            let clamp_y = row == 0 || luma_y.is_multiple_of(64);
            let rows = self.row(y + row, clamp_y, None)?;
            self.q3_run(simd, &rows, x, true, true, out);
            let Some(left) = left.as_deref_mut() else {
                continue;
            };
            let left_rows = if clamp_y && row > 0 && weights[0] != [0; 3] {
                self.row(y + row, false, None)?
            } else {
                rows
            };
            for ((taps, tap_row), weights) in taps.iter_mut().zip(left_rows).zip(weights) {
                for ((taps, &column), weight) in taps.iter_mut().zip(&columns).zip(weights) {
                    if weight != 0 {
                        taps[row % 8] = tap_row[column].into();
                    }
                }
            }
            if row % 8 == 7 || row + 1 == height {
                let group = &mut left[row - row % 8..=row];
                group.copy_from_slice(&gathered_q3(&taps, weights)[..group.len()]); // splot-copy-ok: publish CfL lumaLeft samples
            }
        }
        Ok(())
    }
}

/// The § 7.13.5 Q3 sums of eight samples whose taps were gathered into
/// `taps[dy + 1][dx + 1]`, weighted by `weights[dy + 1][dx + 1]`, in lanes.
pub(super) fn gathered_q3(taps: &[[[u16; 8]; 3]; 3], weights: [[i32; 3]; 3]) -> [i16; 8] {
    baseline_simd!(simd => {
        let mut acc = u16x8::splat(simd, 0);
        for (taps, weights) in taps.iter().zip(weights) {
            for (taps, weight) in taps.iter().zip(weights) {
                if weight != 0 {
                    acc += u16x8::from_slice(simd, taps) * weight as u16;
                }
            }
        }
        acc.bitcast::<i16x8<_>>().into()
    })
}

/// A luma sample type the CfL lane forms load as `u16` lanes.
trait CflLaneSample: PredictedSample + Into<u16> {
    /// Loads `row[..16]` as two halves of eight.
    fn load_halves<S: Simd>(simd: S, row: &[Self]) -> (u16x8<S>, u16x8<S>);
    /// `samples` as bytes, for 8-bit planes.
    #[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
    fn as_u8(samples: &[Self]) -> Option<&[u8]>;
}

impl CflLaneSample for u8 {
    #[inline]
    fn load_halves<S: Simd>(simd: S, row: &[Self]) -> (u16x8<S>, u16x8<S>) {
        simd.widen_u8x16(fearless_simd::u8x16::from_slice(simd, &row[..16]))
    }

    #[inline]
    #[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
    fn as_u8(samples: &[Self]) -> Option<&[u8]> {
        Some(samples)
    }
}

impl CflLaneSample for u16 {
    #[inline]
    fn load_halves<S: Simd>(simd: S, row: &[Self]) -> (u16x8<S>, u16x8<S>) {
        (
            u16x8::from_slice(simd, &row[..8]),
            u16x8::from_slice(simd, &row[8..16]),
        )
    }

    #[inline]
    #[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
    fn as_u8(_samples: &[Self]) -> Option<&[u8]> {
        None
    }
}

/// The first `out.len()` (below 8) of `lanes`, in fixed-size moves.
#[inline]
fn store_partial(lanes: [i16; 8], out: &mut [i16]) {
    let mut at = 0;
    for size in [4, 2, 1] {
        if out.len() - at >= size {
            out[at..at + size].copy_from_slice(&lanes[at..at + size]); // splot-copy-ok: publish CfL luma samples
            at += size;
        }
    }
}

/// The 17 taps of a [`CflLuma::q3_group`] from `row[start - lead]` when the
/// row does not hold them: the samples past its end repeat its last one, and
/// a tap left of its first sample reads that sample.
#[cold]
fn padded_taps<P: Copy>(row: &[P], start: usize, lead: usize) -> [P; 17] {
    let mut padded = [row[row.len() - 1]; 17];
    let stored = &row[start..row.len().min(start + 16)];
    padded[lead..lead + stored.len()].copy_from_slice(stored); // splot-copy-ok: pad a CfL luma row end
    padded[0] = row[start.saturating_sub(lead)];
    padded
}

fn clamped_cfl_luma_coordinate(
    chroma_coordinate: usize,
    subsampling: usize,
    max_luma_coordinate: usize,
) -> usize {
    chroma_coordinate.min(max_luma_coordinate >> subsampling) << subsampling
}

const fn cfl_filter_index(cfl_ds_filter_index: u8) -> Option<usize> {
    match cfl_ds_filter_index {
        0 | 3 => Some(0),
        1 => Some(1),
        2 => Some(2),
        _ => None,
    }
}

#[cfg(test)]
#[path = "cfl_tests.rs"]
mod tests;
