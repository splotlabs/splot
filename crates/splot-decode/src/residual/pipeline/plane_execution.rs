// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Shared residual-plane execution wrapper.

use splot_core::symbol::SymbolDecoder;
use splot_recon::{CurrentFrameWorkspace, PlaneId, ReconSample};

use crate::bitstream::tile_payload::{
    DecodeTileWorkUnit, GeneralIntraResidualError, LumaCoeffBlock, LumaTransformPartitionContext,
    LumaTransformTypeContext, TileBlockDecodedState, TileCoeffContextState,
    TransformToolResidualPolicy,
};

use super::{DeblockRecorder, ResidualPlanePlan};

pub(super) struct ResidualPlaneExecution {
    pub(super) coeffs: LumaCoeffBlock,
    pub(super) last_unit_nonzero: Option<bool>,
}

#[allow(clippy::too_many_arguments)]
pub(super) fn execute_residual_plane<T: ReconSample>(
    plane: ResidualPlanePlan,
    work_unit: &mut DecodeTileWorkUnit<'_>,
    symbols: &mut SymbolDecoder<'_>,
    coeff_ctx: &mut TileCoeffContextState,
    workspace: &mut CurrentFrameWorkspace<T>,
    block_decoded: &mut TileBlockDecodedState,
    uv_mode: usize,
    luma_transform_type_context: LumaTransformTypeContext,
    luma_tx_partition_context: Option<LumaTransformPartitionContext>,
    transform_tool_residual_policy: TransformToolResidualPolicy,
    qindex: u32,
    eob_u_nonzero: bool,
    intra_edge: crate::prediction::intra_edge::IntraEdgeCtx,
    deblock: &mut DeblockRecorder<'_>,
) -> core::result::Result<ResidualPlaneExecution, GeneralIntraResidualError> {
    let tx_partition_context = (plane.plane_id == PlaneId::Y)
        .then_some(luma_tx_partition_context)
        .flatten();
    plane.execute(
        work_unit,
        symbols,
        coeff_ctx,
        workspace,
        block_decoded,
        uv_mode,
        luma_transform_type_context,
        tx_partition_context,
        transform_tool_residual_policy,
        qindex,
        eob_u_nonzero,
        intra_edge,
        deblock,
    )
}
