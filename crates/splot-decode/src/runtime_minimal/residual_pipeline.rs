// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Runtime residual transform dispatch.

use splot_core::symbol::SymbolDecoder;
use splot_core::tables::conversion::{TX_HEIGHT_LOG2, TX_WIDTH_LOG2};
use splot_recon::{CurrentFrameWorkspace, PlaneId, ReconSample};

use super::block_context::{BlockCtx, TxShape};
use super::capability::missing_capability_message;
use super::intra_prediction::IntraLumaPlan;
use crate::tile_payload::{
    DecodeTileWorkUnit, GeneralIntraResidualError, SupportedChromaMode, TileBlockDecodedState,
    TileCoeffContextState, TransformToolResidualPolicy,
};

const CHROMA_PLANES: [PlaneId; 2] = [PlaneId::U, PlaneId::V];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct GeneralIntraResidualPlan {
    luma: ResidualPlanePlan,
    chroma: Option<[ResidualPlanePlan; 2]>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct ResidualBlockTransforms {
    luma_tx: usize,
    chroma_tx: Option<usize>,
}

impl ResidualBlockTransforms {
    pub(super) const fn luma_tx(self) -> usize {
        self.luma_tx
    }

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
    coeff_plane: usize,
    tx_size: usize,
    x: usize,
    y: usize,
    tx: TxShape,
    reconstruction: ResidualReconstructionPlan,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ResidualReconstructionPlan {
    LumaSquare { plan: IntraLumaPlan, use_tcq: bool },
    ChromaSquare { mode: SupportedChromaMode },
    Rect { use_tcq: bool },
}

impl GeneralIntraResidualPlan {
    pub(super) fn square(
        block_ctx: BlockCtx,
        luma_plan: IntraLumaPlan,
        chroma_mode: Option<SupportedChromaMode>,
        luma_use_tcq: bool,
    ) -> core::result::Result<Self, ResidualPipelineUnsupported> {
        let luma = ResidualPlanePlan::new(
            block_ctx,
            PlaneId::Y,
            ResidualReconstructionPlan::LumaSquare {
                plan: luma_plan,
                use_tcq: luma_use_tcq,
            },
        )?;
        let chroma = chroma_mode
            .map(|mode| chroma_plans(block_ctx, ResidualReconstructionPlan::ChromaSquare { mode }))
            .transpose()?;
        Ok(Self { luma, chroma })
    }

    pub(super) fn rect(
        block_ctx: BlockCtx,
        has_chroma: bool,
        luma_use_tcq: bool,
    ) -> core::result::Result<Self, ResidualPipelineUnsupported> {
        let luma = ResidualPlanePlan::new(
            block_ctx,
            PlaneId::Y,
            ResidualReconstructionPlan::Rect {
                use_tcq: luma_use_tcq,
            },
        )?;
        let chroma = has_chroma
            .then(|| {
                chroma_plans(
                    block_ctx,
                    ResidualReconstructionPlan::Rect { use_tcq: false },
                )
            })
            .transpose()?;
        Ok(Self { luma, chroma })
    }

    pub(super) const fn transforms(self) -> ResidualBlockTransforms {
        ResidualBlockTransforms {
            luma_tx: self.luma.tx_size,
            chroma_tx: match self.chroma {
                Some([u, _]) => Some(u.tx_size),
                None => None,
            },
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn execute<T: ReconSample>(
        self,
        work_unit: &mut DecodeTileWorkUnit<'_>,
        symbols: &mut SymbolDecoder<'_>,
        coeff_ctx: &mut TileCoeffContextState,
        workspace: &mut CurrentFrameWorkspace<T>,
        block_ctx: BlockCtx,
        block_decoded: &TileBlockDecodedState,
        uv_mode: usize,
        qindex: u32,
    ) -> core::result::Result<(), GeneralIntraResidualError> {
        let mut execute = |plane: ResidualPlanePlan, eob_u_nonzero| {
            plane.execute(
                work_unit,
                symbols,
                coeff_ctx,
                workspace,
                block_ctx,
                block_decoded,
                uv_mode,
                qindex,
                eob_u_nonzero,
            )
        };

        execute(self.luma, false)?;
        if let Some([u, v]) = self.chroma {
            let u_all_zero = execute(u, false)?.all_zero;
            execute(v, !u_all_zero)?;
        }
        Ok(())
    }

    #[cfg(test)]
    const fn plane_plan(self, plane_id: PlaneId) -> Option<ResidualPlanePlan> {
        match (plane_id, self.chroma) {
            (PlaneId::Y, _) => Some(self.luma),
            (PlaneId::U, Some([u, _])) => Some(u),
            (PlaneId::V, Some([_, v])) => Some(v),
            (PlaneId::U | PlaneId::V, None) => None,
        }
    }
}

impl ResidualPlanePlan {
    fn new(
        block_ctx: BlockCtx,
        plane_id: PlaneId,
        reconstruction: ResidualReconstructionPlan,
    ) -> core::result::Result<Self, ResidualPipelineUnsupported> {
        let block = block_ctx.plane_block(plane_id);
        let tx = block.tx();
        Ok(Self {
            plane_id,
            coeff_plane: coeff_plane(plane_id),
            tx_size: tx_size_for_plan(tx, plane_id)?,
            x: block.x(),
            y: block.y(),
            tx,
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
        block_ctx: BlockCtx,
        block_decoded: &TileBlockDecodedState,
        uv_mode: usize,
        qindex: u32,
        eob_u_nonzero: bool,
    ) -> core::result::Result<crate::tile_payload::LumaCoeffBlock, GeneralIntraResidualError> {
        let coeffs = crate::tile_payload::decode_general_intra_plane_coeffs(
            work_unit,
            symbols,
            coeff_ctx,
            self.coeff_plane,
            self.tx_size,
            self.x,
            self.y,
            true,
            eob_u_nonzero,
            uv_mode,
            0,
            false,
            false,
            TransformToolResidualPolicy::Allow,
        )?;
        self.reconstruct(workspace, &coeffs, block_ctx, block_decoded, qindex)?;
        Ok(coeffs)
    }

    fn reconstruct<T: ReconSample>(
        self,
        workspace: &mut CurrentFrameWorkspace<T>,
        coeffs: &crate::tile_payload::LumaCoeffBlock,
        block_ctx: BlockCtx,
        block_decoded: &TileBlockDecodedState,
        qindex: u32,
    ) -> core::result::Result<(), GeneralIntraResidualError> {
        match self.reconstruction {
            ResidualReconstructionPlan::LumaSquare { plan, use_tcq } => {
                plan.reconstruct(workspace, coeffs, block_ctx, block_decoded, qindex, use_tcq)
            }
            ResidualReconstructionPlan::ChromaSquare { mode } => {
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
            ResidualReconstructionPlan::Rect { use_tcq } => {
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
                    false,
                    block_ctx.bit_depth(),
                )
            }
        }
    }
}

fn chroma_plans(
    block_ctx: BlockCtx,
    reconstruction: ResidualReconstructionPlan,
) -> core::result::Result<[ResidualPlanePlan; 2], ResidualPipelineUnsupported> {
    let [u, v] =
        CHROMA_PLANES.map(|plane_id| ResidualPlanePlan::new(block_ctx, plane_id, reconstruction));
    Ok([u?, v?])
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
        let plan = GeneralIntraResidualPlan::rect(ctx, false, true).expect("rect luma plan");
        assert!(plan.plane_plan(PlaneId::U).is_none());
        assert!(plan.plane_plan(PlaneId::V).is_none());
        assert_eq!(plan.transforms().chroma_tx(), None);
    }

    fn assert_case(case: Case) {
        let ctx = ctx(case.rect, case.bit_depth);
        let plan = if case.rect.width4() == case.rect.height4() {
            GeneralIntraResidualPlan::square(
                ctx,
                IntraLumaPlan::Dc,
                Some(SupportedChromaMode::Dc),
                true,
            )
        } else {
            GeneralIntraResidualPlan::rect(ctx, case.expect_chroma, true)
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
