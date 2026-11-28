// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Per-transform-unit residual replanning.

use splot_core::simd::SimdBackend;
use splot_recon::{CurrentFrameWorkspace, PlaneId, ReconSample};

use crate::bitstream::tile_payload::{
    CoeffBlock, GeneralIntraResidualError, IntraIstSyntax, LumaCoeffBlock,
    LumaTransformTypeContext, LumaTxPartition, PositionedLumaCoeffBlock, TileBlockDecodedState,
    luma_partition_record_fills_block, luma_transform_record_starts_in_frame,
    luma_transform_records_for_partition,
};
use crate::tile::block_context::{BlockRect, FrameTileCtx, TxShape};

use super::{
    GeneralIntraResidualPlan, RectLumaPlan, ResidualPlanePlan, ResidualReconstructionPlan, TX_4X4,
};

impl GeneralIntraResidualPlan {
    /// Reconstructs planned transform block `index` from its row-major adjusted
    /// `quant` and `eob`, through the decoder's own kernel dispatch; a palette
    /// luma block predicts from its row-major `palette_map`.
    #[expect(clippy::too_many_arguments)]
    pub(crate) fn reconstruct_unit<T: ReconSample>(
        &self,
        backend: SimdBackend,
        index: usize,
        quant: &[i32],
        eob: usize,
        (plane_tx_type, use_tcq, intra_ist): (usize, bool, Option<IntraIstSyntax>),
        palette_map: Option<&[u8]>,
        scratch: &mut crate::pipeline::general_intra::GeneralIntraReconScratch<T>,
        workspace: &mut CurrentFrameWorkspace<T>,
        block_decoded: &TileBlockDecodedState,
        (qindex, lossless): (u32, bool),
        intra_edge: crate::prediction::intra_edge::IntraEdgeCtx,
        luma_context: LumaTransformTypeContext,
    ) -> core::result::Result<(), GeneralIntraResidualError> {
        let (plane, coeffs) = self.unit_coeffs(
            (index, quant, eob),
            LumaCoeffBlock {
                intra_ist,
                use_tcq,
                ..LumaCoeffBlock::empty(plane_tx_type, lossless)
            },
        )?;
        plane.reconstruct(
            backend,
            self.frame,
            scratch,
            workspace,
            CoeffBlock::new(&coeffs, quant)?,
            block_decoded,
            palette_map,
            qindex,
            intra_edge,
            luma_context,
        )
    }

    /// Reconstructs the planned chroma blocks `u` and `v` of one position from
    /// their row-major adjusted `quant` and `eob`, with cross-chroma transform
    /// `cctx_type`, through the decoder's paired reconstruction.
    #[expect(clippy::too_many_arguments)]
    pub(crate) fn reconstruct_cctx_pair<T: ReconSample>(
        &self,
        backend: SimdBackend,
        [u, v]: [(usize, &[i32], usize); 2],
        (plane_tx_type, cctx_type): (usize, usize),
        scratch: &mut crate::pipeline::general_intra::GeneralIntraReconScratch<T>,
        workspace: &mut CurrentFrameWorkspace<T>,
        block_decoded: &TileBlockDecodedState,
        qindex: u32,
        intra_edge: crate::prediction::intra_edge::IntraEdgeCtx,
        luma_context: LumaTransformTypeContext,
    ) -> core::result::Result<(), GeneralIntraResidualError> {
        let block = |unit, cctx_type| {
            self.unit_coeffs(
                unit,
                LumaCoeffBlock {
                    cctx_type,
                    ..LumaCoeffBlock::empty(plane_tx_type, false)
                },
            )
        };
        let (u_plane, u_coeffs) = block(u, Some(cctx_type as u8))?;
        let (v_plane, v_coeffs) = block(v, None)?;
        super::chroma_pair::reconstruct_chroma_pair_or_planes(
            backend,
            self.frame,
            scratch,
            workspace,
            block_decoded,
            (&u_plane, CoeffBlock::new(&u_coeffs, u.1)?),
            Some((&v_plane, CoeffBlock::new(&v_coeffs, v.1)?)),
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
        let (luma_x, luma_y) = (luma.x as usize, luma.y as usize);
        let records = luma_transform_records_for_partition(luma_x, luma_y, tx_size, partition)?;
        let count = records.len();
        let (frame_width, frame_height) = (
            self.frame.frame_mi_cols() * 4,
            self.frame.frame_mi_rows() * 4,
        );
        let mut units = Vec::with_capacity(count);
        for record in records {
            if !luma_transform_record_starts_in_frame(&record, frame_width, frame_height) {
                continue;
            }
            let unit = luma.transform_unit_plan(
                self.frame,
                &PositionedLumaCoeffBlock::new(
                    record.x,
                    record.y,
                    record.tx_size,
                    record.middle,
                    LumaCoeffBlock::empty(0, false),
                )?,
            )?;
            let fills = luma_partition_record_fills_block(
                luma.tx_fills_residual_block(),
                count,
                record,
                luma_x,
                luma_y,
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

    /// Replaces each plane block of a lossless leaf with the 4x4 units that
    /// start in the frame, in the raster order of the decoder's lossless
    /// parse (§ 5.20.6.1 `TxSize = TX_4X4`), none of which fills the block.
    pub(crate) fn split_lossless_units(
        &mut self,
    ) -> core::result::Result<(), GeneralIntraResidualError> {
        let mut units = Vec::with_capacity(self.planes.len());
        for plane in &self.planes {
            if plane.lossless_transform_unit_tx_size(Some(true)) != Some(TX_4X4) {
                units.push(*plane);
                continue;
            }
            for y4 in 0..plane.tx.height4() {
                for x4 in 0..plane.tx.width4() {
                    let (x, y) = (plane.x as usize + x4 * 4, plane.y as usize + y4 * 4);
                    if plane.lossless_unit_starts_in_frame(self.frame, x, y) {
                        let empty = LumaCoeffBlock::empty(0, false);
                        let block = PositionedLumaCoeffBlock::new(x, y, TX_4X4, false, empty)?;
                        units.push(ResidualPlanePlan {
                            residual_width4: plane.residual_width4,
                            residual_height4: plane.residual_height4,
                            ..plane.transform_unit_plan(self.frame, &block)?
                        });
                    }
                }
            }
        }
        self.planes.clear();
        self.planes.extend(units);
        Ok(())
    }

    /// Marks the reconstructed lossless unit `index` of any plane as decoded,
    /// as the decoder's lossless reconstruction does after each unit.
    pub(crate) fn publish_lossless_unit(
        &self,
        index: usize,
        block_decoded: &mut TileBlockDecodedState,
    ) -> core::result::Result<(), GeneralIntraResidualError> {
        let unit = self.unit(index)?;
        let sb_mask = block_decoded.sb_size4().saturating_sub(1);
        block_decoded.set_block(
            unit.plane_id.index(),
            unit.rect.row4() & sb_mask,
            unit.rect.col4() & sb_mask,
            unit.tx.width4(),
            unit.tx.height4(),
        );
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

    /// Plan `index` and `coeffs` with the coefficient span of its adjusted `quant` and `eob`.
    fn unit_coeffs(
        &self,
        (index, quant, eob): (usize, &[i32], usize),
        mut coeffs: LumaCoeffBlock,
    ) -> core::result::Result<(ResidualPlanePlan, LumaCoeffBlock), GeneralIntraResidualError> {
        let plane = self.unit(index)?;
        let nonzero = quant
            .iter()
            .rposition(|&q| q != 0)
            .map_or(0, |last| last + 1);
        coeffs.eob = eob as u16;
        coeffs.quant_start = 0;
        coeffs.quant_len = nonzero as u16;
        coeffs.zero_tail = (quant.len() - nonzero) as u16;
        plane.apply_reconstruction_tx_type(&mut coeffs);
        Ok((plane, coeffs))
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
        &self,
        segment_lossless: Option<bool>,
    ) -> Option<usize> {
        let tx_size = usize::from(self.tx_size);
        if segment_lossless != Some(true) || (!self.fsc_mode && tx_size == TX_4X4) {
            return None;
        }
        if !self.fsc_mode {
            return Some(TX_4X4);
        }
        let (log2_width, log2_height) = tx_size_log2(tx_size).ok()?;
        let unit_width4 = (1usize << log2_width) >> 2;
        let unit_height4 = (1usize << log2_height) >> 2;
        (unit_width4 < self.tx.width4() || unit_height4 < self.tx.height4()).then_some(tx_size)
    }

    pub(super) fn lossless_unit_starts_in_frame(
        &self,
        frame: FrameTileCtx,
        x: usize,
        y: usize,
    ) -> bool {
        let (sub_x, sub_y) = frame.chroma().subsampling(self.plane_id);
        let max_x = (frame.frame_mi_cols() * 4) >> sub_x;
        let max_y = (frame.frame_mi_rows() * 4) >> sub_y;
        x < max_x && y < max_y
    }

    pub(super) fn transform_unit_plan(
        &self,
        frame: FrameTileCtx,
        block: &PositionedLumaCoeffBlock,
    ) -> core::result::Result<ResidualPlanePlan, GeneralIntraResidualError> {
        let reconstruction = self
            .reconstruction
            .for_luma_transform_row(block.y == self.y);
        let tx = transform_shape(usize::from(block.tx_size))?;
        let (width4, height4) = (tx.width4(), tx.height4());
        let (sub_x, sub_y) = frame.chroma().subsampling(self.plane_id);
        let rect = BlockRect::new(
            (block.y as usize >> 2) << sub_y,
            (block.x as usize >> 2) << sub_x,
            width4 << sub_x,
            height4 << sub_y,
        );
        Ok(ResidualPlanePlan {
            rect,
            tx_size: block.tx_size,
            x: block.x,
            y: block.y,
            tx,
            residual_width4: width4 as u16,
            residual_height4: height4 as u16,
            zero_corners: block.middle,
            reconstruction,
            ..*self
        })
    }

    pub(super) const fn tx_fills_residual_block(&self) -> bool {
        self.tx.width4() == self.residual_width4 as usize
            && self.tx.height4() == self.residual_height4 as usize
    }
}

pub(super) fn tx_size_log2(
    tx_size: usize,
) -> core::result::Result<(u32, u32), GeneralIntraResidualError> {
    let shape = transform_shape(tx_size)?;
    Ok((shape.width_log2(), shape.height_log2()))
}

fn transform_shape(tx_size: usize) -> core::result::Result<TxShape, GeneralIntraResidualError> {
    TxShape::from_tx_size(tx_size).ok_or(GeneralIntraResidualError::TransformPartitionGeometry {
        table: "Tx_Width_Log2",
        index: tx_size,
    })
}
