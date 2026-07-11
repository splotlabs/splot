// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Shared residual-plane execution wrapper.

use splot_core::symbol::SymbolDecoder;
use splot_recon::{CurrentFrameWorkspace, PlaneId, ReconSample};

use crate::bitstream::tile_payload::{
    DecodeTileWorkUnit, GeneralIntraResidualError, LumaCoeffBlock, LumaTransformPartitionContext,
    LumaTransformTypeContext, PositionedLumaCoeffBlock, TileBlockDecodedState,
    TileCoeffContextState, TransformToolResidualPolicy, decode_general_intra_luma_partition_coeffs,
    decode_general_intra_plane_coeffs,
};
use crate::pipeline::general_intra::inherited_chroma_angle_delta;

use super::transform_units::tx_size_log2;
use super::{DCT_DCT, DeblockRecorder, GeneralIntraResidualPlan, ResidualPlanePlan, chroma_pair};

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

impl GeneralIntraResidualPlan {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn execute<T: ReconSample>(
        &self,
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
        intra_edge: crate::prediction::intra_edge::IntraEdgeCtx,
        deblock: &mut DeblockRecorder<'_>,
    ) -> core::result::Result<(), GeneralIntraResidualError> {
        let mut u_nonzero = false;
        let mut pending_u = None;
        let mut deferred = Vec::new();
        for &plane in &self.planes {
            let eob_u_nonzero = plane.plane_id == PlaneId::V && u_nonzero;
            if chroma_pair::can_hold_for_cctx_pair(plane, work_unit) {
                let execution = execute_residual_plane(
                    plane.with_deferred_reconstruction(),
                    work_unit,
                    symbols,
                    coeff_ctx,
                    workspace,
                    block_decoded,
                    uv_mode,
                    luma_transform_type_context,
                    luma_tx_partition_context,
                    transform_tool_residual_policy,
                    qindex,
                    false,
                    intra_edge,
                    deblock,
                )?;
                u_nonzero = execution
                    .last_unit_nonzero
                    .unwrap_or(!execution.coeffs.all_zero);
                pending_u = Some((plane, execution.coeffs));
                continue;
            }
            if plane.plane_id == PlaneId::V
                && !plane.defer_reconstruction
                && let Some((u_plane, u_coeffs)) = pending_u.take()
            {
                let execution = execute_residual_plane(
                    plane.with_deferred_reconstruction(),
                    work_unit,
                    symbols,
                    coeff_ctx,
                    workspace,
                    block_decoded,
                    uv_mode,
                    luma_transform_type_context,
                    luma_tx_partition_context,
                    transform_tool_residual_policy,
                    qindex,
                    eob_u_nonzero,
                    intra_edge,
                    deblock,
                )?;
                chroma_pair::reconstruct_chroma_pair_or_planes(
                    workspace,
                    block_decoded,
                    (u_plane, u_coeffs),
                    Some((plane, execution.coeffs)),
                    qindex,
                    intra_edge,
                    luma_transform_type_context,
                )?;
                continue;
            }
            let execution = execute_residual_plane(
                plane,
                work_unit,
                symbols,
                coeff_ctx,
                workspace,
                block_decoded,
                uv_mode,
                luma_transform_type_context,
                luma_tx_partition_context,
                transform_tool_residual_policy,
                qindex,
                eob_u_nonzero,
                intra_edge,
                deblock,
            )?;
            if plane.plane_id == PlaneId::U {
                u_nonzero = execution
                    .last_unit_nonzero
                    .unwrap_or(!execution.coeffs.all_zero);
            }
            if plane.defer_reconstruction {
                deferred.push((plane, execution.coeffs));
            }
        }
        if let Some((u_plane, u_coeffs)) = pending_u {
            chroma_pair::reconstruct_chroma_pair_or_planes(
                workspace,
                block_decoded,
                (u_plane, u_coeffs),
                None,
                qindex,
                intra_edge,
                luma_transform_type_context,
            )?;
        }
        chroma_pair::reconstruct_deferred_planes(
            workspace,
            block_decoded,
            deferred,
            qindex,
            intra_edge,
            luma_transform_type_context,
        )?;
        Ok(())
    }
}

impl ResidualPlanePlan {
    pub(super) fn apply_reconstruction_tx_type(self, coeffs: &mut LumaCoeffBlock) {
        coeffs.plane_tx_type = self.reconstruction_tx_type.unwrap_or(coeffs.plane_tx_type);
    }

    #[allow(clippy::too_many_arguments)]
    fn execute<T: ReconSample>(
        self,
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
        let policy = transform_tool_policy_for_plane(
            transform_tool_residual_policy,
            self.plane_id,
            luma_transform_type_context,
        );
        let angle_delta_uv =
            chroma_angle_delta_uv(self.plane_id, uv_mode, luma_transform_type_context);
        let palette_color_map = self.read_palette_color_map(work_unit, symbols)?;
        if let Some(unit_tx_size) = self.lossless_transform_unit_tx_size(work_unit) {
            return self.execute_lossless_transform_units(
                unit_tx_size,
                work_unit,
                symbols,
                coeff_ctx,
                workspace,
                block_decoded,
                uv_mode,
                angle_delta_uv,
                policy,
                qindex,
                eob_u_nonzero,
                palette_color_map.as_deref(),
                intra_edge,
                luma_transform_type_context,
                deblock,
            );
        }
        if self.plane_id == PlaneId::Y
            && let Some(tx_partition_context) = luma_tx_partition_context
        {
            return self.execute_partitioned_luma(
                work_unit,
                symbols,
                coeff_ctx,
                workspace,
                block_decoded,
                tx_partition_context,
                uv_mode,
                angle_delta_uv,
                policy,
                qindex,
                palette_color_map.as_deref(),
                intra_edge,
                luma_transform_type_context,
                deblock,
            );
        }
        let mut coeffs = crate::bitstream::tile_payload::decode_general_intra_plane_coeffs(
            work_unit,
            symbols,
            coeff_ctx,
            self.coeff_plane,
            self.tx_size,
            self.x,
            self.y,
            self.tx_fills_residual_block(),
            luma_tx_partition_context,
            eob_u_nonzero,
            uv_mode,
            angle_delta_uv,
            DCT_DCT,
            false,
            self.fsc_mode,
            self.txb_skip_fsc_mode,
            chroma_pair::cctx_allowed(self),
            policy,
        )?;
        self.apply_reconstruction_tx_type(&mut coeffs);
        if self.plane_id == PlaneId::Y {
            deblock.record_luma_unit(
                self.y / 4,
                self.x / 4,
                self.tx.width4(),
                self.tx.height4(),
                self.tx_size,
                coeffs.eob,
            );
        } else {
            deblock.record_chroma_unit(self.plane_id, self.x, self.y, self.tx_size);
        }
        if !self.defer_reconstruction {
            self.reconstruct(
                workspace,
                &coeffs,
                block_decoded,
                palette_color_map.as_deref(),
                qindex,
                intra_edge,
                luma_transform_type_context,
            )?;
        }
        Ok(ResidualPlaneExecution {
            coeffs,
            last_unit_nonzero: None,
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn execute_lossless_transform_units<T: ReconSample>(
        self,
        unit_tx_size: usize,
        work_unit: &mut DecodeTileWorkUnit<'_>,
        symbols: &mut SymbolDecoder<'_>,
        coeff_ctx: &mut TileCoeffContextState,
        workspace: &mut CurrentFrameWorkspace<T>,
        block_decoded: &mut TileBlockDecodedState,
        uv_mode: usize,
        angle_delta_uv: i32,
        policy: TransformToolResidualPolicy,
        qindex: u32,
        eob_u_nonzero: bool,
        palette_color_map: Option<&[u8]>,
        intra_edge: crate::prediction::intra_edge::IntraEdgeCtx,
        luma_context: LumaTransformTypeContext,
        deblock: &mut DeblockRecorder<'_>,
    ) -> core::result::Result<ResidualPlaneExecution, GeneralIntraResidualError> {
        let (log2_width, log2_height) = tx_size_log2(unit_tx_size)?;
        let unit_width4 = (1usize << log2_width) >> 2;
        let unit_height4 = (1usize << log2_height) >> 2;
        let mut blocks = Vec::new();
        let mut last_unit_nonzero = None;
        for y4 in (0..self.tx.height4()).step_by(unit_height4) {
            for x4 in (0..self.tx.width4()).step_by(unit_width4) {
                let x = self.x + x4 * 4;
                let y = self.y + y4 * 4;
                if !self.lossless_unit_starts_in_frame(x, y) {
                    continue;
                }
                let mut coeffs = decode_general_intra_plane_coeffs(
                    work_unit,
                    symbols,
                    coeff_ctx,
                    self.coeff_plane,
                    unit_tx_size,
                    x,
                    y,
                    false,
                    None,
                    eob_u_nonzero,
                    uv_mode,
                    angle_delta_uv,
                    DCT_DCT,
                    false,
                    self.fsc_mode,
                    self.txb_skip_fsc_mode,
                    chroma_pair::cctx_allowed(self),
                    policy,
                )?;
                self.apply_reconstruction_tx_type(&mut coeffs);
                let block = PositionedLumaCoeffBlock {
                    x,
                    y,
                    tx_size: unit_tx_size,
                    middle: false,
                    coeffs,
                };
                let unit = self.transform_unit_plan(&block)?;
                let unit_palette_color_map =
                    self.palette_color_map_for_unit(palette_color_map, &block)?;
                unit.reconstruct(
                    workspace,
                    &block.coeffs,
                    block_decoded,
                    unit_palette_color_map.as_deref(),
                    qindex,
                    intra_edge,
                    luma_context,
                )?;
                if unit.plane_id == PlaneId::Y {
                    let row4 = block.y / 4;
                    let col4 = block.x / 4;
                    deblock.record_luma_unit(
                        row4,
                        col4,
                        unit_width4,
                        unit_height4,
                        block.tx_size,
                        block.coeffs.eob,
                    );
                } else {
                    deblock.record_chroma_unit(unit.plane_id, block.x, block.y, block.tx_size);
                }
                let (sub_x, sub_y) = self.block_ctx.chroma().subsampling(unit.plane_id);
                let row4 = ((block.y >> 2) << sub_y) & block_decoded.sb_size4().saturating_sub(1);
                let col4 = ((block.x >> 2) << sub_x) & block_decoded.sb_size4().saturating_sub(1);
                let plane = unit.plane_id.index();
                block_decoded.set_block(plane, row4, col4, unit_width4, unit_height4);
                if unit.plane_id == PlaneId::U {
                    last_unit_nonzero = Some(!block.coeffs.all_zero);
                }
                blocks.push(block);
            }
        }
        let summary = summarize_luma_partition(&blocks);
        Ok(ResidualPlaneExecution {
            coeffs: summary,
            last_unit_nonzero,
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn execute_partitioned_luma<T: ReconSample>(
        self,
        work_unit: &mut DecodeTileWorkUnit<'_>,
        symbols: &mut SymbolDecoder<'_>,
        coeff_ctx: &mut TileCoeffContextState,
        workspace: &mut CurrentFrameWorkspace<T>,
        block_decoded: &mut TileBlockDecodedState,
        tx_partition_context: LumaTransformPartitionContext,
        uv_mode: usize,
        angle_delta_uv: i32,
        policy: TransformToolResidualPolicy,
        qindex: u32,
        palette_color_map: Option<&[u8]>,
        intra_edge: crate::prediction::intra_edge::IntraEdgeCtx,
        luma_context: LumaTransformTypeContext,
        deblock: &mut DeblockRecorder<'_>,
    ) -> core::result::Result<ResidualPlaneExecution, GeneralIntraResidualError> {
        let mut blocks = decode_general_intra_luma_partition_coeffs(
            work_unit,
            symbols,
            coeff_ctx,
            self.tx_size,
            self.x,
            self.y,
            self.block_ctx.frame_mi_cols().saturating_mul(4),
            self.block_ctx.frame_mi_rows().saturating_mul(4),
            self.tx_fills_residual_block(),
            tx_partition_context,
            uv_mode,
            angle_delta_uv,
            self.fsc_mode,
            policy,
        )?;
        if blocks.len() == 1 {
            let block = blocks.remove(0);
            let (log2_width, log2_height) = tx_size_log2(block.tx_size)?;
            let width4 = ((1usize << log2_width) >> 2).max(1);
            let height4 = ((1usize << log2_height) >> 2).max(1);
            deblock.record_luma_unit(
                block.y / 4,
                block.x / 4,
                width4,
                height4,
                block.tx_size,
                block.coeffs.eob,
            );
            let unit_palette_color_map =
                self.palette_color_map_for_unit(palette_color_map, &block)?;
            self.reconstruct(
                workspace,
                &block.coeffs,
                block_decoded,
                unit_palette_color_map.as_deref(),
                qindex,
                intra_edge,
                luma_context,
            )?;
            block_decoded.set_luma_transform(block.x, block.y, width4, height4);
            return Ok(ResidualPlaneExecution {
                coeffs: block.coeffs,
                last_unit_nonzero: None,
            });
        }

        for block in &blocks {
            let unit = self.transform_unit_plan(block)?;
            let unit_palette_color_map =
                self.palette_color_map_for_unit(palette_color_map, block)?;
            unit.reconstruct(
                workspace,
                &block.coeffs,
                block_decoded,
                unit_palette_color_map.as_deref(),
                qindex,
                intra_edge,
                luma_context,
            )?;
            let (log2_width, log2_height) = tx_size_log2(block.tx_size)?;
            let width4 = ((1usize << log2_width) >> 2).max(1);
            let height4 = ((1usize << log2_height) >> 2).max(1);
            deblock.record_luma_unit(
                block.y / 4,
                block.x / 4,
                width4,
                height4,
                block.tx_size,
                block.coeffs.eob,
            );
            block_decoded.set_luma_transform(block.x, block.y, width4, height4);
        }
        let summary = summarize_luma_partition(&blocks);
        Ok(ResidualPlaneExecution {
            coeffs: summary,
            last_unit_nonzero: None,
        })
    }
}

fn summarize_luma_partition(
    blocks: &[PositionedLumaCoeffBlock],
) -> crate::bitstream::tile_payload::LumaCoeffBlock {
    crate::bitstream::tile_payload::LumaCoeffBlock {
        all_zero: blocks.iter().all(|block| block.coeffs.all_zero),
        eob: blocks
            .iter()
            .fold(0usize, |sum, block| sum.saturating_add(block.coeffs.eob)),
        quant: Vec::new(),
        intra_ist: blocks.iter().find_map(|block| block.coeffs.intra_ist),
        cctx_type: blocks.iter().find_map(|block| block.coeffs.cctx_type),
        plane_tx_type: blocks
            .iter()
            .find(|block| !block.coeffs.all_zero)
            .or_else(|| blocks.first())
            .map_or(0, |block| block.coeffs.plane_tx_type),
        use_tcq: blocks.iter().any(|block| block.coeffs.use_tcq),
        lossless: blocks.iter().any(|block| block.coeffs.lossless),
    }
}

pub(super) fn chroma_angle_delta_uv(
    plane_id: PlaneId,
    uv_mode: usize,
    luma: LumaTransformTypeContext,
) -> i32 {
    if matches!(plane_id, PlaneId::U | PlaneId::V) {
        i32::from(inherited_chroma_angle_delta(
            uv_mode,
            luma.y_mode(),
            luma.angle_delta_y(),
        ))
    } else {
        0
    }
}

const fn transform_tool_policy_for_plane(
    policy: TransformToolResidualPolicy,
    plane_id: PlaneId,
    luma: LumaTransformTypeContext,
) -> TransformToolResidualPolicy {
    match (policy, plane_id) {
        (
            TransformToolResidualPolicy::AdmitTransformToolSubset {
                active_intra_ist,
                active_chroma,
                ..
            },
            PlaneId::Y,
        ) => TransformToolResidualPolicy::AdmitTransformToolSubset {
            luma: Some(luma),
            active_intra_ist,
            active_chroma,
        },
        _ => policy,
    }
}
