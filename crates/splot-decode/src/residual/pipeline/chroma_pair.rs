// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Paired chroma residual reconstruction.

use splot_recon::{CurrentFrameWorkspace, PlaneId, PlaneRect, ReconSample};

use crate::bitstream::tile_payload::{
    DecodeTileWorkUnit, GeneralIntraResidualError, LumaCoeffBlock, LumaTransformTypeContext,
    TileBlockDecodedState, current_frame_qm_segment_id, is_cctx_geometry_allowed,
    reconstruct_general_intra_chroma_cctx_pair_with_predictions,
};

use super::ResidualPlanePlan;

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
    workspace: &mut CurrentFrameWorkspace<T>,
    block_decoded: &TileBlockDecodedState,
    u: (ResidualPlanePlan, LumaCoeffBlock),
    v: Option<(ResidualPlanePlan, LumaCoeffBlock)>,
    qindex: u32,
    intra_edge: crate::prediction::intra_edge::IntraEdgeCtx,
    luma_context: LumaTransformTypeContext,
) -> core::result::Result<(), GeneralIntraResidualError> {
    let (u_plane, u_coeffs) = u;
    let cctx_type = u_coeffs.cctx_type.unwrap_or(0);
    let Some((v_plane, v_coeffs)) = v else {
        if cctx_type != 0 {
            return Err(GeneralIntraResidualError::UnexpectedBranch);
        }
        return u_plane.reconstruct(
            workspace,
            &u_coeffs,
            block_decoded,
            None,
            qindex,
            intra_edge,
            luma_context,
        );
    };
    if cctx_type == 0 {
        u_plane.reconstruct(
            workspace,
            &u_coeffs,
            block_decoded,
            None,
            qindex,
            intra_edge,
            luma_context,
        )?;
        return v_plane.reconstruct(
            workspace,
            &v_coeffs,
            block_decoded,
            None,
            qindex,
            intra_edge,
            luma_context,
        );
    }
    reconstruct_chroma_cctx_pair(
        workspace,
        block_decoded,
        (u_plane, u_coeffs),
        (v_plane, v_coeffs),
        qindex,
        cctx_type,
        intra_edge,
        luma_context,
    )
}

#[allow(clippy::too_many_arguments)]
fn reconstruct_chroma_cctx_pair<T: ReconSample>(
    workspace: &mut CurrentFrameWorkspace<T>,
    block_decoded: &TileBlockDecodedState,
    u: (ResidualPlanePlan, LumaCoeffBlock),
    v: (ResidualPlanePlan, LumaCoeffBlock),
    qindex: u32,
    cctx_type: usize,
    intra_edge: crate::prediction::intra_edge::IntraEdgeCtx,
    luma_context: LumaTransformTypeContext,
) -> core::result::Result<(), GeneralIntraResidualError> {
    let (u_plane, u_coeffs) = u;
    let (v_plane, v_coeffs) = v;
    if u_plane.plane_id != PlaneId::U
        || v_plane.plane_id != PlaneId::V
        || u_plane.x != v_plane.x
        || u_plane.y != v_plane.y
        || u_plane.tx != v_plane.tx
    {
        return Err(GeneralIntraResidualError::UnexpectedBranch);
    }
    let u_prediction_block = prediction_only_coeff_block(&u_coeffs);
    u_plane.reconstruct(
        workspace,
        &u_prediction_block,
        block_decoded,
        None,
        qindex,
        intra_edge,
        luma_context,
    )?;
    let u_prediction = read_plane_prediction(workspace, u_plane)?;
    let v_prediction_block = prediction_only_coeff_block(&v_coeffs);
    v_plane.reconstruct(
        workspace,
        &v_prediction_block,
        block_decoded,
        None,
        qindex,
        intra_edge,
        luma_context,
    )?;
    let v_prediction = read_plane_prediction(workspace, v_plane)?;
    let (u_out, v_out) = reconstruct_general_intra_chroma_cctx_pair_with_predictions(
        &u_coeffs,
        &u_prediction,
        &v_coeffs,
        &v_prediction,
        qindex,
        u_plane.tx.width_log2(),
        u_plane.tx.height_log2(),
        cctx_type,
        false,
        u_plane.block_ctx.bit_depth(),
    )?;
    write_plane_block(workspace, u_plane, &u_out)?;
    write_plane_block(workspace, v_plane, &v_out)?;
    Ok(())
}

fn prediction_only_coeff_block(coeffs: &LumaCoeffBlock) -> LumaCoeffBlock {
    LumaCoeffBlock {
        all_zero: true,
        eob: 0,
        quant: Vec::new(),
        intra_ist: None,
        cctx_type: None,
        plane_tx_type: coeffs.plane_tx_type,
        use_tcq: false,
        lossless: coeffs.lossless,
    }
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
) -> core::result::Result<Vec<T>, GeneralIntraResidualError> {
    let (rect, width) = plane_rect(plane)?;
    let expected = width
        .checked_mul(rect.height())
        .ok_or(GeneralIntraResidualError::UnexpectedBranch)?;
    let mut out = Vec::with_capacity(expected);
    for row in workspace.rect_rows(plane.plane_id, rect)? {
        out.extend_from_slice(row);
    }
    if out.len() != expected {
        return Err(GeneralIntraResidualError::PredictionLength {
            expected,
            actual: out.len(),
        });
    }
    Ok(out)
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
