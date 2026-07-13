// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Block-to-plane residual planning.

use splot_core::tables::conversion::{TX_HEIGHT_LOG2, TX_WIDTH_LOG2};
use splot_recon::PlaneId;

use crate::bitstream::tile_payload::SupportedChromaMode;
use crate::prediction::intra::IntraLumaPlan;
use crate::support::capability::missing_capability_message;
use crate::tile::block_context::{BlockCtx, BlockRect, TxShape};

use super::{
    CHROMA_PLANES, CHUNK_64_N4, GeneralIntraResidualPlan, IDTX, RectChromaPlan, RectLumaPlan,
    RecycledVec, ResidualPipelineUnsupported, ResidualPlanePlan, ResidualReconstructionPlan,
};

// AV2 § 9.3 caps a block axis at 64 4x4 units
// (`docs/spec/av2/1.0.0/09-additional-tables/09-02-conversion-tables.md`).
const MAX_RESIDUAL_CHUNKS_PER_AXIS: usize = 64 / CHUNK_64_N4;
pub(super) const MAX_RESIDUAL_PLANES: usize =
    MAX_RESIDUAL_CHUNKS_PER_AXIS * MAX_RESIDUAL_CHUNKS_PER_AXIS * (1 + CHROMA_PLANES.len());

std::thread_local! {
    static RESIDUAL_PLANE_PLANS: std::cell::Cell<Option<Vec<ResidualPlanePlan>>> =
        const { std::cell::Cell::new(None) };
}

fn take_residual_plane_plans() -> RecycledVec<ResidualPlanePlan> {
    RecycledVec::take(&RESIDUAL_PLANE_PLANS, MAX_RESIDUAL_PLANES)
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
        let mut planes = take_residual_plane_plans();
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
        let mut planes = take_residual_plane_plans();
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
                above_mrl_index,
                is_sb_boundary,
                secondary_mrl,
                use_tcq,
            } => ResidualReconstructionPlan::LumaRectOneSidedLeftMrl {
                p_angle,
                mrl_index,
                above_mrl_index,
                is_sb_boundary,
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
        lossless_luma_fsc: bool,
    ) -> core::result::Result<Self, ResidualPipelineUnsupported> {
        let reconstruction = chroma_reconstruction(chroma_plan);
        let mut planes = take_residual_plane_plans();
        let chroma_block = block_ctx.plane_block(PlaneId::U);
        let chroma = chroma_plans(
            block_ctx,
            reconstruction,
            chroma_block.width4(),
            chroma_block.height4(),
            false,
            false,
        )?;
        planes.extend(if lossless_luma_fsc {
            chroma.map(|plane| plane.with_reconstruction_tx_type(IDTX))
        } else {
            chroma
        });
        Ok(Self { planes })
    }

    pub(crate) fn chroma_tx(&self) -> Option<usize> {
        self.planes
            .iter()
            .find(|plane| plane.plane_id == PlaneId::U)
            .map(|plane| plane.tx_size)
    }
}

fn chroma_reconstruction(plan: RectChromaPlan) -> ResidualReconstructionPlan {
    match plan {
        RectChromaPlan::Mode(SupportedChromaMode::Dc, None) => {
            ResidualReconstructionPlan::Rect { use_tcq: false }
        }
        RectChromaPlan::Mode(mode, dpcm) => ResidualReconstructionPlan::Chroma { mode, dpcm },
        RectChromaPlan::OneSided(p_angle, dpcm) => {
            ResidualReconstructionPlan::ChromaOneSided(p_angle, dpcm)
        }
        RectChromaPlan::Middle(p_angle, dpcm) => {
            ResidualReconstructionPlan::ChromaMiddle(p_angle, dpcm)
        }
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
            reconstruction_tx_type: None,
            reconstruction,
        })
    }

    pub(super) const fn with_deferred_reconstruction(self) -> Self {
        Self {
            defer_reconstruction: true,
            ..self
        }
    }

    const fn with_reconstruction_tx_type(self, plane_tx_type: usize) -> Self {
        Self {
            reconstruction_tx_type: Some(plane_tx_type),
            ..self
        }
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
    if width_chunks > MAX_RESIDUAL_CHUNKS_PER_AXIS || height_chunks > MAX_RESIDUAL_CHUNKS_PER_AXIS {
        return Err(UNSUPPORTED_RESIDUAL_PLANE_CAPACITY);
    }
    let (sub_x, sub_y) = block_ctx.chroma().subsampling(PlaneId::U);
    let double_chroma_w = sub_x != 0 && width_chunks > 1 && !lossless;
    let double_chroma_h = sub_y != 0 && height_chunks > 1 && !lossless;
    let defer_chroma_reconstruction = chroma_reconstruction
        .is_some_and(chroma_depends_on_complete_luma)
        && (width_chunks > 1 || height_chunks > 1);
    let chroma_block = block_ctx.plane_block(PlaneId::U);

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
                        let chroma_ctx = residual_chunk_ctx(
                            block_ctx,
                            chunk_x,
                            chunk_y,
                            chunk_width,
                            chunk_height,
                        )?;
                        planes.extend(chroma_plans(
                            chroma_ctx,
                            reconstruction,
                            chroma_block.width4(),
                            chroma_block.height4(),
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
    let (block, tx) = residual_chunk_geometry(block, chunk_x, chunk_y, chunk_width, chunk_height)?;
    let mut chunk_ctx = BlockCtx::new(
        block,
        tx,
        block_ctx.frame_mi_cols(),
        block_ctx.frame_mi_rows(),
        block_ctx.bit_depth(),
        block_ctx.chroma(),
    )
    .with_tile_bounds_from(block_ctx);
    if let Some((chroma_ref, chroma_tx)) = block_ctx.chroma_ref() {
        let (chroma_ref, chroma_tx) = if chroma_ref.width4() == block_ctx.block().width4()
            && chroma_ref.height4() == block_ctx.block().height4()
        {
            residual_chunk_geometry(chroma_ref, chunk_x, chunk_y, chunk_width, chunk_height)?
        } else {
            (chroma_ref, chroma_tx)
        };
        chunk_ctx = chunk_ctx.with_chroma_ref(chroma_ref, chroma_tx);
    }
    Ok(chunk_ctx)
}

fn residual_chunk_geometry(
    block: BlockRect,
    chunk_x: usize,
    chunk_y: usize,
    chunk_width: usize,
    chunk_height: usize,
) -> core::result::Result<(BlockRect, TxShape), ResidualPipelineUnsupported> {
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
    Ok((BlockRect::new(row4, col4, width4, height4), tx))
}

fn chroma_plans(
    block_ctx: BlockCtx,
    reconstruction: ResidualReconstructionPlan,
    residual_width4: usize,
    residual_height4: usize,
    txb_skip_fsc_mode: bool,
    defer_reconstruction: bool,
) -> core::result::Result<[ResidualPlanePlan; 2], ResidualPipelineUnsupported> {
    let [u, v] = CHROMA_PLANES.map(|plane_id| {
        let plan = ResidualPlanePlan::new(
            block_ctx,
            plane_id,
            reconstruction,
            residual_width4,
            residual_height4,
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

pub(super) const fn coeff_plane(plane_id: PlaneId) -> usize {
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

pub(super) fn rect_tx_size_from_log2(w_log2: u32, h_log2: u32) -> Option<usize> {
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

const UNSUPPORTED_RESIDUAL_PLANE_CAPACITY: ResidualPipelineUnsupported = unsupported(
    "general_intra_residual_plane_capacity",
    missing_capability_message!("intra.residual_plane.capacity"),
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
