// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Per-transform-unit residual replanning.

use splot_core::tables::conversion::{TX_HEIGHT_LOG2, TX_WIDTH_LOG2};
use splot_recon::PlaneId;

use crate::bitstream::tile_payload::{
    DecodeTileWorkUnit, GeneralIntraResidualError, PositionedLumaCoeffBlock,
    current_frame_qm_segment_id,
};
use crate::tile::block_context::{BlockCtx, BlockRect, TxShape};

use super::{ResidualPlanePlan, ResidualReconstructionPlan, TX_4X4};

impl ResidualReconstructionPlan {
    pub(super) fn for_luma_transform_row(self, is_parent_row: bool) -> Self {
        if is_parent_row {
            return self;
        }
        match self {
            Self::LumaRectOneSidedLeftMrl {
                p_angle,
                mrl_index,
                secondary_mrl,
                use_tcq,
                ..
            } => Self::LumaRectOneSidedLeftMrl {
                p_angle,
                mrl_index,
                above_mrl_index: mrl_index,
                is_sb_boundary: false,
                secondary_mrl,
                use_tcq,
            },
            Self::LumaRectOneSidedAboveMrl {
                p_angle,
                mrl_index,
                secondary_mrl,
                use_tcq,
                ..
            } => Self::LumaRectOneSidedAboveMrl {
                p_angle,
                mrl_index,
                above_mrl_index: mrl_index,
                secondary_mrl,
                use_tcq,
            },
            Self::LumaRectCardinalMrl {
                direction,
                mrl_index,
                secondary_mrl,
                use_tcq,
                ..
            } => Self::LumaRectCardinalMrl {
                direction,
                mrl_index,
                above_mrl_index: mrl_index,
                secondary_mrl,
                use_tcq,
            },
            Self::LumaRectMiddleMrl {
                p_angle,
                mrl_index,
                secondary_mrl,
                use_tcq,
                ..
            } => Self::LumaRectMiddleMrl {
                p_angle,
                mrl_index,
                above_mrl_index: mrl_index,
                is_sb_boundary: false,
                secondary_mrl,
                use_tcq,
            },
            _ => self,
        }
    }
}

impl ResidualPlanePlan {
    pub(super) fn lossless_transform_unit_tx_size(
        self,
        work_unit: &DecodeTileWorkUnit<'_>,
    ) -> Option<usize> {
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

    pub(super) fn lossless_unit_starts_in_frame(self, x: usize, y: usize) -> bool {
        let (sub_x, sub_y) = self.block_ctx.chroma().subsampling(self.plane_id);
        let max_x = (self.block_ctx.frame_mi_cols() * 4) >> sub_x;
        let max_y = (self.block_ctx.frame_mi_rows() * 4) >> sub_y;
        x < max_x && y < max_y
    }

    pub(super) fn transform_unit_plan(
        &self,
        block: &PositionedLumaCoeffBlock,
    ) -> core::result::Result<ResidualPlanePlan, GeneralIntraResidualError> {
        let reconstruction = self
            .reconstruction
            .for_luma_transform_row(block.y == self.y);
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
            (block.y >> 2) * scale_y,
            (block.x >> 2) * scale_x,
            width4 * scale_x,
            height4 * scale_y,
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

    pub(super) const fn tx_fills_residual_block(self) -> bool {
        self.tx.width4() == self.residual_width4 && self.tx.height4() == self.residual_height4
    }
}

pub(super) fn tx_size_log2(
    tx_size: usize,
) -> core::result::Result<(u32, u32), GeneralIntraResidualError> {
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
