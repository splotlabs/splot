// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Runtime residual transform dispatch.

use splot_core::symbol::SymbolDecoder;
use splot_core::tables::conversion::{TX_HEIGHT_LOG2, TX_WIDTH_LOG2};
use splot_recon::{
    CurrentFrameWorkspace, DpcmDirection, IntraCardinalDirection, PlaneId, ReconSample,
};

use crate::bitstream::tile_payload::{
    CflParams, DecodeTileWorkUnit, GeneralIntraResidualError, LumaPalette,
    LumaTransformPartitionContext, LumaTransformTypeContext, PositionedLumaCoeffBlock,
    SupportedChromaMode, SupportedDirectionalLumaMode, SupportedNonDcLumaMode,
    TileBlockDecodedState, TileCdfSelector, TileCoeffContextState, TransformToolResidualPolicy,
    current_frame_qm_segment_id, decode_general_intra_luma_partition_coeffs,
    decode_general_intra_plane_coeffs,
};
use crate::pipeline::reconstruct::IntraEdgeAvailability as EdgeAvail;
use crate::pipeline::reconstruct::MiddleEdgeAvailability as MiddleAvail;
use crate::pipeline::reconstruct::OneSidedAboveMrl as AboveMrl;
use crate::prediction::intra::IntraLumaPlan;
use crate::support::capability::missing_capability_message;
use crate::tile::block_context::{BlockCtx, BlockRect, NeighbourAvailability, TxShape};

mod chroma_pair;
mod deblock_recorder;
mod plane_execution;

pub(crate) use deblock_recorder::DeblockRecorder;
use plane_execution::{ResidualPlaneExecution, execute_residual_plane};

const CHROMA_PLANES: [PlaneId; 2] = [PlaneId::U, PlaneId::V];
const CHUNK_64_N4: usize = 16;
const PALETTE_MAX_SIZE: usize = 8;
const PALETTE_COLOR_CONTEXTS: usize = 5;
const PALETTE_ROW_COPY_PREVIOUS: u8 = 2;
const PALETTE_ROW_COPY_LAST: u8 = 1;
const PALETTE_DIRECTION_REASON: &str = "palette_direction";
const PALETTE_UNIFORM_REASON: &str = "palette_color_idx_uniform";
const TX_4X4: usize = 0;
const DCT_DCT: usize = 0;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct GeneralIntraResidualPlan {
    planes: Vec<ResidualPlanePlan>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ResidualBlockTransforms {
    luma_tx: usize,
    chroma_tx: Option<usize>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RectLumaPlan {
    Palette {
        palette: LumaPalette,
        use_tcq: bool,
    },
    Dc {
        use_tcq: bool,
    },
    Dip {
        mode: u8,
        transpose: bool,
        use_tcq: bool,
    },
    Middle {
        p_angle: u16,
        use_tcq: bool,
    },
    MiddleMrl {
        p_angle: u16,
        mrl_index: usize,
        above_mrl_index: usize,
        is_sb_boundary: bool,
        secondary_mrl: bool,
        use_tcq: bool,
    },
    OneSidedAboveMrl {
        p_angle: u16,
        mrl_index: usize,
        above_mrl_index: usize,
        secondary_mrl: bool,
        use_tcq: bool,
    },
    OneSidedLeftMrl {
        p_angle: u16,
        mrl_index: usize,
        secondary_mrl: bool,
        use_tcq: bool,
    },
    CardinalMrl {
        direction: IntraCardinalDirection,
        mrl_index: usize,
        above_mrl_index: usize,
        secondary_mrl: bool,
        use_tcq: bool,
    },
    OneSidedAbove {
        p_angle: u16,
        use_tcq: bool,
    },
    OneSidedLeft {
        p_angle: u16,
        use_tcq: bool,
    },
    Cardinal {
        direction: IntraCardinalDirection,
        use_tcq: bool,
    },
    Paeth {
        use_tcq: bool,
    },
    Smooth {
        mode: SupportedNonDcLumaMode,
        use_tcq: bool,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RectChromaPlan {
    Mode(SupportedChromaMode, Option<DpcmDirection>),
    OneSided {
        p_angle: u16,
    },
    Middle {
        p_angle: u16,
    },
    Cfl {
        params: CflParams,
        cfl_ds_filter_index: u8,
        sb_mib: usize,
    },
}

impl ResidualBlockTransforms {
    pub(crate) const fn luma_tx(self) -> usize {
        self.luma_tx
    }

    pub(crate) const fn chroma_tx(self) -> Option<usize> {
        self.chroma_tx
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ResidualPipelineUnsupported {
    reason_id: &'static str,
    message: &'static str,
    spec_section: &'static str,
}

impl ResidualPipelineUnsupported {
    pub(crate) const fn reason_id(self) -> &'static str {
        self.reason_id
    }

    pub(crate) const fn message(self) -> &'static str {
        self.message
    }

    pub(crate) const fn spec_section(self) -> &'static str {
        self.spec_section
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ResidualPlanePlan {
    plane_id: PlaneId,
    block_ctx: BlockCtx,
    coeff_plane: usize,
    tx_size: usize,
    x: usize,
    y: usize,
    tx: TxShape,
    residual_width4: usize,
    residual_height4: usize,
    fsc_mode: bool,
    txb_skip_fsc_mode: bool,
    zero_corners: bool,
    defer_reconstruction: bool,
    reconstruction: ResidualReconstructionPlan,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ResidualReconstructionPlan {
    LumaPalette {
        palette: LumaPalette,
        use_tcq: bool,
    },
    LumaSquare {
        plan: IntraLumaPlan,
        use_tcq: bool,
    },
    LumaRectSmooth {
        mode: SupportedNonDcLumaMode,
        use_tcq: bool,
    },
    LumaRectDip {
        mode: u8,
        transpose: bool,
        use_tcq: bool,
    },
    LumaRectMiddle {
        p_angle: u16,
        use_tcq: bool,
    },
    LumaRectMiddleMrl {
        p_angle: u16,
        mrl_index: usize,
        above_mrl_index: usize,
        is_sb_boundary: bool,
        secondary_mrl: bool,
        use_tcq: bool,
    },
    LumaRectOneSidedAboveMrl {
        p_angle: u16,
        mrl_index: usize,
        above_mrl_index: usize,
        secondary_mrl: bool,
        use_tcq: bool,
    },
    LumaRectOneSidedLeftMrl {
        p_angle: u16,
        mrl_index: usize,
        secondary_mrl: bool,
        use_tcq: bool,
    },
    LumaRectCardinalMrl {
        direction: IntraCardinalDirection,
        mrl_index: usize,
        above_mrl_index: usize,
        secondary_mrl: bool,
        use_tcq: bool,
    },
    LumaRectOneSidedAbove {
        p_angle: u16,
        use_tcq: bool,
    },
    LumaRectOneSidedLeft {
        p_angle: u16,
        use_tcq: bool,
    },
    LumaRectCardinal {
        direction: IntraCardinalDirection,
        use_tcq: bool,
    },
    LumaRectPaeth {
        use_tcq: bool,
    },
    Chroma {
        mode: SupportedChromaMode,
        dpcm: Option<DpcmDirection>,
    },
    ChromaOneSided {
        p_angle: u16,
    },
    ChromaMiddle {
        p_angle: u16,
    },
    ChromaCfl {
        params: CflParams,
        cfl_ds_filter_index: u8,
        sb_mib: usize,
    },
    Rect {
        use_tcq: bool,
    },
}

impl GeneralIntraResidualPlan {
    pub(crate) fn square(
        block_ctx: BlockCtx,
        luma_plan: IntraLumaPlan,
        chroma_plan: Option<RectChromaPlan>,
        luma_use_tcq: bool,
        luma_fsc_mode: bool,
        luma_lossless_tx_size: Option<usize>,
        lossless: bool,
    ) -> core::result::Result<Self, ResidualPipelineUnsupported> {
        let mut planes = Vec::new();
        let luma_reconstruction = ResidualReconstructionPlan::LumaSquare {
            plan: luma_plan,
            use_tcq: luma_use_tcq,
        };
        let luma_reconstruction = match luma_plan {
            IntraLumaPlan::Palette { palette } => ResidualReconstructionPlan::LumaPalette {
                palette,
                use_tcq: luma_use_tcq,
            },
            _ => luma_reconstruction,
        };
        let chroma_reconstruction = chroma_plan.map(chroma_reconstruction);
        push_ordered_planes(
            &mut planes,
            block_ctx,
            luma_reconstruction,
            chroma_reconstruction,
            luma_fsc_mode,
            luma_lossless_tx_size,
            lossless,
        )?;
        Ok(Self { planes })
    }

    pub(crate) fn rect(
        block_ctx: BlockCtx,
        luma_plan: RectLumaPlan,
        chroma_plan: Option<RectChromaPlan>,
        luma_fsc_mode: bool,
        luma_lossless_tx_size: Option<usize>,
        lossless: bool,
    ) -> core::result::Result<Self, ResidualPipelineUnsupported> {
        let mut planes = Vec::new();
        let chroma_reconstruction = chroma_plan.map(chroma_reconstruction);
        let luma_reconstruction = match luma_plan {
            RectLumaPlan::Palette { palette, use_tcq } => {
                ResidualReconstructionPlan::LumaPalette { palette, use_tcq }
            }
            RectLumaPlan::Dc { use_tcq } => ResidualReconstructionPlan::Rect { use_tcq },
            RectLumaPlan::Dip {
                mode,
                transpose,
                use_tcq,
            } => ResidualReconstructionPlan::LumaRectDip {
                mode,
                transpose,
                use_tcq,
            },
            RectLumaPlan::Middle { p_angle, use_tcq } => {
                ResidualReconstructionPlan::LumaRectMiddle { p_angle, use_tcq }
            }
            RectLumaPlan::MiddleMrl {
                p_angle,
                mrl_index,
                above_mrl_index,
                is_sb_boundary,
                secondary_mrl,
                use_tcq,
            } => ResidualReconstructionPlan::LumaRectMiddleMrl {
                p_angle,
                mrl_index,
                above_mrl_index,
                is_sb_boundary,
                secondary_mrl,
                use_tcq,
            },
            RectLumaPlan::OneSidedAboveMrl {
                p_angle,
                mrl_index,
                above_mrl_index,
                secondary_mrl,
                use_tcq,
            } => ResidualReconstructionPlan::LumaRectOneSidedAboveMrl {
                p_angle,
                mrl_index,
                above_mrl_index,
                secondary_mrl,
                use_tcq,
            },
            RectLumaPlan::OneSidedLeftMrl {
                p_angle,
                mrl_index,
                secondary_mrl,
                use_tcq,
            } => ResidualReconstructionPlan::LumaRectOneSidedLeftMrl {
                p_angle,
                mrl_index,
                secondary_mrl,
                use_tcq,
            },
            RectLumaPlan::CardinalMrl {
                direction,
                mrl_index,
                above_mrl_index,
                secondary_mrl,
                use_tcq,
            } => ResidualReconstructionPlan::LumaRectCardinalMrl {
                direction,
                mrl_index,
                above_mrl_index,
                secondary_mrl,
                use_tcq,
            },
            RectLumaPlan::OneSidedAbove { p_angle, use_tcq } => {
                ResidualReconstructionPlan::LumaRectOneSidedAbove { p_angle, use_tcq }
            }
            RectLumaPlan::OneSidedLeft { p_angle, use_tcq } => {
                ResidualReconstructionPlan::LumaRectOneSidedLeft { p_angle, use_tcq }
            }
            RectLumaPlan::Cardinal { direction, use_tcq } => {
                ResidualReconstructionPlan::LumaRectCardinal { direction, use_tcq }
            }
            RectLumaPlan::Paeth { use_tcq } => {
                ResidualReconstructionPlan::LumaRectPaeth { use_tcq }
            }
            RectLumaPlan::Smooth { mode, use_tcq } => {
                ResidualReconstructionPlan::LumaRectSmooth { mode, use_tcq }
            }
        };
        push_ordered_planes(
            &mut planes,
            block_ctx,
            luma_reconstruction,
            chroma_reconstruction,
            luma_fsc_mode,
            luma_lossless_tx_size,
            lossless,
        )?;
        Ok(Self { planes })
    }

    pub(crate) fn chroma(
        block_ctx: BlockCtx,
        chroma_plan: RectChromaPlan,
    ) -> core::result::Result<Self, ResidualPipelineUnsupported> {
        let reconstruction = chroma_reconstruction(chroma_plan);
        let mut planes = Vec::new();
        planes.extend(chroma_plans(block_ctx, reconstruction, false, false)?);
        Ok(Self { planes })
    }

    pub(crate) fn transforms(&self) -> ResidualBlockTransforms {
        ResidualBlockTransforms {
            luma_tx: self
                .planes
                .iter()
                .find(|plane| plane.plane_id == PlaneId::Y)
                .map_or(0, |plane| plane.tx_size),
            chroma_tx: self
                .planes
                .iter()
                .find(|plane| plane.plane_id == PlaneId::U)
                .map(|plane| plane.tx_size),
        }
    }

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
        if !self.planes.iter().any(|plane| plane.plane_id == PlaneId::Y) {
            deblock.record_chroma_part_block();
        }
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

fn chroma_reconstruction(plan: RectChromaPlan) -> ResidualReconstructionPlan {
    match plan {
        RectChromaPlan::Mode(SupportedChromaMode::Dc, None) => {
            ResidualReconstructionPlan::Rect { use_tcq: false }
        }
        RectChromaPlan::Mode(mode, dpcm) => ResidualReconstructionPlan::Chroma { mode, dpcm },
        RectChromaPlan::OneSided { p_angle } => {
            ResidualReconstructionPlan::ChromaOneSided { p_angle }
        }
        RectChromaPlan::Middle { p_angle } => ResidualReconstructionPlan::ChromaMiddle { p_angle },
        RectChromaPlan::Cfl {
            params,
            cfl_ds_filter_index,
            sb_mib,
        } => ResidualReconstructionPlan::ChromaCfl {
            params,
            cfl_ds_filter_index,
            sb_mib,
        },
    }
}

impl ResidualPlanePlan {
    #[allow(clippy::too_many_arguments)]
    fn new(
        block_ctx: BlockCtx,
        plane_id: PlaneId,
        reconstruction: ResidualReconstructionPlan,
        residual_width4: usize,
        residual_height4: usize,
        fsc_mode: bool,
        txb_skip_fsc_mode: bool,
        tx_size_override: Option<usize>,
    ) -> core::result::Result<Self, ResidualPipelineUnsupported> {
        let block = block_ctx.plane_block(plane_id);
        let tx = block.tx();
        Ok(Self {
            plane_id,
            block_ctx,
            coeff_plane: coeff_plane(plane_id),
            tx_size: tx_size_override.unwrap_or(tx_size_for_plan(tx, plane_id)?),
            x: block.x(),
            y: block.y(),
            tx,
            residual_width4,
            residual_height4,
            fsc_mode,
            txb_skip_fsc_mode,
            zero_corners: false,
            defer_reconstruction: false,
            reconstruction,
        })
    }

    const fn with_deferred_reconstruction(self) -> Self {
        Self {
            defer_reconstruction: true,
            ..self
        }
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
        let coeffs = crate::bitstream::tile_payload::decode_general_intra_plane_coeffs(
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
            policy,
        )?;
        if self.plane_id == PlaneId::Y {
            deblock.record_luma_unit(
                self.y / 4,
                self.x / 4,
                self.tx.width4(),
                self.tx.height4(),
                self.tx_size,
                coeffs.eob,
            );
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

    fn lossless_transform_unit_tx_size(self, work_unit: &DecodeTileWorkUnit<'_>) -> Option<usize> {
        if work_unit
            .coeff_frame_facts()
            .lossless_for_segment(current_frame_qm_segment_id())
            != Some(true)
            || (!self.fsc_mode && self.tx_size == TX_4X4)
        {
            return None;
        }
        if !self.fsc_mode {
            return Some(TX_4X4);
        }
        let (log2_width, log2_height) = tx_size_log2(self.tx_size).ok()?;
        let unit_width4 = (1usize << log2_width) >> 2;
        let unit_height4 = (1usize << log2_height) >> 2;
        (unit_width4 < self.tx.width4() || unit_height4 < self.tx.height4()).then_some(self.tx_size)
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
                let coeffs = decode_general_intra_plane_coeffs(
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
                    policy,
                )?;
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

    fn lossless_unit_starts_in_frame(self, x: usize, y: usize) -> bool {
        let (sub_x, sub_y) = self.block_ctx.chroma().subsampling(self.plane_id);
        let max_x = (self.block_ctx.frame_mi_cols() * 4) >> sub_x;
        let max_y = (self.block_ctx.frame_mi_rows() * 4) >> sub_y;
        x < max_x && y < max_y
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
            deblock.record_luma_unit(
                block.y / 4,
                block.x / 4,
                ((1usize << log2_width) / 4).max(1),
                ((1usize << log2_height) / 4).max(1),
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
            return Ok(ResidualPlaneExecution {
                coeffs: block.coeffs,
                last_unit_nonzero: None,
            });
        }

        let sb_mask = block_decoded.sb_size4().saturating_sub(1);
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
            deblock.record_luma_unit(
                block.y / 4,
                block.x / 4,
                ((1usize << log2_width) / 4).max(1),
                ((1usize << log2_height) / 4).max(1),
                block.tx_size,
                block.coeffs.eob,
            );
            block_decoded.set_block(
                0,
                (block.y >> 2) & sb_mask,
                (block.x >> 2) & sb_mask,
                ((1usize << log2_width) >> 2).max(1),
                ((1usize << log2_height) >> 2).max(1),
            );
        }
        let summary = summarize_luma_partition(&blocks);
        Ok(ResidualPlaneExecution {
            coeffs: summary,
            last_unit_nonzero: None,
        })
    }

    fn transform_unit_plan(
        &self,
        block: &PositionedLumaCoeffBlock,
    ) -> core::result::Result<ResidualPlanePlan, GeneralIntraResidualError> {
        let reconstruction = match self.reconstruction {
            ResidualReconstructionPlan::Rect { .. }
            | ResidualReconstructionPlan::Chroma { .. }
            | ResidualReconstructionPlan::ChromaMiddle { .. }
            | ResidualReconstructionPlan::ChromaOneSided { .. }
            | ResidualReconstructionPlan::LumaPalette { .. }
            | ResidualReconstructionPlan::LumaRectCardinal { .. }
            | ResidualReconstructionPlan::LumaRectPaeth { .. }
            | ResidualReconstructionPlan::LumaRectSmooth { .. }
            | ResidualReconstructionPlan::LumaRectMiddle { .. }
            | ResidualReconstructionPlan::LumaRectOneSidedAbove { .. }
            | ResidualReconstructionPlan::LumaRectOneSidedLeft { .. }
            | ResidualReconstructionPlan::LumaRectOneSidedLeftMrl { .. }
            | ResidualReconstructionPlan::LumaRectDip { .. } => self.reconstruction,
            ResidualReconstructionPlan::LumaRectOneSidedAboveMrl {
                p_angle,
                mrl_index,
                above_mrl_index,
                secondary_mrl,
                use_tcq,
            } => ResidualReconstructionPlan::LumaRectOneSidedAboveMrl {
                p_angle,
                mrl_index,
                above_mrl_index: if block.y == self.y {
                    above_mrl_index
                } else {
                    mrl_index
                },
                secondary_mrl,
                use_tcq,
            },
            ResidualReconstructionPlan::LumaRectCardinalMrl {
                direction,
                mrl_index,
                above_mrl_index,
                secondary_mrl,
                use_tcq,
            } => ResidualReconstructionPlan::LumaRectCardinalMrl {
                direction,
                mrl_index,
                above_mrl_index: if block.y == self.y {
                    above_mrl_index
                } else {
                    mrl_index
                },
                secondary_mrl,
                use_tcq,
            },
            ResidualReconstructionPlan::LumaRectMiddleMrl {
                p_angle,
                mrl_index,
                above_mrl_index,
                is_sb_boundary,
                secondary_mrl,
                use_tcq,
            } => ResidualReconstructionPlan::LumaRectMiddleMrl {
                p_angle,
                mrl_index,
                above_mrl_index: if block.y == self.y {
                    above_mrl_index
                } else {
                    mrl_index
                },
                is_sb_boundary: is_sb_boundary && block.y == self.y,
                secondary_mrl,
                use_tcq,
            },
            ResidualReconstructionPlan::LumaSquare {
                plan: IntraLumaPlan::CardinalNeighbour { direction },
                use_tcq,
            } => ResidualReconstructionPlan::LumaRectCardinal { direction, use_tcq },
            ResidualReconstructionPlan::LumaSquare {
                plan: IntraLumaPlan::Dc,
                use_tcq,
            } => ResidualReconstructionPlan::Rect { use_tcq },
            ResidualReconstructionPlan::LumaSquare {
                plan: IntraLumaPlan::Dip { mode, transpose },
                use_tcq,
            } => ResidualReconstructionPlan::LumaRectDip {
                mode,
                transpose,
                use_tcq,
            },
            ResidualReconstructionPlan::LumaSquare {
                plan: IntraLumaPlan::PaethNeighbour,
                use_tcq,
            } => ResidualReconstructionPlan::LumaRectPaeth { use_tcq },
            ResidualReconstructionPlan::LumaSquare {
                plan: IntraLumaPlan::DirectionalMiddle { p_angle },
                use_tcq,
            } => ResidualReconstructionPlan::LumaRectMiddle { p_angle, use_tcq },
            ResidualReconstructionPlan::LumaSquare {
                plan: IntraLumaPlan::DirectionalOneSidedAbove { p_angle },
                use_tcq,
            } => ResidualReconstructionPlan::LumaRectOneSidedAbove { p_angle, use_tcq },
            ResidualReconstructionPlan::LumaSquare {
                plan: IntraLumaPlan::DirectionalOneSidedLeft { p_angle },
                use_tcq,
            } => ResidualReconstructionPlan::LumaRectOneSidedLeft { p_angle, use_tcq },
            ResidualReconstructionPlan::LumaSquare {
                plan:
                    IntraLumaPlan::DirectionalFirst {
                        mode: SupportedDirectionalLumaMode::D135,
                    },
                use_tcq,
            } if block.coeffs.lossless => {
                if block.x == self.x && block.y == self.y {
                    self.reconstruction
                } else {
                    ResidualReconstructionPlan::LumaRectMiddle {
                        p_angle: crate::prediction::intra::directional_mode_p_angle(
                            SupportedDirectionalLumaMode::D135,
                        ),
                        use_tcq,
                    }
                }
            }
            ResidualReconstructionPlan::LumaSquare {
                plan: IntraLumaPlan::DirectionalNeighbour { mode },
                use_tcq,
            } => {
                let p_angle = crate::prediction::intra::directional_mode_p_angle(mode);
                if p_angle < 90 {
                    ResidualReconstructionPlan::LumaRectOneSidedAbove { p_angle, use_tcq }
                } else if p_angle > 180 {
                    ResidualReconstructionPlan::LumaRectOneSidedLeft { p_angle, use_tcq }
                } else {
                    ResidualReconstructionPlan::LumaRectMiddle { p_angle, use_tcq }
                }
            }
            ResidualReconstructionPlan::LumaSquare {
                plan: IntraLumaPlan::NonDcNeighbour { mode },
                use_tcq,
            } => ResidualReconstructionPlan::LumaRectSmooth { mode, use_tcq },
            ResidualReconstructionPlan::LumaSquare {
                plan: IntraLumaPlan::NonDcFirst { mode },
                use_tcq,
            } => {
                if block.x == self.x && block.y == self.y {
                    self.reconstruction
                } else {
                    ResidualReconstructionPlan::LumaRectSmooth { mode, use_tcq }
                }
            }
            _ => {
                return Err(GeneralIntraResidualError::UnsupportedTransformPartition {
                    reason: "general_intra_partitioned_interior_edge_prediction",
                });
            }
        };
        let (log2_width, log2_height) = tx_size_log2(block.tx_size)?;
        let width4 = (1usize << log2_width) >> 2;
        let height4 = (1usize << log2_height) >> 2;
        let tx = TxShape::from_luma_4x4(width4.max(1), height4.max(1)).ok_or(
            GeneralIntraResidualError::TransformPartitionGeometry {
                table: "Tx_Width_Log2",
                index: block.tx_size,
            },
        )?;
        let block_ctx = self.transform_unit_block_ctx(block, tx, width4.max(1), height4.max(1))?;
        Ok(ResidualPlanePlan {
            block_ctx,
            tx_size: block.tx_size,
            x: block.x,
            y: block.y,
            tx,
            residual_width4: width4.max(1),
            residual_height4: height4.max(1),
            zero_corners: block.middle,
            reconstruction,
            ..*self
        })
    }

    fn transform_unit_block_ctx(
        &self,
        block: &PositionedLumaCoeffBlock,
        tx: TxShape,
        width4: usize,
        height4: usize,
    ) -> core::result::Result<BlockCtx, GeneralIntraResidualError> {
        if self.plane_id == PlaneId::Y {
            return Ok(BlockCtx::new(
                BlockRect::new(block.y >> 2, block.x >> 2, width4, height4),
                tx,
                self.block_ctx.frame_mi_cols(),
                self.block_ctx.frame_mi_rows(),
                self.block_ctx.bit_depth(),
                self.block_ctx.chroma(),
            )
            .with_tile_bounds_from(self.block_ctx));
        }
        let (sub_x, sub_y) = self.block_ctx.chroma().subsampling(self.plane_id);
        let scale_x = 1usize << sub_x;
        let scale_y = 1usize << sub_y;
        let chroma_ref = BlockRect::new(
            (block.y >> 2).checked_mul(scale_y).ok_or(
                GeneralIntraResidualError::TransformPartitionGeometry {
                    table: "Lossless_Chroma_Row",
                    index: block.tx_size,
                },
            )?,
            (block.x >> 2).checked_mul(scale_x).ok_or(
                GeneralIntraResidualError::TransformPartitionGeometry {
                    table: "Lossless_Chroma_Col",
                    index: block.tx_size,
                },
            )?,
            width4.checked_mul(scale_x).ok_or(
                GeneralIntraResidualError::TransformPartitionGeometry {
                    table: "Lossless_Chroma_Width",
                    index: block.tx_size,
                },
            )?,
            height4.checked_mul(scale_y).ok_or(
                GeneralIntraResidualError::TransformPartitionGeometry {
                    table: "Lossless_Chroma_Height",
                    index: block.tx_size,
                },
            )?,
        );
        let chroma_tx = TxShape::from_luma_4x4(chroma_ref.width4(), chroma_ref.height4()).ok_or(
            GeneralIntraResidualError::TransformPartitionGeometry {
                table: "Lossless_Chroma_Tx",
                index: block.tx_size,
            },
        )?;
        Ok(BlockCtx::new(
            self.block_ctx.block(),
            self.block_ctx.plane_block(PlaneId::Y).tx(),
            self.block_ctx.frame_mi_cols(),
            self.block_ctx.frame_mi_rows(),
            self.block_ctx.bit_depth(),
            self.block_ctx.chroma(),
        )
        .with_tile_bounds_from(self.block_ctx)
        .with_chroma_ref(chroma_ref, chroma_tx))
    }

    fn palette_color_map_for_unit(
        &self,
        parent_map: Option<&[u8]>,
        block: &PositionedLumaCoeffBlock,
    ) -> core::result::Result<Option<Vec<u8>>, GeneralIntraResidualError> {
        let Some(parent_map) = parent_map else {
            return Ok(None);
        };
        let parent_width = 1usize << self.tx.width_log2();
        let parent_height = 1usize << self.tx.height_log2();
        let expected_parent = parent_width.saturating_mul(parent_height);
        if parent_map.len() != expected_parent {
            return Err(GeneralIntraResidualError::PredictionLength {
                expected: expected_parent,
                actual: parent_map.len(),
            });
        }
        let (log2_width, log2_height) = tx_size_log2(block.tx_size)?;
        let unit_width = 1usize << log2_width;
        let unit_height = 1usize << log2_height;
        let local_x = block
            .x
            .checked_sub(self.x)
            .ok_or(GeneralIntraResidualError::UnexpectedBranch)?;
        let local_y = block
            .y
            .checked_sub(self.y)
            .ok_or(GeneralIntraResidualError::UnexpectedBranch)?;
        if local_x.saturating_add(unit_width) > parent_width
            || local_y.saturating_add(unit_height) > parent_height
        {
            return Err(GeneralIntraResidualError::UnexpectedBranch);
        }
        let mut unit_map = Vec::with_capacity(unit_width.saturating_mul(unit_height));
        for row in 0..unit_height {
            let start = (local_y + row) * parent_width + local_x;
            let end = start + unit_width;
            unit_map.extend_from_slice(&parent_map[start..end]);
        }
        Ok(Some(unit_map))
    }

    fn read_palette_color_map(
        self,
        work_unit: &mut DecodeTileWorkUnit<'_>,
        symbols: &mut SymbolDecoder<'_>,
    ) -> core::result::Result<Option<Vec<u8>>, GeneralIntraResidualError> {
        let ResidualReconstructionPlan::LumaPalette { palette, .. } = self.reconstruction else {
            return Ok(None);
        };
        let plane_width = 1usize << self.tx.width_log2();
        let plane_height = 1usize << self.tx.height_log2();
        let frame_width = self.block_ctx.frame_mi_cols().saturating_mul(4);
        let frame_height = self.block_ctx.frame_mi_rows().saturating_mul(4);
        let cols = plane_width.min(frame_width.saturating_sub(self.x));
        let rows = plane_height.min(frame_height.saturating_sub(self.y));
        let mut color_map = vec![0u8; plane_width.saturating_mul(plane_height)];
        let direction = if plane_width < 64 && plane_height < 64 {
            read_palette_literal(symbols, 1, PALETTE_DIRECTION_REASON)? != 0
        } else {
            false
        };
        let axis1_limit = if direction { rows } else { cols };
        let axis2_limit = if direction { cols } else { rows };
        let mut prev_identity_row_flag = 0usize;

        for ax2 in 0..axis2_limit {
            let ctx = if ax2 == 0 { 3 } else { prev_identity_row_flag };
            let identity_row_flag = work_unit
                .cdf_mut()
                .tile_cdfs_mut()
                .read_block_symbol_trace(TileCdfSelector::IdentityRowY { ctx }, symbols)
                .map(splot_core::symbol::Symbol::get)
                .map_err(|source| GeneralIntraResidualError::PaletteSymbolRead { source })?;
            if identity_row_flag == PALETTE_ROW_COPY_PREVIOUS && ax2 == 0 {
                return Err(GeneralIntraResidualError::PaletteInvalidIdentityRow);
            }
            for ax1 in 0..axis1_limit {
                let y = if direction { ax1 } else { ax2 };
                let x = if direction { ax2 } else { ax1 };
                let offset = y * plane_width + x;
                color_map[offset] = if identity_row_flag == PALETTE_ROW_COPY_PREVIOUS {
                    if direction {
                        color_map[y * plane_width + x - 1]
                    } else {
                        color_map[(y - 1) * plane_width + x]
                    }
                } else if identity_row_flag == PALETTE_ROW_COPY_LAST && ax1 > 0 {
                    if direction {
                        color_map[(y - 1) * plane_width + x]
                    } else {
                        color_map[y * plane_width + x - 1]
                    }
                } else if ax2 == 0 && ax1 == 0 {
                    read_palette_uniform(symbols, palette.size())? as u8
                } else {
                    let (color_ctx, color_order) =
                        palette_color_index_context(&color_map, plane_width, y, x);
                    let color_idx = work_unit
                        .cdf_mut()
                        .tile_cdfs_mut()
                        .read_block_symbol_trace(
                            TileCdfSelector::PaletteYColorIndex {
                                palette_size: palette.size(),
                                ctx: color_ctx,
                            },
                            symbols,
                        )
                        .map(splot_core::symbol::Symbol::get)
                        .map_err(|source| GeneralIntraResidualError::PaletteSymbolRead { source })?
                        as usize;
                    *color_order.get(color_idx).ok_or(
                        GeneralIntraResidualError::PaletteColorIndex {
                            color_index: color_idx,
                            palette_size: palette.size(),
                        },
                    )?
                };
            }
            prev_identity_row_flag = usize::from(identity_row_flag);
        }
        if cols != 0 && cols < plane_width {
            for y in 0..rows {
                let fill = color_map[y * plane_width + cols - 1];
                for x in cols..plane_width {
                    color_map[y * plane_width + x] = fill;
                }
            }
        }
        if rows != 0 {
            for y in rows..plane_height {
                let src = (rows - 1) * plane_width;
                let dst = y * plane_width;
                for x in 0..plane_width {
                    color_map[dst + x] = color_map[src + x];
                }
            }
        }
        Ok(Some(color_map))
    }

    const fn tx_fills_residual_block(self) -> bool {
        self.tx.width4() == self.residual_width4 && self.tx.height4() == self.residual_height4
    }

    fn unit_directional_replan(
        &self,
        luma_context: LumaTransformTypeContext,
    ) -> ResidualReconstructionPlan {
        let (directional, use_tcq) = match self.reconstruction {
            ResidualReconstructionPlan::LumaRectOneSidedAbove { use_tcq, .. }
            | ResidualReconstructionPlan::LumaRectOneSidedLeft { use_tcq, .. }
            | ResidualReconstructionPlan::LumaRectMiddle { use_tcq, .. } => (true, use_tcq),
            ResidualReconstructionPlan::LumaSquare { plan, use_tcq } => (
                matches!(
                    plan,
                    crate::prediction::intra::IntraLumaPlan::DirectionalMiddle { .. }
                        | crate::prediction::intra::IntraLumaPlan::DirectionalOneSidedAbove { .. }
                        | crate::prediction::intra::IntraLumaPlan::DirectionalOneSidedLeft { .. }
                        | crate::prediction::intra::IntraLumaPlan::DirectionalNeighbour { .. }
                ),
                use_tcq,
            ),
            _ => (false, false),
        };
        if !directional || self.plane_id != PlaneId::Y || luma_context.mrl_index() != 0 {
            return self.reconstruction;
        }
        let Some(base) = luma_context.y_mode().mode_to_angle() else {
            return self.reconstruction;
        };
        let nominal = i32::from(base) + i32::from(luma_context.angle_delta_y()) * 3;
        let unit_w = 1usize << self.tx.width_log2();
        let unit_h = 1usize << self.tx.height_log2();
        let mapped =
            crate::pipeline::general_intra::wide_angle_mapped_p_angle(unit_w, unit_h, nominal);
        let Ok(p_angle) = u16::try_from(mapped) else {
            return self.reconstruction;
        };
        if p_angle == 90 || p_angle == 180 {
            return self.reconstruction;
        }
        if p_angle < 90 {
            ResidualReconstructionPlan::LumaRectOneSidedAbove { p_angle, use_tcq }
        } else if p_angle > 180 {
            ResidualReconstructionPlan::LumaRectOneSidedLeft { p_angle, use_tcq }
        } else {
            ResidualReconstructionPlan::LumaRectMiddle { p_angle, use_tcq }
        }
    }

    fn luma_corner_neighbours(
        self,
        block_ctx: BlockCtx,
        block_decoded: &TileBlockDecodedState,
    ) -> NeighbourAvailability {
        let neighbours = block_ctx.neighbours_from_block_decoded(PlaneId::Y, block_decoded);
        if self.zero_corners {
            neighbours.without_corners()
        } else {
            neighbours
        }
    }

    fn plane_neighbours(
        self,
        block_ctx: BlockCtx,
        block_decoded: &TileBlockDecodedState,
    ) -> NeighbourAvailability {
        block_ctx.neighbours_from_block_decoded(self.plane_id, block_decoded)
    }

    #[allow(clippy::too_many_arguments)]
    fn reconstruct<T: ReconSample>(
        self,
        workspace: &mut CurrentFrameWorkspace<T>,
        coeffs: &crate::bitstream::tile_payload::LumaCoeffBlock,
        block_decoded: &TileBlockDecodedState,
        palette_color_map: Option<&[u8]>,
        qindex: u32,
        intra_edge: crate::prediction::intra_edge::IntraEdgeCtx,
        luma_context: LumaTransformTypeContext,
    ) -> core::result::Result<(), GeneralIntraResidualError> {
        let block_ctx = self.block_ctx;
        match self.unit_directional_replan(luma_context) {
            ResidualReconstructionPlan::LumaPalette { palette, use_tcq } => {
                let color_map =
                    palette_color_map.ok_or(GeneralIntraResidualError::UnexpectedBranch)?;
                crate::pipeline::reconstruct::reconstruct_general_intra_luma_palette_block_into(
                    workspace,
                    coeffs,
                    palette,
                    color_map,
                    self.x,
                    self.y,
                    self.tx.width_log2(),
                    self.tx.height_log2(),
                    qindex,
                    use_tcq,
                    luma_context,
                    block_ctx.bit_depth(),
                )
            }
            ResidualReconstructionPlan::LumaSquare { plan, use_tcq } => plan.reconstruct(
                workspace,
                coeffs,
                block_ctx,
                block_decoded,
                qindex,
                use_tcq,
                intra_edge.enable_ibp,
                luma_context,
            ),
            ResidualReconstructionPlan::LumaRectSmooth { mode, use_tcq } => {
                let neighbours = self.luma_corner_neighbours(block_ctx, block_decoded);
                let edges = block_ctx.neighbours(PlaneId::Y);
                crate::pipeline::reconstruct::reconstruct_general_intra_luma_smooth_rect_block_with_availability_into(
                    workspace,
                    coeffs,
                    mode,
                    self.x,
                    self.y,
                    self.tx.width_log2(),
                    self.tx.height_log2(),
                    qindex,
                    use_tcq,
                    neighbours.num_above_right(),
                    neighbours.num_below_left(),
                    Some(luma_context),
                    EdgeAvail::new(edges.has_above(), edges.has_left()),
                    block_ctx.bit_depth(),
                )
            }
            ResidualReconstructionPlan::LumaRectDip {
                mode,
                transpose,
                use_tcq,
            } => {
                let neighbours = self.luma_corner_neighbours(block_ctx, block_decoded);
                let edges = block_ctx.neighbours(PlaneId::Y);
                crate::pipeline::reconstruct::reconstruct_general_intra_luma_dip_rect_block_into(
                    workspace,
                    coeffs,
                    mode,
                    transpose,
                    self.x,
                    self.y,
                    self.tx.width_log2(),
                    self.tx.height_log2(),
                    qindex,
                    use_tcq,
                    neighbours.num_above_right(),
                    neighbours.num_below_left(),
                    luma_context,
                    EdgeAvail::new(edges.has_above(), edges.has_left()),
                    block_ctx.bit_depth(),
                )
            }
            ResidualReconstructionPlan::LumaRectMiddle { p_angle, use_tcq } => {
                let (w, h) = (1u32 << self.tx.width_log2(), 1u32 << self.tx.height_log2());
                let apply_ibp = intra_edge.enable_ibp && !(w == 4 && h == 4);
                let edges = crate::prediction::intra_edge::UnitEdges {
                    above: block_ctx.neighbours(PlaneId::Y).has_above(),
                    left: block_ctx.neighbours(PlaneId::Y).has_left(),
                };
                let edge_filters = crate::prediction::intra_edge::unit_middle_edge_filters(
                    intra_edge,
                    workspace,
                    PlaneId::Y,
                    i32::from(p_angle),
                    apply_ibp,
                    edges,
                    self.x,
                    self.y,
                    w,
                    h,
                )?;
                crate::pipeline::reconstruct::reconstruct_general_intra_middle_neighbour_rect_block_into(
                    workspace,
                    coeffs,
                    p_angle,
                    PlaneId::Y,
                    self.x,
                    self.y,
                    self.tx.width_log2(),
                    self.tx.height_log2(),
                    qindex,
                    use_tcq,
                    Some(luma_context),
                    block_ctx.bit_depth(),
                    MiddleAvail { above: edges.above, left: edges.left },
                    edge_filters,
                )
            }
            ResidualReconstructionPlan::LumaRectMiddleMrl {
                p_angle,
                mrl_index,
                above_mrl_index,
                is_sb_boundary,
                secondary_mrl,
                use_tcq,
            } => {
                let edges = block_ctx.neighbours(PlaneId::Y);
                crate::pipeline::reconstruct::reconstruct_general_intra_two_sided_middle_luma_mrl_block_into(
                    workspace,
                    coeffs,
                    p_angle,
                    self.x,
                    self.y,
                    self.tx.width_log2(),
                    self.tx.height_log2(),
                    qindex,
                    mrl_index,
                    above_mrl_index,
                    is_sb_boundary,
                    secondary_mrl,
                    use_tcq,
                    Some(luma_context),
                    MiddleAvail::new(edges.has_above(), edges.has_left()),
                    block_ctx.bit_depth(),
                )
            }
            ResidualReconstructionPlan::LumaRectOneSidedAboveMrl {
                p_angle,
                mrl_index,
                above_mrl_index,
                secondary_mrl,
                use_tcq,
            } => {
                let neighbours = self.luma_corner_neighbours(block_ctx, block_decoded);
                let edges = block_ctx.neighbours(PlaneId::Y);
                let availability = EdgeAvail::new(edges.has_above(), edges.has_left());
                let mrl = AboveMrl {
                    mrl_index,
                    above_mrl_index,
                };
                if secondary_mrl {
                    crate::pipeline::reconstruct::reconstruct_general_intra_mrl_secondary_above_block_into(
                        workspace,
                        coeffs,
                        p_angle,
                        self.x,
                        self.y,
                        self.tx.width_log2(),
                        self.tx.height_log2(),
                        qindex,
                        neighbours.num_above_right(),
                        mrl,
                        use_tcq,
                        availability,
                        block_ctx.bit_depth(),
                    )
                } else {
                    crate::pipeline::reconstruct::reconstruct_general_intra_one_sided_neighbour_block_into(
                        workspace,
                        coeffs,
                        p_angle,
                        PlaneId::Y,
                        self.x,
                        self.y,
                        self.tx.width_log2(),
                        self.tx.height_log2(),
                        qindex,
                        neighbours.num_above_right(),
                        mrl,
                        use_tcq,
                        Some(luma_context),
                        availability,
                        block_ctx.bit_depth(),
                        crate::pipeline::reconstruct::OneSidedEdgeFilter::default(),
                    )
                }
            }
            ResidualReconstructionPlan::LumaRectOneSidedAbove { p_angle, use_tcq } => {
                let neighbours = self.luma_corner_neighbours(block_ctx, block_decoded);
                let (w, h) = (1u32 << self.tx.width_log2(), 1u32 << self.tx.height_log2());
                let apply_ibp = intra_edge.enable_ibp && !(w == 4 && h == 4);
                let edges = crate::prediction::intra_edge::UnitEdges {
                    above: block_ctx.neighbours(PlaneId::Y).has_above(),
                    left: block_ctx.neighbours(PlaneId::Y).has_left(),
                };
                let edge_filter = crate::prediction::intra_edge::unit_edge_filter(
                    intra_edge,
                    workspace,
                    i32::from(p_angle),
                    crate::prediction::intra_edge::UnitEdgeRole::Primary { apply_ibp },
                    edges,
                    self.x,
                    self.y,
                    w,
                    h,
                )?;
                if apply_ibp && luma_context.angle_delta_y() % 2 == 0 && edges.left {
                    let secondary_filter = crate::prediction::intra_edge::unit_edge_filter(
                        intra_edge,
                        workspace,
                        i32::from(p_angle),
                        crate::prediction::intra_edge::UnitEdgeRole::IbpSecondary,
                        edges,
                        self.x,
                        self.y,
                        w,
                        h,
                    )?;
                    crate::pipeline::reconstruct::reconstruct_general_intra_one_sided_ibp_luma_block_into(
                        workspace,
                        coeffs,
                        p_angle,
                        PlaneId::Y,
                        self.x,
                        self.y,
                        self.tx.width_log2(),
                        self.tx.height_log2(),
                        qindex,
                        neighbours.num_above_right(),
                        edge_filter,
                        crate::pipeline::reconstruct::IbpSecondary {
                            second_angle: p_angle + 180,
                            edge_filter: secondary_filter,
                            num4_far: neighbours.num_below_left(),
                        },
                        edges.above,
                        use_tcq,
                        Some(luma_context),
                        block_ctx.bit_depth(),
                    )
                } else {
                    let availability = EdgeAvail::new(edges.above, edges.left);
                    crate::pipeline::reconstruct::reconstruct_general_intra_one_sided_neighbour_block_into(
                        workspace,
                        coeffs,
                        p_angle,
                        PlaneId::Y,
                        self.x,
                        self.y,
                        self.tx.width_log2(),
                        self.tx.height_log2(),
                        qindex,
                        neighbours.num_above_right(),
                        crate::pipeline::reconstruct::OneSidedAboveMrl::default(),
                        use_tcq,
                        Some(luma_context),
                        availability,
                        block_ctx.bit_depth(),
                        edge_filter,
                    )
                }
            }
            ResidualReconstructionPlan::LumaRectOneSidedLeftMrl {
                p_angle,
                mrl_index,
                secondary_mrl,
                use_tcq,
            } => {
                let neighbours = self.luma_corner_neighbours(block_ctx, block_decoded);
                let edges = block_ctx.neighbours(PlaneId::Y);
                let availability = EdgeAvail::new(edges.has_above(), edges.has_left());
                if secondary_mrl {
                    crate::pipeline::reconstruct::reconstruct_general_intra_mrl_secondary_left_block_into(
                        workspace,
                        coeffs,
                        p_angle,
                        self.x,
                        self.y,
                        self.tx.width_log2(),
                        self.tx.height_log2(),
                        qindex,
                        neighbours.num_below_left(),
                        block_ctx.neighbours(PlaneId::Y).has_above(),
                        mrl_index,
                        use_tcq,
                        availability.left,
                        block_ctx.bit_depth(),
                    )
                } else {
                    crate::pipeline::reconstruct::reconstruct_general_intra_one_sided_left_neighbour_block_into(
                        workspace,
                        coeffs,
                        p_angle,
                        PlaneId::Y,
                        self.x,
                        self.y,
                        self.tx.width_log2(),
                        self.tx.height_log2(),
                        qindex,
                        neighbours.num_below_left(),
                        block_ctx.neighbours(PlaneId::Y).has_above(),
                        mrl_index,
                        use_tcq,
                        Some(luma_context),
                        availability,
                        block_ctx.bit_depth(),
                        crate::pipeline::reconstruct::OneSidedEdgeFilter::default(),
                    )
                }
            }
            ResidualReconstructionPlan::LumaRectCardinalMrl {
                direction,
                mrl_index,
                above_mrl_index,
                secondary_mrl,
                use_tcq,
            } => {
                crate::pipeline::reconstruct::reconstruct_general_intra_cardinal_mrl_luma_block_into(
                    workspace,
                    coeffs,
                    direction,
                    self.x,
                    self.y,
                    self.tx.width_log2(),
                    self.tx.height_log2(),
                    qindex,
                    mrl_index,
                    above_mrl_index,
                    secondary_mrl,
                    use_tcq,
                    block_ctx.bit_depth(),
                )
            }
            ResidualReconstructionPlan::LumaRectOneSidedLeft { p_angle, use_tcq } => {
                let neighbours = self.luma_corner_neighbours(block_ctx, block_decoded);
                let (w, h) = (1u32 << self.tx.width_log2(), 1u32 << self.tx.height_log2());
                let apply_ibp = intra_edge.enable_ibp && !(w == 4 && h == 4);
                let edges = crate::prediction::intra_edge::UnitEdges {
                    above: block_ctx.neighbours(PlaneId::Y).has_above(),
                    left: block_ctx.neighbours(PlaneId::Y).has_left(),
                };
                let edge_filter = crate::prediction::intra_edge::unit_edge_filter(
                    intra_edge,
                    workspace,
                    i32::from(p_angle),
                    crate::prediction::intra_edge::UnitEdgeRole::Primary { apply_ibp },
                    edges,
                    self.x,
                    self.y,
                    w,
                    h,
                )?;
                if apply_ibp && luma_context.angle_delta_y() % 2 == 0 {
                    let secondary_filter = crate::prediction::intra_edge::unit_edge_filter(
                        intra_edge,
                        workspace,
                        i32::from(p_angle),
                        crate::prediction::intra_edge::UnitEdgeRole::IbpSecondary,
                        edges,
                        self.x,
                        self.y,
                        w,
                        h,
                    )?;
                    crate::pipeline::reconstruct::reconstruct_general_intra_one_sided_ibp_luma_block_into(
                        workspace,
                        coeffs,
                        p_angle,
                        PlaneId::Y,
                        self.x,
                        self.y,
                        self.tx.width_log2(),
                        self.tx.height_log2(),
                        qindex,
                        neighbours.num_below_left(),
                        edge_filter,
                        crate::pipeline::reconstruct::IbpSecondary {
                            second_angle: p_angle - 180,
                            edge_filter: secondary_filter,
                            num4_far: neighbours.num_above_right(),
                        },
                        edges.above,
                        use_tcq,
                        Some(luma_context),
                        block_ctx.bit_depth(),
                    )
                } else {
                    let availability = EdgeAvail::new(edges.above, edges.left);
                    crate::pipeline::reconstruct::reconstruct_general_intra_one_sided_left_neighbour_block_into(
                        workspace,
                        coeffs,
                        p_angle,
                        PlaneId::Y,
                        self.x,
                        self.y,
                        self.tx.width_log2(),
                        self.tx.height_log2(),
                        qindex,
                        neighbours.num_below_left(),
                        block_ctx.neighbours(PlaneId::Y).has_above(),
                        0,
                        use_tcq,
                        Some(luma_context),
                        availability,
                        block_ctx.bit_depth(),
                        edge_filter,
                    )
                }
            }
            ResidualReconstructionPlan::LumaRectCardinal { direction, use_tcq } => {
                let neighbours = block_ctx.neighbours(PlaneId::Y);
                crate::pipeline::reconstruct::reconstruct_general_intra_cardinal_neighbour_block_into(
                    workspace,
                    coeffs,
                    direction,
                    PlaneId::Y,
                    self.x,
                    self.y,
                    self.tx.width_log2(),
                    self.tx.height_log2(),
                    qindex,
                    use_tcq,
                    Some(luma_context),
                    None,
                    EdgeAvail::new(neighbours.has_above(), neighbours.has_left()),
                    block_ctx.bit_depth(),
                )
            }
            ResidualReconstructionPlan::LumaRectPaeth { use_tcq } => {
                let neighbours = block_ctx.neighbours(PlaneId::Y);
                crate::pipeline::reconstruct::reconstruct_general_intra_luma_paeth_neighbour_block_into(
                    workspace,
                    coeffs,
                    PlaneId::Y,
                    self.x,
                    self.y,
                    self.tx.width_log2(),
                    self.tx.height_log2(),
                    qindex,
                    use_tcq,
                    EdgeAvail::new(neighbours.has_above(), neighbours.has_left()),
                    block_ctx.bit_depth(),
                )
            }
            ResidualReconstructionPlan::ChromaOneSided { p_angle } => {
                let neighbours = self.plane_neighbours(block_ctx, block_decoded);
                let (w, h) = (1u32 << self.tx.width_log2(), 1u32 << self.tx.height_log2());
                let apply_ibp = intra_edge.enable_ibp && !(w == 4 && h == 4);
                let edges = crate::prediction::intra_edge::UnitEdges {
                    above: neighbours.has_above(),
                    left: neighbours.has_left(),
                };
                let edge_filter = crate::prediction::intra_edge::unit_edge_filter_for_plane(
                    intra_edge.chroma(),
                    workspace,
                    self.plane_id,
                    i32::from(p_angle),
                    crate::prediction::intra_edge::UnitEdgeRole::Primary { apply_ibp },
                    edges,
                    self.x,
                    self.y,
                    w,
                    h,
                )?;
                let availability = EdgeAvail::new(neighbours.has_above(), neighbours.has_left());
                if p_angle < 90 {
                    crate::pipeline::reconstruct::reconstruct_general_intra_one_sided_neighbour_block_into(
                        workspace,
                        coeffs,
                        p_angle,
                        self.plane_id,
                        self.x,
                        self.y,
                        self.tx.width_log2(),
                        self.tx.height_log2(),
                        qindex,
                        neighbours.num_above_right(),
                        crate::pipeline::reconstruct::OneSidedAboveMrl::default(),
                        false,
                        None,
                        availability,
                        block_ctx.bit_depth(),
                        edge_filter,
                    )
                } else {
                    crate::pipeline::reconstruct::reconstruct_general_intra_one_sided_left_neighbour_block_into(
                        workspace,
                        coeffs,
                        p_angle,
                        self.plane_id,
                        self.x,
                        self.y,
                        self.tx.width_log2(),
                        self.tx.height_log2(),
                        qindex,
                        neighbours.num_below_left(),
                        neighbours.has_above(),
                        0,
                        false,
                        None,
                        availability,
                        block_ctx.bit_depth(),
                        edge_filter,
                    )
                }
            }
            ResidualReconstructionPlan::ChromaMiddle { p_angle } => {
                let neighbours = self.plane_neighbours(block_ctx, block_decoded);
                let (w, h) = (1u32 << self.tx.width_log2(), 1u32 << self.tx.height_log2());
                let apply_ibp = intra_edge.enable_ibp && !(w == 4 && h == 4);
                let edges = crate::prediction::intra_edge::UnitEdges {
                    above: neighbours.has_above(),
                    left: neighbours.has_left(),
                };
                let edge_filters = crate::prediction::intra_edge::unit_middle_edge_filters(
                    intra_edge.chroma(),
                    workspace,
                    self.plane_id,
                    i32::from(p_angle),
                    apply_ibp,
                    edges,
                    self.x,
                    self.y,
                    w,
                    h,
                )?;
                crate::pipeline::reconstruct::reconstruct_general_intra_middle_neighbour_rect_block_into(
                    workspace,
                    coeffs,
                    p_angle,
                    self.plane_id,
                    self.x,
                    self.y,
                    self.tx.width_log2(),
                    self.tx.height_log2(),
                    qindex,
                    false,
                    None,
                    block_ctx.bit_depth(),
                    MiddleAvail { above: edges.above, left: edges.left },
                    edge_filters,
                )
            }
            ResidualReconstructionPlan::Chroma { mode, dpcm } => {
                let neighbours = self.plane_neighbours(block_ctx, block_decoded);
                crate::pipeline::reconstruct::reconstruct_general_intra_chroma_block_into(
                    workspace,
                    coeffs,
                    self.plane_id,
                    self.x,
                    self.y,
                    self.tx.width_log2(),
                    self.tx.height_log2(),
                    qindex,
                    mode,
                    dpcm,
                    neighbours.num_above_right(),
                    neighbours.num_below_left(),
                    intra_edge.enable_ibp
                        && !(self.tx.width_log2() == 2 && self.tx.height_log2() == 2),
                    EdgeAvail::new(neighbours.has_above(), neighbours.has_left()),
                    block_ctx.bit_depth(),
                )
            }
            ResidualReconstructionPlan::ChromaCfl {
                params,
                cfl_ds_filter_index,
                sb_mib,
            } => {
                let neighbours = self.plane_neighbours(block_ctx, block_decoded);
                crate::pipeline::reconstruct::reconstruct_general_intra_chroma_cfl_block_into(
                    workspace,
                    coeffs,
                    self.plane_id,
                    self.x,
                    self.y,
                    self.tx.width_log2(),
                    self.tx.height_log2(),
                    qindex,
                    params,
                    cfl_ds_filter_index,
                    sb_mib,
                    neighbours.num_above_right(),
                    neighbours.num_below_left(),
                    block_ctx.bit_depth(),
                )
            }
            ResidualReconstructionPlan::Rect { use_tcq } => {
                let ibp_dc = intra_edge.enable_ibp
                    && !(self.tx.width_log2() == 2 && self.tx.height_log2() == 2);
                let neighbours = self.plane_neighbours(block_ctx, block_decoded);
                crate::pipeline::reconstruct::reconstruct_general_intra_block_rect_with_availability_into(
                    workspace,
                    coeffs,
                    self.plane_id,
                    self.x,
                    self.y,
                    self.tx.width_log2(),
                    self.tx.height_log2(),
                    qindex,
                    use_tcq,
                    ibp_dc,
                    (self.plane_id == PlaneId::Y).then_some(luma_context),
                    EdgeAvail::new(neighbours.has_above(), neighbours.has_left()),
                    block_ctx.bit_depth(),
                )
            }
        }
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

fn tx_size_log2(tx_size: usize) -> core::result::Result<(u32, u32), GeneralIntraResidualError> {
    let error = |table| GeneralIntraResidualError::TransformPartitionGeometry {
        table,
        index: tx_size,
    };
    let width = *TX_WIDTH_LOG2
        .get(tx_size)
        .ok_or_else(|| error("Tx_Width_Log2"))?;
    let height = *TX_HEIGHT_LOG2
        .get(tx_size)
        .ok_or_else(|| error("Tx_Height_Log2"))?;
    Ok((
        u32::try_from(width).map_err(|_| error("Tx_Width_Log2"))?,
        u32::try_from(height).map_err(|_| error("Tx_Height_Log2"))?,
    ))
}

fn chroma_angle_delta_uv(
    plane_id: PlaneId,
    uv_mode: usize,
    luma_transform_type_context: LumaTransformTypeContext,
) -> i32 {
    if matches!(plane_id, PlaneId::U | PlaneId::V)
        && uv_mode == luma_transform_type_context.y_mode().value()
    {
        i32::from(luma_transform_type_context.angle_delta_y())
    } else {
        0
    }
}

fn read_palette_uniform(
    symbols: &mut SymbolDecoder<'_>,
    num_values: usize,
) -> core::result::Result<usize, GeneralIntraResidualError> {
    let bits = unsigned_bits(num_values);
    if bits == 0 {
        return Ok(0);
    }
    let m = (1usize << bits) - num_values;
    let value = read_palette_literal(symbols, (bits - 1) as u32, PALETTE_UNIFORM_REASON)? as usize;
    if value < m {
        Ok(value)
    } else {
        let extra = read_palette_literal(symbols, 1, PALETTE_UNIFORM_REASON)? as usize;
        Ok((value << 1) - m + extra)
    }
}

fn read_palette_literal(
    symbols: &mut SymbolDecoder<'_>,
    bits: u32,
    reason: &'static str,
) -> core::result::Result<u32, GeneralIntraResidualError> {
    symbols
        .read_literal(bits)
        .map_err(|source| GeneralIntraResidualError::PaletteLiteral { reason, source })
}

const fn unsigned_bits(num_values: usize) -> usize {
    if num_values == 0 {
        0
    } else {
        usize::BITS as usize - num_values.leading_zeros() as usize
    }
}

fn palette_color_index_context(
    color_map: &[u8],
    stride: usize,
    row: usize,
    col: usize,
) -> (usize, [u8; PALETTE_MAX_SIZE]) {
    let mut color_order = [0u8; PALETTE_MAX_SIZE];
    let mut color_status = [false; PALETTE_MAX_SIZE];
    for (index, value) in color_order.iter_mut().enumerate() {
        *value = index as u8;
    }
    let mut color_count = 0usize;
    let color_index_ctx = if row > 0 && col > 0 {
        let left = usize::from(color_map[row * stride + col - 1]);
        let top_left = usize::from(color_map[(row - 1) * stride + col - 1]);
        let top = usize::from(color_map[(row - 1) * stride + col]);
        if left == top_left && left == top {
            swap_color_order(
                &mut color_order,
                &mut color_status,
                0,
                left,
                &mut color_count,
            );
            4
        } else if left == top {
            swap_color_order(
                &mut color_order,
                &mut color_status,
                0,
                left,
                &mut color_count,
            );
            swap_color_order(
                &mut color_order,
                &mut color_status,
                1,
                top_left,
                &mut color_count,
            );
            3
        } else if left == top_left {
            swap_color_order(
                &mut color_order,
                &mut color_status,
                0,
                left,
                &mut color_count,
            );
            swap_color_order(
                &mut color_order,
                &mut color_status,
                1,
                top,
                &mut color_count,
            );
            2
        } else if top_left == top {
            swap_color_order(
                &mut color_order,
                &mut color_status,
                0,
                top,
                &mut color_count,
            );
            swap_color_order(
                &mut color_order,
                &mut color_status,
                1,
                left,
                &mut color_count,
            );
            2
        } else {
            swap_color_order(
                &mut color_order,
                &mut color_status,
                0,
                left,
                &mut color_count,
            );
            swap_color_order(
                &mut color_order,
                &mut color_status,
                1,
                top,
                &mut color_count,
            );
            swap_color_order(
                &mut color_order,
                &mut color_status,
                2,
                top_left,
                &mut color_count,
            );
            1
        }
    } else if col == 0 && row > 0 {
        let top = usize::from(color_map[(row - 1) * stride + col]);
        swap_color_order(
            &mut color_order,
            &mut color_status,
            0,
            top,
            &mut color_count,
        );
        0
    } else if col > 0 && row == 0 {
        let left = usize::from(color_map[row * stride + col - 1]);
        swap_color_order(
            &mut color_order,
            &mut color_status,
            0,
            left,
            &mut color_count,
        );
        0
    } else {
        0
    };
    let mut write_idx = color_count;
    for (read_idx, status) in color_status.iter().enumerate() {
        if !status && write_idx < color_order.len() {
            color_order[write_idx] = read_idx as u8;
            write_idx += 1;
        }
    }
    debug_assert!(color_index_ctx < PALETTE_COLOR_CONTEXTS);
    (color_index_ctx, color_order)
}

fn swap_color_order(
    color_order: &mut [u8; PALETTE_MAX_SIZE],
    color_status: &mut [bool; PALETTE_MAX_SIZE],
    switch_idx: usize,
    max_idx: usize,
    color_count: &mut usize,
) {
    if switch_idx < color_order.len() && max_idx < color_status.len() {
        color_order[switch_idx] = max_idx as u8;
        color_status[max_idx] = true;
        *color_count += 1;
    }
}

fn push_ordered_planes(
    planes: &mut Vec<ResidualPlanePlan>,
    block_ctx: BlockCtx,
    luma_reconstruction: ResidualReconstructionPlan,
    chroma_reconstruction: Option<ResidualReconstructionPlan>,
    luma_fsc_mode: bool,
    luma_lossless_tx_size: Option<usize>,
    lossless: bool,
) -> core::result::Result<(), ResidualPipelineUnsupported> {
    let block = block_ctx.block();
    let width_chunks = (block.width4() >> 4).max(1);
    let height_chunks = (block.height4() >> 4).max(1);
    let (sub_x, sub_y) = block_ctx.chroma().subsampling(PlaneId::U);
    let double_chroma_w = sub_x != 0 && width_chunks > 1 && !lossless;
    let double_chroma_h = sub_y != 0 && height_chunks > 1 && !lossless;
    let defer_chroma_reconstruction = chroma_reconstruction
        .is_some_and(chroma_depends_on_complete_luma)
        && (width_chunks > 1 || height_chunks > 1);

    for start_chunk_y in (0..height_chunks).step_by(2) {
        for start_chunk_x in (0..width_chunks).step_by(2) {
            for chunk_y in start_chunk_y..(start_chunk_y + 2).min(height_chunks) {
                for chunk_x in start_chunk_x..(start_chunk_x + 2).min(width_chunks) {
                    planes.push(ResidualPlanePlan::new(
                        residual_chunk_ctx(block_ctx, chunk_x, chunk_y, 1, 1)?,
                        PlaneId::Y,
                        luma_reconstruction,
                        block.width4(),
                        block.height4(),
                        luma_fsc_mode,
                        luma_fsc_mode,
                        luma_lossless_tx_size,
                    )?);
                    if let Some(reconstruction) = chroma_reconstruction
                        && (!double_chroma_w || chunk_x.is_multiple_of(2))
                        && (!double_chroma_h || chunk_y.is_multiple_of(2))
                    {
                        let chunk_width = if double_chroma_w { 2 } else { 1 };
                        let chunk_height = if double_chroma_h { 2 } else { 1 };
                        let chroma_ctx = if lossless || (sub_x == 0 && sub_y == 0) {
                            residual_chunk_ctx(
                                block_ctx,
                                chunk_x,
                                chunk_y,
                                chunk_width,
                                chunk_height,
                            )?
                        } else {
                            block_ctx
                        };
                        planes.extend(chroma_plans(
                            chroma_ctx,
                            reconstruction,
                            luma_fsc_mode,
                            defer_chroma_reconstruction,
                        )?);
                    }
                }
            }
        }
    }
    Ok(())
}

const fn chroma_depends_on_complete_luma(reconstruction: ResidualReconstructionPlan) -> bool {
    matches!(reconstruction, ResidualReconstructionPlan::ChromaCfl { .. })
}

fn residual_chunk_ctx(
    block_ctx: BlockCtx,
    chunk_x: usize,
    chunk_y: usize,
    chunk_width: usize,
    chunk_height: usize,
) -> core::result::Result<BlockCtx, ResidualPipelineUnsupported> {
    let block = block_ctx.block();
    let offset_x4 = chunk_x
        .checked_mul(CHUNK_64_N4)
        .ok_or(UNSUPPORTED_LARGE_BLOCK_CHUNK_GEOMETRY)?;
    let offset_y4 = chunk_y
        .checked_mul(CHUNK_64_N4)
        .ok_or(UNSUPPORTED_LARGE_BLOCK_CHUNK_GEOMETRY)?;
    let width4 = block
        .width4()
        .checked_sub(offset_x4)
        .ok_or(UNSUPPORTED_LARGE_BLOCK_CHUNK_GEOMETRY)?
        .min(CHUNK_64_N4.saturating_mul(chunk_width));
    let height4 = block
        .height4()
        .checked_sub(offset_y4)
        .ok_or(UNSUPPORTED_LARGE_BLOCK_CHUNK_GEOMETRY)?
        .min(CHUNK_64_N4.saturating_mul(chunk_height));
    let row4 = block
        .row4()
        .checked_add(offset_y4)
        .ok_or(UNSUPPORTED_LARGE_BLOCK_CHUNK_GEOMETRY)?;
    let col4 = block
        .col4()
        .checked_add(offset_x4)
        .ok_or(UNSUPPORTED_LARGE_BLOCK_CHUNK_GEOMETRY)?;
    let tx =
        TxShape::from_luma_4x4(width4, height4).ok_or(UNSUPPORTED_LARGE_BLOCK_CHUNK_GEOMETRY)?;
    Ok(BlockCtx::new(
        BlockRect::new(row4, col4, width4, height4),
        tx,
        block_ctx.frame_mi_cols(),
        block_ctx.frame_mi_rows(),
        block_ctx.bit_depth(),
        block_ctx.chroma(),
    )
    .with_tile_bounds_from(block_ctx))
}

fn chroma_plans(
    block_ctx: BlockCtx,
    reconstruction: ResidualReconstructionPlan,
    txb_skip_fsc_mode: bool,
    defer_reconstruction: bool,
) -> core::result::Result<[ResidualPlanePlan; 2], ResidualPipelineUnsupported> {
    let [u, v] = CHROMA_PLANES.map(|plane_id| {
        let block = block_ctx.plane_block(plane_id);
        let plan = ResidualPlanePlan::new(
            block_ctx,
            plane_id,
            reconstruction,
            block.width4(),
            block.height4(),
            false,
            txb_skip_fsc_mode,
            None,
        )?;
        Ok(if defer_reconstruction {
            plan.with_deferred_reconstruction()
        } else {
            plan
        })
    });
    Ok([u?, v?])
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

const fn coeff_plane(plane_id: PlaneId) -> usize {
    match plane_id {
        PlaneId::Y => 0,
        PlaneId::U => 1,
        PlaneId::V => 2,
    }
}

fn tx_size_for_plan(
    tx: TxShape,
    plane_id: PlaneId,
) -> core::result::Result<usize, ResidualPipelineUnsupported> {
    tx.square_tx_index()
        .or_else(|| rect_tx_size_from_log2(tx.width_log2(), tx.height_log2()))
        .ok_or_else(|| unsupported_tx_size(plane_id))
}

fn rect_tx_size_from_log2(w_log2: u32, h_log2: u32) -> Option<usize> {
    let w = i32::try_from(w_log2).ok()?;
    let h = i32::try_from(h_log2).ok()?;
    TX_WIDTH_LOG2
        .iter()
        .zip(TX_HEIGHT_LOG2.iter())
        .position(|(&tw, &th)| tw == w && th == h)
}

const fn unsupported_tx_size(plane_id: PlaneId) -> ResidualPipelineUnsupported {
    match plane_id {
        PlaneId::Y => unsupported(
            "general_intra_rect_tx_size",
            missing_capability_message!("intra.rect.tx_size", table = "missing"),
            crate::pipeline::GENERAL_INTRA_PARTITION_SPEC_SECTION,
        ),
        PlaneId::U | PlaneId::V => unsupported(
            "general_intra_rect_chroma_tx_size",
            missing_capability_message!("intra.rect.chroma_tx_size", table = "missing"),
            crate::pipeline::GENERAL_INTRA_PARTITION_SPEC_SECTION,
        ),
    }
}

const UNSUPPORTED_LARGE_BLOCK_CHUNK_GEOMETRY: ResidualPipelineUnsupported = unsupported(
    "general_intra_large_block_chunk_geometry",
    missing_capability_message!("intra.large_block.chunk_geometry"),
    crate::pipeline::GENERAL_INTRA_PARTITION_SPEC_SECTION,
);

const fn unsupported(
    reason_id: &'static str,
    message: &'static str,
    spec_section: &'static str,
) -> ResidualPipelineUnsupported {
    ResidualPipelineUnsupported {
        reason_id,
        message,
        spec_section,
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic)]
mod tests;
