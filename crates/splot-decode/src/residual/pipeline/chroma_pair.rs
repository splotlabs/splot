// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Paired chroma residual reconstruction.

use splot_recon::{CurrentFrameWorkspace, PlaneId, PlaneRect, ReconSample};

use crate::bitstream::tile_payload::{
    CoeffBlock, DecodeTileWorkUnit, GeneralIntraResidualError, LumaCoeffBlock,
    LumaTransformTypeContext, TileBlockDecodedState, current_frame_qm_segment_id,
    is_cctx_geometry_allowed, reconstruct_general_intra_chroma_cctx_pair_into,
};
use crate::prediction::chroma::cfl::reconstruct_general_intra_chroma_cfl_pair_into;

use super::{ResidualPlanePlan, ResidualReconstructionPlan};

pub(super) fn can_hold_for_cctx_pair(
    plane: ResidualPlanePlan,
    work_unit: &DecodeTileWorkUnit<'_>,
) -> bool {
    plane.plane_id == PlaneId::U
        && !plane.defer_reconstruction
        && matches!(
            work_unit
                .coeff_frame_facts()
                .lossless_for_segment(current_frame_qm_segment_id()),
            Some(false)
        )
}

pub(super) fn cctx_allowed(plane: ResidualPlanePlan) -> bool {
    if plane.plane_id == PlaneId::Y {
        return false;
    }
    let (sub_x, sub_y) = plane.block_ctx.chroma().subsampling(plane.plane_id);
    let block = plane.block_ctx.plane_block(plane.plane_id);
    is_cctx_geometry_allowed(
        sub_x != 0 && sub_y != 0,
        block.width4() * 4,
        block.height4() * 4,
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn reconstruct_chroma_pair_or_planes<T: ReconSample>(
    scratch: &mut crate::pipeline::general_intra::GeneralIntraReconScratch<T>,
    workspace: &mut CurrentFrameWorkspace<T>,
    block_decoded: &TileBlockDecodedState,
    u: &(ResidualPlanePlan, CoeffBlock<'_>),
    v: Option<&(ResidualPlanePlan, CoeffBlock<'_>)>,
    qindex: u32,
    intra_edge: crate::prediction::intra_edge::IntraEdgeCtx,
    luma_context: LumaTransformTypeContext,
) -> core::result::Result<(), GeneralIntraResidualError> {
    let (u_plane, u_coeffs) = *u;
    let cctx_type = u_coeffs.cctx_type.unwrap_or(0);
    let Some(&(v_plane, v_coeffs)) = v else {
        if cctx_type != 0 {
            return Err(GeneralIntraResidualError::InvalidReconstructionState {
                context: "CCTX paired V plane",
            });
        }
        return u_plane.reconstruct(
            scratch,
            workspace,
            u_coeffs,
            block_decoded,
            None,
            qindex,
            intra_edge,
            luma_context,
        );
    };
    if cctx_type == 0 {
        if reconstruct_cfl_pair(
            scratch,
            workspace,
            block_decoded,
            &(u_plane, u_coeffs),
            &(v_plane, v_coeffs),
            qindex,
        )? {
            return Ok(());
        }
        u_plane.reconstruct(
            scratch,
            workspace,
            u_coeffs,
            block_decoded,
            None,
            qindex,
            intra_edge,
            luma_context,
        )?;
        return v_plane.reconstruct(
            scratch,
            workspace,
            v_coeffs,
            block_decoded,
            None,
            qindex,
            intra_edge,
            luma_context,
        );
    }
    reconstruct_chroma_cctx_pair(
        scratch,
        workspace,
        block_decoded,
        &(u_plane, u_coeffs),
        &(v_plane, v_coeffs),
        qindex,
        cctx_type,
        intra_edge,
        luma_context,
    )
}

fn reconstruct_cfl_pair<T: ReconSample>(
    scratch: &mut crate::pipeline::general_intra::GeneralIntraReconScratch<T>,
    workspace: &mut CurrentFrameWorkspace<T>,
    block_decoded: &TileBlockDecodedState,
    u: &(ResidualPlanePlan, CoeffBlock<'_>),
    v: &(ResidualPlanePlan, CoeffBlock<'_>),
    qindex: u32,
) -> core::result::Result<bool, GeneralIntraResidualError> {
    let (u_plane, u_coeffs) = *u;
    let (v_plane, v_coeffs) = *v;
    let (
        ResidualReconstructionPlan::ChromaCfl {
            params: u_params,
            cfl_ds_filter_index: u_filter,
            sb_mib: u_sb_mib,
        },
        ResidualReconstructionPlan::ChromaCfl {
            params: v_params,
            cfl_ds_filter_index: v_filter,
            sb_mib: v_sb_mib,
        },
    ) = (u_plane.reconstruction, v_plane.reconstruction)
    else {
        return Ok(false);
    };
    if u_plane.plane_id != PlaneId::U
        || v_plane.plane_id != PlaneId::V
        || u_plane.x != v_plane.x
        || u_plane.y != v_plane.y
        || u_plane.tx != v_plane.tx
        || u_params != v_params
        || u_filter != v_filter
        || u_sb_mib != v_sb_mib
    {
        return Ok(false);
    }

    let u_neighbours = u_plane.plane_neighbours(u_plane.block_ctx, block_decoded);
    let v_neighbours = v_plane.plane_neighbours(v_plane.block_ctx, block_decoded);
    reconstruct_general_intra_chroma_cfl_pair_into(
        scratch,
        workspace,
        u_coeffs,
        v_coeffs,
        u_plane.x,
        u_plane.y,
        u_plane.tx.width_log2(),
        u_plane.tx.height_log2(),
        qindex,
        u_params,
        u_filter,
        u_sb_mib,
        u_neighbours,
        v_neighbours,
        u_plane.block_ctx.bit_depth(),
    )?;
    Ok(true)
}

#[allow(clippy::too_many_arguments)]
fn reconstruct_chroma_cctx_pair<T: ReconSample>(
    scratch: &mut crate::pipeline::general_intra::GeneralIntraReconScratch<T>,
    workspace: &mut CurrentFrameWorkspace<T>,
    block_decoded: &TileBlockDecodedState,
    u: &(ResidualPlanePlan, CoeffBlock<'_>),
    v: &(ResidualPlanePlan, CoeffBlock<'_>),
    qindex: u32,
    cctx_type: usize,
    intra_edge: crate::prediction::intra_edge::IntraEdgeCtx,
    luma_context: LumaTransformTypeContext,
) -> core::result::Result<(), GeneralIntraResidualError> {
    let (u_plane, u_coeffs) = *u;
    let (v_plane, v_coeffs) = *v;
    if u_plane.plane_id != PlaneId::U
        || v_plane.plane_id != PlaneId::V
        || u_plane.x != v_plane.x
        || u_plane.y != v_plane.y
        || u_plane.tx != v_plane.tx
    {
        return Err(GeneralIntraResidualError::InvalidReconstructionState {
            context: "CCTX paired plane geometry",
        });
    }
    let u_prediction_block = prediction_only_coeff_block(u_coeffs);
    u_plane.reconstruct(
        scratch,
        workspace,
        CoeffBlock::new(&u_prediction_block, &[])?,
        block_decoded,
        None,
        qindex,
        intra_edge,
        luma_context,
    )?;
    read_plane_prediction(workspace, u_plane, &mut scratch.cctx_mut().u_prediction)?;
    let v_prediction_block = prediction_only_coeff_block(v_coeffs);
    v_plane.reconstruct(
        scratch,
        workspace,
        CoeffBlock::new(&v_prediction_block, &[])?,
        block_decoded,
        None,
        qindex,
        intra_edge,
        luma_context,
    )?;
    let cctx = scratch.cctx_mut();
    read_plane_prediction(workspace, v_plane, &mut cctx.v_prediction)?;
    reconstruct_general_intra_chroma_cctx_pair_into(
        u_coeffs,
        &cctx.u_prediction,
        v_coeffs,
        &cctx.v_prediction,
        qindex,
        u_plane.tx.width_log2(),
        u_plane.tx.height_log2(),
        cctx_type,
        false,
        u_plane.block_ctx.bit_depth(),
        &mut cctx.u_output,
        &mut cctx.v_output,
    )?;
    write_plane_block(workspace, u_plane, &cctx.u_output)?;
    write_plane_block(workspace, v_plane, &cctx.v_output)?;
    Ok(())
}

fn prediction_only_coeff_block(coeffs: CoeffBlock<'_>) -> LumaCoeffBlock {
    LumaCoeffBlock::empty(coeffs.plane_tx_type, coeffs.lossless)
}

fn plane_rect(
    plane: ResidualPlanePlan,
) -> core::result::Result<(PlaneRect, usize), GeneralIntraResidualError> {
    let width = 1usize << plane.tx.width_log2();
    let height = 1usize << plane.tx.height_log2();
    Ok((PlaneRect::new(plane.x, plane.y, width, height)?, width))
}

fn read_plane_prediction<T: ReconSample>(
    workspace: &CurrentFrameWorkspace<T>,
    plane: ResidualPlanePlan,
    out: &mut Vec<T>,
) -> core::result::Result<(), GeneralIntraResidualError> {
    let (rect, width) = plane_rect(plane)?;
    let expected = width.checked_mul(rect.height()).ok_or(
        GeneralIntraResidualError::InvalidReconstructionState {
            context: "CCTX prediction sample count",
        },
    )?;
    out.clear();
    out.reserve(expected);
    for row in workspace.rect_rows(plane.plane_id, rect)? {
        out.extend_from_slice(row);
    }
    if out.len() != expected {
        return Err(GeneralIntraResidualError::PredictionLength {
            expected,
            actual: out.len(),
        });
    }
    Ok(())
}

fn write_plane_block<T: ReconSample>(
    workspace: &mut CurrentFrameWorkspace<T>,
    plane: ResidualPlanePlan,
    samples: &[T],
) -> core::result::Result<(), GeneralIntraResidualError> {
    let (rect, width) = plane_rect(plane)?;
    workspace.write_rect(plane.plane_id, rect, samples, width)?;
    Ok(())
}
