// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Runtime residual transform dispatch.

use splot_core::symbol::SymbolDecoder;
use splot_core::tables::conversion::{TX_HEIGHT_LOG2, TX_WIDTH_LOG2};
use splot_recon::{CurrentFrameWorkspace, IntraCardinalDirection, PlaneId, ReconSample};

use super::block_context::{BlockCtx, BlockRect, TxShape};
use super::capability::missing_capability_message;
use super::intra_prediction::IntraLumaPlan;
use crate::tile_payload::{
    CflParams, DecodeTileWorkUnit, GeneralIntraResidualError, LumaPalette,
    LumaTransformPartitionContext, LumaTransformTypeContext, PositionedLumaCoeffBlock,
    SupportedChromaMode, SupportedNonDcLumaMode, TileBlockDecodedState, TileCdfSelector,
    TileCoeffContextState, TransformToolResidualPolicy, decode_general_intra_luma_partition_coeffs,
};

const CHROMA_PLANES: [PlaneId; 2] = [PlaneId::U, PlaneId::V];
const CHUNK_64_N4: usize = 16;
const PALETTE_MAX_SIZE: usize = 8;
const PALETTE_COLOR_CONTEXTS: usize = 5;
const PALETTE_ROW_COPY_PREVIOUS: u8 = 2;
const PALETTE_ROW_COPY_LAST: u8 = 1;
const PALETTE_DIRECTION_REASON: &str = "palette_direction";
const PALETTE_UNIFORM_REASON: &str = "palette_color_idx_uniform";

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct GeneralIntraResidualPlan {
    planes: Vec<ResidualPlanePlan>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct ResidualBlockTransforms {
    luma_tx: usize,
    chroma_tx: Option<usize>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum RectLumaPlan {
    Palette {
        palette: LumaPalette,
        use_tcq: bool,
    },
    Dc {
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
pub(super) enum RectChromaPlan {
    Mode(SupportedChromaMode),
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
    pub(super) const fn chroma_tx(self) -> Option<usize> {
        self.chroma_tx
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct ResidualPipelineUnsupported {
    reason_id: &'static str,
    message: &'static str,
    spec_section: &'static str,
}

impl ResidualPipelineUnsupported {
    pub(super) const fn reason_id(self) -> &'static str {
        self.reason_id
    }

    pub(super) const fn message(self) -> &'static str {
        self.message
    }

    pub(super) const fn spec_section(self) -> &'static str {
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
    /// § 5.20.7.24 `allowCorners == 0` (a § 5.20.6.3 middle transform unit):
    /// the top-right/bottom-left availability counts read as zero.
    zero_corners: bool,
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
    pub(super) fn square(
        block_ctx: BlockCtx,
        luma_plan: IntraLumaPlan,
        chroma_plan: Option<RectChromaPlan>,
        luma_use_tcq: bool,
        luma_fsc_mode: bool,
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
        )?;
        Ok(Self { planes })
    }

    pub(super) fn rect(
        block_ctx: BlockCtx,
        luma_plan: RectLumaPlan,
        chroma_plan: Option<RectChromaPlan>,
        luma_fsc_mode: bool,
    ) -> core::result::Result<Self, ResidualPipelineUnsupported> {
        let mut planes = Vec::new();
        let chroma_reconstruction = chroma_plan.map(chroma_reconstruction);
        let luma_reconstruction = match luma_plan {
            RectLumaPlan::Palette { palette, use_tcq } => {
                ResidualReconstructionPlan::LumaPalette { palette, use_tcq }
            }
            RectLumaPlan::Dc { use_tcq } => ResidualReconstructionPlan::Rect { use_tcq },
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
        )?;
        Ok(Self { planes })
    }

    pub(super) fn chroma(
        block_ctx: BlockCtx,
        chroma_plan: RectChromaPlan,
    ) -> core::result::Result<Self, ResidualPipelineUnsupported> {
        let reconstruction = chroma_reconstruction(chroma_plan);
        let mut planes = Vec::new();
        planes.extend(chroma_plans(block_ctx, reconstruction, false)?);
        Ok(Self { planes })
    }

    pub(super) fn transforms(&self) -> ResidualBlockTransforms {
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
    pub(super) fn execute<T: ReconSample>(
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
        intra_edge: super::intra_edge::IntraEdgeCtx,
        deblock: &mut DeblockRecorder<'_>,
    ) -> core::result::Result<(), GeneralIntraResidualError> {
        let mut execute =
            |plane: ResidualPlanePlan, eob_u_nonzero, deblock: &mut DeblockRecorder<'_>| {
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
            };

        let mut u_nonzero = false;
        for &plane in &self.planes {
            let eob_u_nonzero = plane.plane_id == PlaneId::V && u_nonzero;
            let coeffs = execute(plane, eob_u_nonzero, deblock)?;
            if plane.plane_id == PlaneId::U {
                u_nonzero = !coeffs.all_zero;
            }
        }
        Ok(())
    }

    #[cfg(test)]
    fn plane_plan(&self, plane_id: PlaneId) -> Option<ResidualPlanePlan> {
        self.planes
            .iter()
            .find(|plane| plane.plane_id == plane_id)
            .copied()
    }
}

/// Collects one § 7.17 deblock record per executed LUMA transform unit —
/// the spec's `DeblockingTxSizes` are per 4x4 unit, so interior transform
/// edges of a multi-unit block must be visible to the § 7.17.2 edge walk.
/// Chroma keeps the block's single transform (`chroma_tx` at the block
/// origin) on every record.
pub(super) struct DeblockRecorder<'a> {
    pub(super) blocks: &'a mut Vec<super::deblock::DeblockBlock>,
    pub(super) block_r: usize,
    pub(super) block_c: usize,
    pub(super) chroma_tx: Option<usize>,
}

impl DeblockRecorder<'_> {
    fn record_luma_unit(
        &mut self,
        r: usize,
        c: usize,
        n4w: usize,
        n4h: usize,
        luma_tx: usize,
        qindex: u32,
    ) {
        self.blocks.push(super::deblock::DeblockBlock {
            r,
            c,
            block_r: self.block_r,
            block_c: self.block_c,
            chroma_base_r: self.block_r,
            chroma_base_c: self.block_c,
            n4w,
            n4h,
            luma_tx,
            chroma_tx: self.chroma_tx,
            qindex,
            skip: false,
        });
    }
}

fn chroma_reconstruction(plan: RectChromaPlan) -> ResidualReconstructionPlan {
    match plan {
        RectChromaPlan::Mode(mode) => ResidualReconstructionPlan::Chroma { mode },
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
    fn new(
        block_ctx: BlockCtx,
        plane_id: PlaneId,
        reconstruction: ResidualReconstructionPlan,
        residual_width4: usize,
        residual_height4: usize,
        fsc_mode: bool,
        txb_skip_fsc_mode: bool,
    ) -> core::result::Result<Self, ResidualPipelineUnsupported> {
        let block = block_ctx.plane_block(plane_id);
        let tx = block.tx();
        Ok(Self {
            plane_id,
            block_ctx,
            coeff_plane: coeff_plane(plane_id),
            tx_size: tx_size_for_plan(tx, plane_id)?,
            x: block.x(),
            y: block.y(),
            tx,
            residual_width4,
            residual_height4,
            fsc_mode,
            txb_skip_fsc_mode,
            zero_corners: false,
            reconstruction,
        })
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
        intra_edge: super::intra_edge::IntraEdgeCtx,
        deblock: &mut DeblockRecorder<'_>,
    ) -> core::result::Result<crate::tile_payload::LumaCoeffBlock, GeneralIntraResidualError> {
        let policy = transform_tool_policy_for_plane(
            transform_tool_residual_policy,
            self.plane_id,
            luma_transform_type_context,
        );
        let angle_delta_uv =
            chroma_angle_delta_uv(self.plane_id, uv_mode, luma_transform_type_context);
        let palette_color_map = self.read_palette_color_map(work_unit, symbols)?;
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
        let trace_bits = crate::trace_flags::trace_flag!("SPLOT_TRACE_GENERAL_INTRA_BITS");
        let start_bits = symbols.consumed_bits().get();
        let coeffs = crate::tile_payload::decode_general_intra_plane_coeffs(
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
            false,
            self.fsc_mode,
            self.txb_skip_fsc_mode,
            policy,
        )?;
        if trace_bits {
            eprintln!(
                "general intra residual plane block={:?} plane={:?} tx_size={} xy=({}, {}) fsc={} bits={}..{} all_zero={} eob={} tx_type={} recon={:?}",
                self.block_ctx.block(),
                self.plane_id,
                self.tx_size,
                self.x,
                self.y,
                self.fsc_mode,
                start_bits,
                symbols.consumed_bits().get(),
                coeffs.all_zero,
                coeffs.eob,
                coeffs.plane_tx_type,
                self.reconstruction
            );
        }
        if self.plane_id == PlaneId::Y {
            deblock.record_luma_unit(
                self.y / 4,
                self.x / 4,
                self.residual_width4,
                self.residual_height4,
                self.tx_size,
                qindex,
            );
        }
        self.reconstruct(
            workspace,
            &coeffs,
            block_decoded,
            palette_color_map.as_deref(),
            qindex,
            intra_edge,
            luma_transform_type_context,
        )?;
        Ok(coeffs)
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
        intra_edge: super::intra_edge::IntraEdgeCtx,
        luma_context: LumaTransformTypeContext,
        deblock: &mut DeblockRecorder<'_>,
    ) -> core::result::Result<crate::tile_payload::LumaCoeffBlock, GeneralIntraResidualError> {
        let trace_bits = crate::trace_flags::trace_flag!("SPLOT_TRACE_GENERAL_INTRA_BITS");
        let start_bits = symbols.consumed_bits().get();
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
            if trace_bits {
                eprintln!(
                    "general intra residual plane block={:?} plane={:?} tx_size={} xy=({}, {}) fsc={} bits={}..{} all_zero={} eob={} tx_type={} recon={:?}",
                    self.block_ctx.block(),
                    self.plane_id,
                    block.tx_size,
                    block.x,
                    block.y,
                    self.fsc_mode,
                    start_bits,
                    symbols.consumed_bits().get(),
                    block.coeffs.all_zero,
                    block.coeffs.eob,
                    block.coeffs.plane_tx_type,
                    self.reconstruction
                );
            }
            let (log2_width, log2_height) = tx_size_log2(block.tx_size)?;
            deblock.record_luma_unit(
                block.y / 4,
                block.x / 4,
                ((1usize << log2_width) / 4).max(1),
                ((1usize << log2_height) / 4).max(1),
                block.tx_size,
                qindex,
            );
            self.reconstruct(
                workspace,
                &block.coeffs,
                block_decoded,
                palette_color_map,
                qindex,
                intra_edge,
                luma_context,
            )?;
            return Ok(block.coeffs);
        }

        let sb_mask = block_decoded.sb_size4().saturating_sub(1);
        for block in &blocks {
            let unit = self.transform_unit_plan(block)?;
            unit.reconstruct(
                workspace,
                &block.coeffs,
                block_decoded,
                palette_color_map,
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
                qindex,
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
        if trace_bits {
            eprintln!(
                "general intra residual partition block={:?} plane={:?} tx_size={} xy=({}, {}) parts={} fsc={} bits={}..{} all_zero={} eob={} tx_type={} recon={:?}",
                self.block_ctx.block(),
                self.plane_id,
                self.tx_size,
                self.x,
                self.y,
                blocks.len(),
                self.fsc_mode,
                start_bits,
                symbols.consumed_bits().get(),
                summary.all_zero,
                summary.eob,
                summary.plane_tx_type,
                self.reconstruction
            );
        }
        Ok(summary)
    }

    /// Re-scopes this plan to one § 5.20.7.24 transform unit: each unit runs
    /// the same prediction process as a standalone block of the unit's
    /// geometry, reading edges from the just-reconstructed workspace (so
    /// interior units see sibling-unit samples) and the per-unit-maintained
    /// `BlockDecoded` counts. The above-row MRL read offset zeroes only at a
    /// superblock boundary; interior units sit below sibling units, never at
    /// one, so they read the full `MrlIndex` line (AVM `above_mrl_idx` rule).
    /// § 5.20.6.3 `LumaTxMiddle` units pass `allowCorners = 0` (§ 5.20.7.24),
    /// zeroing the top-right/bottom-left counts; modes consuming those counts
    /// defer until the zeroed-count variant lands.
    fn transform_unit_plan(
        &self,
        block: &PositionedLumaCoeffBlock,
    ) -> core::result::Result<ResidualPlanePlan, GeneralIntraResidualError> {
        let reconstruction = match self.reconstruction {
            ResidualReconstructionPlan::Rect { .. }
            | ResidualReconstructionPlan::LumaRectCardinal { .. }
            | ResidualReconstructionPlan::LumaRectPaeth { .. }
            | ResidualReconstructionPlan::LumaRectSmooth { .. }
            | ResidualReconstructionPlan::LumaRectMiddle { .. }
            | ResidualReconstructionPlan::LumaRectOneSidedAbove { .. }
            | ResidualReconstructionPlan::LumaRectOneSidedLeft { .. }
            | ResidualReconstructionPlan::LumaRectOneSidedLeftMrl { .. } => self.reconstruction,
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
                plan: IntraLumaPlan::DirectionalNeighbour { mode },
                use_tcq,
            } => {
                let p_angle = super::intra_prediction::directional_mode_p_angle(mode);
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
            _ => {
                if crate::trace_flags::trace_flag!("SPLOT_TRACE_GENERAL_INTRA_BITS") {
                    eprintln!(
                        "general intra partition defer recon={:?} xy=({}, {}) unit=({}, {}, tx={})",
                        self.reconstruction, self.x, self.y, block.x, block.y, block.tx_size
                    );
                }
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
        let block_ctx = BlockCtx::new(
            BlockRect::new(block.y >> 2, block.x >> 2, width4.max(1), height4.max(1)),
            tx,
            self.block_ctx.frame_mi_cols(),
            self.block_ctx.frame_mi_rows(),
            self.block_ctx.bit_depth(),
            self.block_ctx.chroma(),
        );
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

    /// § 5.20.7.24 availability counts for the luma prediction arms, zeroed
    /// when `allowCorners == 0` for this transform unit.
    /// AVM re-derives the § 5.20.7.29 WAIP wide-angle remap inside every
    /// per-TU `av2_predict_intra_block` call with the UNIT's dimensions
    /// (`wide_angle_mapping`, reconintra.h:220-257), so a unit of a
    /// directional block can land in a different zone than the block-level
    /// plan. Re-derive this unit's `pAngle` from the coded mode and
    /// re-select the directional arm; square single-unit directional plans
    /// re-plan onto the rect arms so the §7.13.2.7 / §7.13.2.9 machinery
    /// serves them. MRL plans keep their block-level arm (their `pAngle`
    /// carries the MRL delta and their arms carry per-unit MRL state).
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
                    super::intra_prediction::IntraLumaPlan::DirectionalMiddle { .. }
                        | super::intra_prediction::IntraLumaPlan::DirectionalOneSidedAbove { .. }
                        | super::intra_prediction::IntraLumaPlan::DirectionalOneSidedLeft { .. }
                        | super::intra_prediction::IntraLumaPlan::DirectionalNeighbour { .. }
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
        let mapped = super::general_intra::wide_angle_mapped_p_angle(unit_w, unit_h, nominal);
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

    /// § 5.20.7.24 availability counts for the luma prediction arms, zeroed
    /// when `allowCorners == 0` for this transform unit.
    fn luma_corner_neighbours(
        self,
        block_ctx: BlockCtx,
        block_decoded: &TileBlockDecodedState,
    ) -> super::block_context::NeighbourAvailability {
        let neighbours = block_ctx.neighbours_from_block_decoded(PlaneId::Y, block_decoded);
        if self.zero_corners {
            neighbours.without_corners()
        } else {
            neighbours
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn reconstruct<T: ReconSample>(
        self,
        workspace: &mut CurrentFrameWorkspace<T>,
        coeffs: &crate::tile_payload::LumaCoeffBlock,
        block_decoded: &TileBlockDecodedState,
        palette_color_map: Option<&[u8]>,
        qindex: u32,
        intra_edge: super::intra_edge::IntraEdgeCtx,
        luma_context: LumaTransformTypeContext,
    ) -> core::result::Result<(), GeneralIntraResidualError> {
        let block_ctx = self.block_ctx;
        match self.unit_directional_replan(luma_context) {
            ResidualReconstructionPlan::LumaPalette { palette, use_tcq } => {
                let color_map = palette_color_map.ok_or(GeneralIntraResidualError::UnexpectedBranch)?;
                crate::runtime_minimal_recon::reconstruct_general_intra_luma_palette_block_into(
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
            ),
            ResidualReconstructionPlan::LumaRectSmooth { mode, use_tcq } => {
                let neighbours = self.luma_corner_neighbours(block_ctx, block_decoded);
                crate::runtime_minimal_recon::reconstruct_general_intra_luma_smooth_rect_block_into(
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
                    None,
                    block_ctx.bit_depth(),
                )
            }
            ResidualReconstructionPlan::LumaRectMiddle { p_angle, use_tcq } => {
                let (w, h) = (1u32 << self.tx.width_log2(), 1u32 << self.tx.height_log2());
                let apply_ibp = intra_edge.enable_ibp && !(w == 4 && h == 4);
                let edges = super::intra_edge::UnitEdges {
                    above: block_ctx.neighbours(PlaneId::Y).has_above(),
                    left: block_ctx.neighbours(PlaneId::Y).has_left(),
                };
                let edge_filters = super::intra_edge::unit_middle_edge_filters(
                    intra_edge,
                    workspace,
                    i32::from(p_angle),
                    apply_ibp,
                    edges,
                    self.x,
                    self.y,
                    w,
                    h,
                )?;
                crate::runtime_minimal_recon::reconstruct_general_intra_middle_neighbour_rect_block_into(
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
                    None,
                    block_ctx.bit_depth(),
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
                crate::runtime_minimal_recon::reconstruct_general_intra_two_sided_middle_luma_mrl_block_into(
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
                    None,
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
                let mrl = crate::runtime_minimal_recon::OneSidedAboveMrl {
                    mrl_index,
                    above_mrl_index,
                };
                if secondary_mrl {
                    crate::runtime_minimal_recon::reconstruct_general_intra_mrl_secondary_above_block_into(
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
                        block_ctx.bit_depth(),
                    )
                } else {
                    crate::runtime_minimal_recon::reconstruct_general_intra_one_sided_neighbour_block_into(
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
                        None,
                        block_ctx.bit_depth(),
                        crate::runtime_minimal_recon::OneSidedEdgeFilter::default(),
                    )
                }
            }
            ResidualReconstructionPlan::LumaRectOneSidedAbove { p_angle, use_tcq } => {
                let neighbours = self.luma_corner_neighbours(block_ctx, block_decoded);
                let (w, h) = (1u32 << self.tx.width_log2(), 1u32 << self.tx.height_log2());
                let apply_ibp = intra_edge.enable_ibp && !(w == 4 && h == 4);
                let edges = super::intra_edge::UnitEdges {
                    above: block_ctx.neighbours(PlaneId::Y).has_above(),
                    left: block_ctx.neighbours(PlaneId::Y).has_left(),
                };
                let edge_filter = super::intra_edge::unit_edge_filter(
                    intra_edge,
                    workspace,
                    i32::from(p_angle),
                    super::intra_edge::UnitEdgeRole::Primary { apply_ibp },
                    edges,
                    self.x,
                    self.y,
                    w,
                    h,
                )?;
                if apply_ibp && luma_context.angle_delta_y() % 2 == 0 && edges.left {
                    let secondary_filter = super::intra_edge::unit_edge_filter(
                        intra_edge,
                        workspace,
                        i32::from(p_angle),
                        super::intra_edge::UnitEdgeRole::IbpSecondary,
                        edges,
                        self.x,
                        self.y,
                        w,
                        h,
                    )?;
                    crate::runtime_minimal_recon::reconstruct_general_intra_one_sided_ibp_luma_block_into(
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
                        crate::runtime_minimal_recon::IbpSecondary {
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
                    crate::runtime_minimal_recon::reconstruct_general_intra_one_sided_neighbour_block_into(
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
                        crate::runtime_minimal_recon::OneSidedAboveMrl::default(),
                        use_tcq,
                        None,
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
                if secondary_mrl {
                    crate::runtime_minimal_recon::reconstruct_general_intra_mrl_secondary_left_block_into(
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
                        block_ctx.bit_depth(),
                    )
                } else {
                    crate::runtime_minimal_recon::reconstruct_general_intra_one_sided_left_neighbour_block_into(
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
                        None,
                        block_ctx.bit_depth(),
                        crate::runtime_minimal_recon::OneSidedEdgeFilter::default(),
                    )
                }
            }
            ResidualReconstructionPlan::LumaRectCardinalMrl {
                direction,
                mrl_index,
                above_mrl_index,
                secondary_mrl,
                use_tcq,
            } => crate::runtime_minimal_recon::reconstruct_general_intra_cardinal_mrl_luma_block_into(
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
            ),
            ResidualReconstructionPlan::LumaRectOneSidedLeft { p_angle, use_tcq } => {
                let neighbours = self.luma_corner_neighbours(block_ctx, block_decoded);
                let (w, h) = (1u32 << self.tx.width_log2(), 1u32 << self.tx.height_log2());
                let apply_ibp = intra_edge.enable_ibp && !(w == 4 && h == 4);
                let edges = super::intra_edge::UnitEdges {
                    above: block_ctx.neighbours(PlaneId::Y).has_above(),
                    left: block_ctx.neighbours(PlaneId::Y).has_left(),
                };
                let edge_filter = super::intra_edge::unit_edge_filter(
                    intra_edge,
                    workspace,
                    i32::from(p_angle),
                    super::intra_edge::UnitEdgeRole::Primary { apply_ibp },
                    edges,
                    self.x,
                    self.y,
                    w,
                    h,
                )?;
                if apply_ibp && luma_context.angle_delta_y() % 2 == 0 {
                    let secondary_filter = super::intra_edge::unit_edge_filter(
                        intra_edge,
                        workspace,
                        i32::from(p_angle),
                        super::intra_edge::UnitEdgeRole::IbpSecondary,
                        edges,
                        self.x,
                        self.y,
                        w,
                        h,
                    )?;
                    crate::runtime_minimal_recon::reconstruct_general_intra_one_sided_ibp_luma_block_into(
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
                        crate::runtime_minimal_recon::IbpSecondary {
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
                    crate::runtime_minimal_recon::reconstruct_general_intra_one_sided_left_neighbour_block_into(
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
                        None,
                        block_ctx.bit_depth(),
                        edge_filter,
                    )
                }
            }
            ResidualReconstructionPlan::LumaRectCardinal { direction, use_tcq } => {
                crate::runtime_minimal_recon::reconstruct_general_intra_cardinal_neighbour_block_into(
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
                    None,
                    block_ctx.bit_depth(),
                )
            }
            ResidualReconstructionPlan::LumaRectPaeth { use_tcq } => {
                crate::runtime_minimal_recon::reconstruct_general_intra_luma_paeth_neighbour_block_into(
                    workspace,
                    coeffs,
                    PlaneId::Y,
                    self.x,
                    self.y,
                    self.tx.width_log2(),
                    self.tx.height_log2(),
                    qindex,
                    use_tcq,
                    block_ctx.bit_depth(),
                )
            }
            ResidualReconstructionPlan::ChromaMiddle { p_angle } => {
                crate::runtime_minimal_recon::reconstruct_general_intra_middle_neighbour_rect_block_into(
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
                    middle_edge_filters(),
                )
            }
            ResidualReconstructionPlan::Chroma { mode } => {
                let neighbours = block_ctx.neighbours(PlaneId::U);
                crate::runtime_minimal_recon::reconstruct_general_intra_chroma_block_into(
                    workspace,
                    coeffs,
                    self.plane_id,
                    self.x,
                    self.y,
                    self.tx.width_log2(),
                    self.tx.height_log2(),
                    qindex,
                    mode,
                    neighbours.num_above_right(),
                    neighbours.num_below_left(),
                    block_ctx.bit_depth(),
                )
            }
            ResidualReconstructionPlan::ChromaCfl {
                params,
                cfl_ds_filter_index,
                sb_mib,
            } => {
                let neighbours = block_ctx.neighbours(PlaneId::U);
                crate::runtime_minimal_recon::reconstruct_general_intra_chroma_cfl_block_into(
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
                    && self.plane_id == PlaneId::Y
                    && !(self.tx.width_log2() == 2 && self.tx.height_log2() == 2);
                crate::runtime_minimal_recon::reconstruct_general_intra_block_rect_into(
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
                    block_ctx.bit_depth(),
                )
            }
        }
    }
}

fn summarize_luma_partition(
    blocks: &[PositionedLumaCoeffBlock],
) -> crate::tile_payload::LumaCoeffBlock {
    crate::tile_payload::LumaCoeffBlock {
        all_zero: blocks.iter().all(|block| block.coeffs.all_zero),
        eob: blocks
            .iter()
            .fold(0usize, |sum, block| sum.saturating_add(block.coeffs.eob)),
        quant: Vec::new(),
        intra_ist: blocks.iter().find_map(|block| block.coeffs.intra_ist),
        plane_tx_type: blocks
            .iter()
            .find(|block| !block.coeffs.all_zero)
            .or_else(|| blocks.first())
            .map_or(0, |block| block.coeffs.plane_tx_type),
    }
}

fn tx_size_log2(tx_size: usize) -> core::result::Result<(u32, u32), GeneralIntraResidualError> {
    let width = *TX_WIDTH_LOG2.get(tx_size).ok_or(
        GeneralIntraResidualError::TransformPartitionGeometry {
            table: "Tx_Width_Log2",
            index: tx_size,
        },
    )?;
    let height = *TX_HEIGHT_LOG2.get(tx_size).ok_or(
        GeneralIntraResidualError::TransformPartitionGeometry {
            table: "Tx_Height_Log2",
            index: tx_size,
        },
    )?;
    let width = u32::try_from(width).map_err(|_| {
        GeneralIntraResidualError::TransformPartitionGeometry {
            table: "Tx_Width_Log2",
            index: tx_size,
        }
    })?;
    let height = u32::try_from(height).map_err(|_| {
        GeneralIntraResidualError::TransformPartitionGeometry {
            table: "Tx_Height_Log2",
            index: tx_size,
        }
    })?;
    Ok((width, height))
}

fn middle_edge_filters() -> crate::runtime_minimal_recon::TwoSidedMiddleEdgeFilters {
    crate::runtime_minimal_recon::TwoSidedMiddleEdgeFilters {
        above: crate::runtime_minimal_recon::OneSidedEdgeFilter::default(),
        left: crate::runtime_minimal_recon::OneSidedEdgeFilter::default(),
    }
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
) -> core::result::Result<(), ResidualPipelineUnsupported> {
    let block = block_ctx.block();
    let width_chunks = (block.width4() >> 4).max(1);
    let height_chunks = (block.height4() >> 4).max(1);

    for start_chunk_y in (0..height_chunks).step_by(2) {
        for start_chunk_x in (0..width_chunks).step_by(2) {
            for chunk_y in start_chunk_y..(start_chunk_y + 2).min(height_chunks) {
                for chunk_x in start_chunk_x..(start_chunk_x + 2).min(width_chunks) {
                    planes.push(ResidualPlanePlan::new(
                        luma_chunk_ctx(block_ctx, chunk_x, chunk_y)?,
                        PlaneId::Y,
                        luma_reconstruction,
                        block.width4(),
                        block.height4(),
                        luma_fsc_mode,
                        luma_fsc_mode,
                    )?);
                    if let Some(reconstruction) = chroma_reconstruction
                        && chunk_x % 2 == 0
                        && chunk_y % 2 == 0
                    {
                        planes.extend(chroma_plans(block_ctx, reconstruction, luma_fsc_mode)?);
                    }
                }
            }
        }
    }
    Ok(())
}

fn luma_chunk_ctx(
    block_ctx: BlockCtx,
    chunk_x: usize,
    chunk_y: usize,
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
        .min(CHUNK_64_N4);
    let height4 = block
        .height4()
        .checked_sub(offset_y4)
        .ok_or(UNSUPPORTED_LARGE_BLOCK_CHUNK_GEOMETRY)?
        .min(CHUNK_64_N4);
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
    ))
}

fn chroma_plans(
    block_ctx: BlockCtx,
    reconstruction: ResidualReconstructionPlan,
    txb_skip_fsc_mode: bool,
) -> core::result::Result<[ResidualPlanePlan; 2], ResidualPipelineUnsupported> {
    let [u, v] = CHROMA_PLANES.map(|plane_id| {
        let block = block_ctx.plane_block(plane_id);
        ResidualPlanePlan::new(
            block_ctx,
            plane_id,
            reconstruction,
            block.width4(),
            block.height4(),
            false,
            txb_skip_fsc_mode,
        )
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
            super::GENERAL_INTRA_PARTITION_SPEC_SECTION,
        ),
        PlaneId::U | PlaneId::V => unsupported(
            "general_intra_rect_chroma_tx_size",
            missing_capability_message!("intra.rect.chroma_tx_size", table = "missing"),
            super::GENERAL_INTRA_PARTITION_SPEC_SECTION,
        ),
    }
}

const UNSUPPORTED_LARGE_BLOCK_CHUNK_GEOMETRY: ResidualPipelineUnsupported = unsupported(
    "general_intra_large_block_chunk_geometry",
    missing_capability_message!("intra.large_block.chunk_geometry"),
    super::GENERAL_INTRA_PARTITION_SPEC_SECTION,
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
mod tests {
    use super::super::block_context::{BlockRect, ChromaSampling, TxShape};
    use super::*;
    use splot_recon::BitDepth;

    #[derive(Clone, Copy)]
    struct Case {
        label: &'static str,
        rect: BlockRect,
        bit_depth: BitDepth,
        plane: PlaneId,
        expected_tx_log2: (u32, u32),
        expect_chroma: bool,
    }

    #[test]
    fn plans_square_and_rectangular_residual_planes() {
        let cases = [
            Case {
                label: "square luma 8-bit",
                rect: BlockRect::new(0, 0, 16, 16),
                bit_depth: BitDepth::Eight,
                plane: PlaneId::Y,
                expected_tx_log2: (6, 6),
                expect_chroma: true,
            },
            Case {
                label: "square chroma-u 10-bit",
                rect: BlockRect::new(0, 0, 16, 16),
                bit_depth: BitDepth::Ten,
                plane: PlaneId::U,
                expected_tx_log2: (5, 5),
                expect_chroma: true,
            },
            Case {
                label: "square chroma-v dependency",
                rect: BlockRect::new(0, 0, 16, 16),
                bit_depth: BitDepth::Eight,
                plane: PlaneId::V,
                expected_tx_log2: (5, 5),
                expect_chroma: true,
            },
            Case {
                label: "rect luma",
                rect: BlockRect::new(0, 0, 16, 8),
                bit_depth: BitDepth::Eight,
                plane: PlaneId::Y,
                expected_tx_log2: (6, 5),
                expect_chroma: true,
            },
            Case {
                label: "rect chroma-u",
                rect: BlockRect::new(0, 0, 16, 8),
                bit_depth: BitDepth::Ten,
                plane: PlaneId::U,
                expected_tx_log2: (5, 4),
                expect_chroma: true,
            },
            Case {
                label: "rect chroma-v dependency",
                rect: BlockRect::new(0, 0, 16, 8),
                bit_depth: BitDepth::Eight,
                plane: PlaneId::V,
                expected_tx_log2: (5, 4),
                expect_chroma: true,
            },
        ];

        for case in cases {
            assert_case(case);
        }
    }

    #[test]
    fn omits_chroma_plans_for_luma_only_blocks() {
        let block = BlockRect::new(0, 0, 16, 8);
        let ctx = ctx(block, BitDepth::Eight);
        let plan =
            GeneralIntraResidualPlan::rect(ctx, RectLumaPlan::Dc { use_tcq: true }, None, false)
                .expect("rect luma plan");
        assert!(plan.plane_plan(PlaneId::U).is_none());
        assert!(plan.plane_plan(PlaneId::V).is_none());
        assert_eq!(plan.transforms().chroma_tx(), None);
    }

    #[test]
    fn fsc_coefficients_are_luma_only() {
        let block = BlockRect::new(0, 0, 16, 16);
        let ctx = ctx(block, BitDepth::Ten);
        let plan = GeneralIntraResidualPlan::square(
            ctx,
            IntraLumaPlan::Dc,
            Some(RectChromaPlan::Mode(SupportedChromaMode::Dc)),
            true,
            true,
        )
        .expect("square fsc plan");

        assert!(plan.plane_plan(PlaneId::Y).expect("luma").fsc_mode);
        assert!(!plan.plane_plan(PlaneId::U).expect("chroma u").fsc_mode);
        assert!(!plan.plane_plan(PlaneId::V).expect("chroma v").fsc_mode);
        assert!(
            plan.plane_plan(PlaneId::U)
                .expect("chroma u")
                .txb_skip_fsc_mode
        );
        assert!(
            plan.plane_plan(PlaneId::V)
                .expect("chroma v")
                .txb_skip_fsc_mode
        );
    }

    #[test]
    fn large_luma_chunks_do_not_fill_parent_residual_block() {
        let block = BlockRect::new(0, 0, 32, 16);
        let ctx = ctx(block, BitDepth::Ten);
        let plan =
            GeneralIntraResidualPlan::rect(ctx, RectLumaPlan::Dc { use_tcq: true }, None, false)
                .expect("rect luma plan");
        let luma: Vec<_> = plan
            .planes
            .iter()
            .filter(|plane| plane.plane_id == PlaneId::Y)
            .copied()
            .collect();

        assert_eq!(luma.len(), 2);
        assert!(
            luma.iter()
                .all(|plane| (plane.tx.width4(), plane.tx.height4()) == (16, 16))
        );
        assert!(luma.iter().all(|plane| !plane.tx_fills_residual_block()));
        assert!(
            luma.iter()
                .all(|plane| (plane.residual_width4, plane.residual_height4) == (32, 16))
        );
    }

    #[test]
    fn chroma_angle_delta_tracks_directional_follow_mode() {
        let luma =
            LumaTransformTypeContext::new(crate::tile_payload::IntraYMode::D135_PRED_FOR_TEST, -3);

        assert_eq!(
            chroma_angle_delta_uv(
                PlaneId::U,
                crate::tile_payload::IntraYMode::D135_PRED_FOR_TEST.value(),
                luma,
            ),
            -3
        );
        assert_eq!(
            chroma_angle_delta_uv(
                PlaneId::V,
                crate::tile_payload::IntraYMode::DC_PRED.value(),
                luma
            ),
            0
        );
        assert_eq!(
            chroma_angle_delta_uv(
                PlaneId::Y,
                crate::tile_payload::IntraYMode::D135_PRED_FOR_TEST.value(),
                luma,
            ),
            0
        );
    }

    fn assert_case(case: Case) {
        let ctx = ctx(case.rect, case.bit_depth);
        let plan = if case.rect.width4() == case.rect.height4() {
            GeneralIntraResidualPlan::square(
                ctx,
                IntraLumaPlan::Dc,
                Some(RectChromaPlan::Mode(SupportedChromaMode::Dc)),
                true,
                false,
            )
        } else {
            GeneralIntraResidualPlan::rect(
                ctx,
                RectLumaPlan::Dc { use_tcq: true },
                case.expect_chroma
                    .then_some(RectChromaPlan::Mode(SupportedChromaMode::Dc)),
                false,
            )
        }
        .unwrap_or_else(|error| panic!("{}: {}", case.label, error.reason_id()));
        let plane = plan
            .plane_plan(case.plane)
            .unwrap_or_else(|| panic!("{}: missing plane", case.label));
        assert_eq!(
            plane.tx.width_log2(),
            case.expected_tx_log2.0,
            "{}",
            case.label
        );
        assert_eq!(
            plane.tx.height_log2(),
            case.expected_tx_log2.1,
            "{}",
            case.label
        );
        assert_eq!(plane.coeff_plane, coeff_plane(case.plane), "{}", case.label);
    }

    fn ctx(block: BlockRect, bit_depth: BitDepth) -> BlockCtx {
        let tx = TxShape::from_luma_4x4(block.width4(), block.height4()).expect("test tx shape");
        BlockCtx::new(block, tx, 32, 32, bit_depth, ChromaSampling::Yuv420)
    }
}
