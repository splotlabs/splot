// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Per-transform-unit residual replanning.

use splot_core::tables::conversion::{TX_HEIGHT_LOG2, TX_WIDTH_LOG2};
use splot_recon::{CurrentFrameWorkspace, PlaneId, ReconSample};

use crate::bitstream::tile_payload::{
    CoeffBlock, DecodeTileWorkUnit, GeneralIntraResidualError, LumaCoeffBlock,
    LumaTransformTypeContext, LumaTxPartition, PositionedLumaCoeffBlock, TileBlockDecodedState,
    current_frame_qm_segment_id, luma_partition_record_fills_block,
    luma_transform_record_starts_in_frame, luma_transform_records_for_partition,
};
use crate::tile::block_context::{BlockCtx, BlockRect, TxShape};

use super::{
    GeneralIntraResidualPlan, RectLumaPlan, ResidualPlanePlan, ResidualReconstructionPlan, TX_4X4,
};

impl GeneralIntraResidualPlan {
    /// Reconstructs planned transform block `index` from its row-major adjusted
    /// `quant` and `eob`, through the decoder's own kernel dispatch.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn reconstruct_unit<T: ReconSample>(
        &self,
        index: usize,
        quant: &[i32],
        eob: usize,
        plane_tx_type: usize,
        scratch: &mut crate::pipeline::general_intra::GeneralIntraReconScratch<T>,
        workspace: &mut CurrentFrameWorkspace<T>,
        block_decoded: &TileBlockDecodedState,
        qindex: u32,
        intra_edge: crate::prediction::intra_edge::IntraEdgeCtx,
        luma_context: LumaTransformTypeContext,
    ) -> core::result::Result<(), GeneralIntraResidualError> {
        let plane = self.unit(index)?;
        let nonzero = quant
            .iter()
            .rposition(|&q| q != 0)
            .map_or(0, |last| last + 1);
        let mut coeffs = LumaCoeffBlock {
            eob,
            quant_range: 0..nonzero,
            zero_tail: quant.len() - nonzero,
            ..LumaCoeffBlock::empty(plane_tx_type, false)
        };
        plane.apply_reconstruction_tx_type(&mut coeffs);
        plane.reconstruct(
            scratch,
            workspace,
            CoeffBlock::new(&coeffs, quant)?,
            block_decoded,
            None,
            qindex,
            intra_edge,
            luma_context,
        )
    }

    /// Replaces the first plan, the luma plane of a block of at most 64x64, with
    /// the in-frame units of the § 5.20.6.3 `partition` of its `tx_size`, planned
    /// as the decoder plans parsed partition units.
    pub(crate) fn partition_luma(
        &mut self,
        tx_size: usize,
        partition: LumaTxPartition,
    ) -> core::result::Result<(), GeneralIntraResidualError> {
        let luma = self.unit(0)?;
        if luma.plane_id != PlaneId::Y {
            return Err(GeneralIntraResidualError::InvalidReconstructionState {
                context: "luma transform partition plane",
            });
        }
        let records = luma_transform_records_for_partition(luma.x, luma.y, tx_size, partition)?;
        let count = records.len();
        let (frame_width, frame_height) = (
            luma.block_ctx.frame_mi_cols() * 4,
            luma.block_ctx.frame_mi_rows() * 4,
        );
        let mut units = Vec::with_capacity(count);
        for record in records {
            if !luma_transform_record_starts_in_frame(&record, frame_width, frame_height) {
                continue;
            }
            let unit = luma.transform_unit_plan(&PositionedLumaCoeffBlock {
                x: record.x,
                y: record.y,
                tx_size: record.tx_size,
                middle: record.middle,
                coeffs: LumaCoeffBlock::empty(0, false),
            })?;
            let fills = luma_partition_record_fills_block(
                luma.tx_fills_residual_block(),
                count,
                record,
                luma.x,
                luma.y,
            );
            units.push(if fills {
                unit
            } else {
                ResidualPlanePlan {
                    residual_width4: luma.residual_width4,
                    residual_height4: luma.residual_height4,
                    ..unit
                }
            });
        }
        self.planes.splice(0..1, units);
        Ok(())
    }

    /// Marks a reconstructed luma transform block `index` as decoded.
    pub(crate) fn publish_unit(
        &self,
        index: usize,
        block_decoded: &mut TileBlockDecodedState,
    ) -> core::result::Result<(), GeneralIntraResidualError> {
        self.unit(index)?.publish_luma_transform(block_decoded);
        Ok(())
    }

    fn unit(
        &self,
        index: usize,
    ) -> core::result::Result<ResidualPlanePlan, GeneralIntraResidualError> {
        self.planes.get(index).copied().ok_or(
            GeneralIntraResidualError::InvalidReconstructionState {
                context: "planned transform block index",
            },
        )
    }
}

impl ResidualReconstructionPlan {
    pub(super) fn for_luma_transform_row(mut self, is_parent_row: bool) -> Self {
        if is_parent_row {
            return self;
        }
        match &mut self {
            Self::Luma(
                RectLumaPlan::OneSidedLeftMrl {
                    mrl_index,
                    above_mrl_index,
                    is_sb_boundary,
                    ..
                }
                | RectLumaPlan::MiddleMrl {
                    mrl_index,
                    above_mrl_index,
                    is_sb_boundary,
                    ..
                },
            ) => {
                *above_mrl_index = *mrl_index;
                *is_sb_boundary = false;
            }
            Self::Luma(
                RectLumaPlan::OneSidedAboveMrl {
                    mrl_index,
                    above_mrl_index,
                    ..
                }
                | RectLumaPlan::CardinalMrl {
                    mrl_index,
                    above_mrl_index,
                    ..
                },
            ) => *above_mrl_index = *mrl_index,
            _ => {}
        }
        self
    }
}

impl ResidualPlanePlan {
    pub(super) fn lossless_transform_unit_tx_size(
        self,
        work_unit: &DecodeTileWorkUnit,
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
        let Some(tx) = TxShape::from_luma_4x4(width4.max(1), height4.max(1)) else {
            return Err(GeneralIntraResidualError::TransformPartitionGeometry {
                table: "Tx_Width_Log2",
                index: block.tx_size,
            });
        };
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
        let Some(chroma_tx) = TxShape::from_luma_4x4(chroma_ref.width4(), chroma_ref.height4())
        else {
            return Err(GeneralIntraResidualError::TransformPartitionGeometry {
                table: "Lossless_Chroma_Tx",
                index: block.tx_size,
            });
        };
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
