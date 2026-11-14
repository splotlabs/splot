// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Shared residual-plane parsing and reconstruction.
//!
//! Feature tracking: `INFRA-DECODE-PARALLEL-STAGES`.

use splot_core::symbol::SymbolDecoder;
use splot_recon::{BitDepth, CurrentFrameWorkspace, PlaneId, ReconSample};

use crate::bitstream::tile_payload::{
    CoeffBlock, DecodeTileWorkUnit, GeneralIntraResidualError, LumaCoeffBlock,
    LumaTransformPartitionContext, LumaTransformPartitionUnits, LumaTransformTypeContext,
    PlaneCoeffBlock, PositionedLumaCoeffBlock, TileBlockDecodedState, TileCoeffContextState,
    TransformToolResidualPolicy, current_segment_lossless,
    decode_general_intra_luma_partition_coeffs, decode_general_intra_plane_coeffs,
};
use crate::pipeline::general_intra::inherited_chroma_angle_delta;

use super::transform_units::tx_size_log2;
use super::{
    DCT_DCT, DeblockRecorder, RectChromaPlan, RectLumaPlan, ResidualPlanError, ResidualPlanePlan,
    chroma_pair,
};
use crate::tile::block_context::{BlockCtx, ChromaSampling, FrameTileCtx};

/// One block's planes, as a range of the row's arena.
///
/// A list per block cost an allocation for every general-intra block until the
/// pools saturated, because the pipeline holds a whole frame's blocks at once.
/// The row already outlives them all -- the commit spine replays it later --
/// so its blocks share one arena and keep a range into it, the way they share
/// one coefficient buffer.
pub(crate) struct ParsedGeneralIntraResidual {
    planes: core::ops::Range<u32>,
}

/// The planes a row's blocks have parsed so far, and the frame and tile
/// facts of the row's tile.
pub(crate) struct ResidualPlaneArena {
    planes: Vec<ParsedResidualPlane>,
    units: Vec<Option<ParsedTransformUnit>>,
    frame: FrameTileCtx,
}

impl Default for ResidualPlaneArena {
    fn default() -> Self {
        Self::new()
    }
}

impl ResidualPlaneArena {
    pub(crate) const fn new() -> Self {
        Self {
            planes: Vec::new(),
            units: Vec::new(),
            frame: FrameTileCtx::new(0, 0, BitDepth::Eight, ChromaSampling::Yuv420),
        }
    }

    pub(crate) fn clear(&mut self) {
        self.planes.clear();
        self.units.clear();
    }

    /// The plane and transform-unit counts parsed into the arena.
    pub(crate) fn lengths(&self) -> [usize; 2] {
        [self.planes.len(), self.units.len()]
    }

    /// Sizes the emptied lists to the decode's typical unit; see
    /// [`fit_list`](crate::support::decode_buffers::fit_list).
    pub(crate) fn fit(&mut self, [planes, units]: [usize; 2]) {
        crate::support::decode_buffers::fit_list(&mut self.planes, planes);
        crate::support::decode_buffers::fit_list(&mut self.units, units);
    }
}

pub(crate) struct ParsedResidualPlane {
    pub(super) plane: ResidualPlanePlan,
    pub(super) kind: ParsedResidualPlaneKind,
    pub(super) cctx_role: CctxRole,
}

pub(super) enum ParsedResidualPlaneKind {
    /// Planned, its coefficients not read yet.
    Unparsed,
    Single {
        coeffs: LumaCoeffBlock,
        palette_color_map: Option<Vec<u8>>,
    },
    Lossless(core::ops::Range<usize>),
    PartitionedLuma(core::ops::Range<usize>),
}

pub(super) struct ParsedTransformUnit {
    pub(super) block: PositionedLumaCoeffBlock,
    pub(super) palette_color_map: Option<Vec<u8>>,
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub(super) enum CctxRole {
    None,
    HoldU,
    PairV,
}

/// A block's planned planes, a range of the row's arena.
pub(crate) struct PlannedGeneralIntraResidual(core::ops::Range<u32>);

impl ResidualPlaneArena {
    fn plan_start(&self) -> Result<u32, ResidualPlanError> {
        u32::try_from(self.planes.len()).map_err(|_| ResidualPlanError::InvalidGeometry)
    }

    fn push_unparsed(&mut self, plane: ResidualPlanePlan) {
        self.planes.push(ParsedResidualPlane {
            plane,
            kind: ParsedResidualPlaneKind::Unparsed,
            cctx_role: CctxRole::None,
        });
    }

    fn planned_since(&self, start: u32) -> Result<PlannedGeneralIntraResidual, ResidualPlanError> {
        Ok(PlannedGeneralIntraResidual(start..self.plan_start()?))
    }

    /// Plans [`super::GeneralIntraResidualPlan::rect`] into the arena.
    pub(crate) fn plan_rect(
        &mut self,
        block_ctx: &BlockCtx,
        luma_plan: RectLumaPlan,
        chroma_plan: Option<RectChromaPlan>,
        luma_fsc_mode: bool,
        luma_lossless_tx_size: Option<usize>,
        lossless: bool,
    ) -> Result<PlannedGeneralIntraResidual, ResidualPlanError> {
        let start = self.plan_start()?;
        self.frame = block_ctx.frame();
        super::plan::plan_rect(
            &mut |plane| self.push_unparsed(plane),
            *block_ctx,
            luma_plan,
            chroma_plan,
            luma_fsc_mode,
            luma_lossless_tx_size,
            lossless,
        )?;
        self.planned_since(start)
    }

    /// Plans [`super::GeneralIntraResidualPlan::chroma`] into the arena.
    pub(crate) fn plan_chroma(
        &mut self,
        block_ctx: &BlockCtx,
        chroma_plan: RectChromaPlan,
        lossless_luma_fsc: bool,
    ) -> Result<PlannedGeneralIntraResidual, ResidualPlanError> {
        let start = self.plan_start()?;
        self.frame = block_ctx.frame();
        super::plan::plan_chroma(
            &mut |plane| self.push_unparsed(plane),
            *block_ctx,
            chroma_plan,
            lossless_luma_fsc,
        )?;
        self.planned_since(start)
    }

    /// Reads the coefficients of the planned planes in syntax order.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn parse(
        &mut self,
        planned: PlannedGeneralIntraResidual,
        work_unit: &mut DecodeTileWorkUnit,
        symbols: &mut SymbolDecoder<'_>,
        coeff_ctx: &mut TileCoeffContextState,
        uv_mode: usize,
        luma_transform_type_context: LumaTransformTypeContext,
        luma_tx_partition_context: Option<LumaTransformPartitionContext>,
        transform_tool_residual_policy: TransformToolResidualPolicy,
        deblock: &mut DeblockRecorder<'_>,
        coeffs_arena: &mut Vec<i32>,
    ) -> core::result::Result<ParsedGeneralIntraResidual, GeneralIntraResidualError> {
        let mut u_nonzero = false;
        let mut pending_u = false;
        let segment_lossless = current_segment_lossless(work_unit);
        let Self {
            planes,
            units,
            frame,
        } = self;
        let frame = *frame;
        let range = planned.0.start as usize..planned.0.end as usize;
        for parsed in planes.get_mut(range).unwrap_or_default() {
            let ParsedResidualPlane {
                plane,
                kind,
                cctx_role,
            } = parsed;
            let eob_u_nonzero = plane.plane_id == PlaneId::V && u_nonzero;
            let (role, pass_eob_u_nonzero) =
                if chroma_pair::can_hold_for_cctx_pair(plane, segment_lossless) {
                    (CctxRole::HoldU, false)
                } else if plane.plane_id == PlaneId::V && !plane.defer_reconstruction && pending_u {
                    (CctxRole::PairV, eob_u_nonzero)
                } else {
                    (CctxRole::None, eob_u_nonzero)
                };
            *kind = plane.parse(
                frame,
                work_unit,
                symbols,
                coeff_ctx,
                uv_mode,
                luma_transform_type_context,
                luma_tx_partition_context,
                transform_tool_residual_policy,
                segment_lossless,
                pass_eob_u_nonzero,
                deblock,
                coeffs_arena,
                units,
            )?;
            *cctx_role = role;
            plane.defer_reconstruction |= role != CctxRole::None;
            if plane.plane_id == PlaneId::U && role != CctxRole::PairV {
                u_nonzero = parsed.u_nonzero(units);
            }
            match role {
                CctxRole::HoldU => pending_u = true,
                CctxRole::PairV => pending_u = false,
                CctxRole::None => {}
            }
        }
        Ok(ParsedGeneralIntraResidual { planes: planned.0 })
    }
}

impl ResidualPlanePlan {
    pub(super) fn apply_reconstruction_tx_type(&self, coeffs: &mut LumaCoeffBlock) {
        if let Some(tx_type) = self.reconstruction_tx_type {
            coeffs.plane_tx_type = tx_type;
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn parse(
        &self,
        frame: FrameTileCtx,
        work_unit: &mut DecodeTileWorkUnit,
        symbols: &mut SymbolDecoder<'_>,
        coeff_ctx: &mut TileCoeffContextState,
        uv_mode: usize,
        luma_transform_type_context: LumaTransformTypeContext,
        luma_tx_partition_context: Option<LumaTransformPartitionContext>,
        transform_tool_residual_policy: TransformToolResidualPolicy,
        segment_lossless: Option<bool>,
        eob_u_nonzero: bool,
        deblock: &mut DeblockRecorder<'_>,
        coeffs_arena: &mut Vec<i32>,
        units: &mut Vec<Option<ParsedTransformUnit>>,
    ) -> core::result::Result<ParsedResidualPlaneKind, GeneralIntraResidualError> {
        let tx_partition_context = (self.plane_id == PlaneId::Y)
            .then_some(luma_tx_partition_context)
            .flatten();
        let policy = transform_tool_policy_for_plane(
            transform_tool_residual_policy,
            self.plane_id,
            luma_transform_type_context,
        );
        let angle_delta_uv =
            chroma_angle_delta_uv(self.plane_id, uv_mode, luma_transform_type_context);
        let palette_color_map = self.read_palette_color_map(frame, work_unit, symbols)?;
        let coeff_block = PlaneCoeffBlock {
            plane: usize::from(self.coeff_plane),
            tx_size: usize::from(self.tx_size),
            x: self.x as usize,
            y: self.y as usize,
            tx_fills_block: self.tx_fills_residual_block(),
            eob_u_nonzero,
            uv_mode,
            angle_delta_uv,
            chroma_inter_tx_type: DCT_DCT,
            is_inter: false,
            fsc_mode: self.fsc_mode,
            txb_skip_fsc_mode: self.txb_skip_fsc_mode,
            cctx_allowed: chroma_pair::cctx_allowed(self, frame),
            policy,
            segment_lossless,
        };
        if let Some(unit_tx_size) = self.lossless_transform_unit_tx_size(segment_lossless) {
            let parsed = self.parse_lossless_transform_units(
                frame,
                unit_tx_size,
                work_unit,
                symbols,
                coeff_ctx,
                &coeff_block,
                palette_color_map.as_deref(),
                deblock,
                coeffs_arena,
                units,
            );
            super::palette::recycle_palette_map(palette_color_map);
            return parsed;
        }
        if let Some(tx_partition_context) = tx_partition_context {
            let parsed = self.parse_partitioned_luma(
                frame,
                work_unit,
                symbols,
                coeff_ctx,
                tx_partition_context,
                uv_mode,
                angle_delta_uv,
                policy,
                palette_color_map.as_deref(),
                deblock,
                coeffs_arena,
                units,
            );
            super::palette::recycle_palette_map(palette_color_map);
            return parsed;
        }
        let mut coeffs = decode_general_intra_plane_coeffs(
            work_unit,
            symbols,
            coeff_ctx,
            coeffs_arena,
            &coeff_block,
        )?;
        self.apply_reconstruction_tx_type(&mut coeffs);
        if self.plane_id == PlaneId::Y {
            deblock.record_luma_unit(
                self.y as usize / 4,
                self.x as usize / 4,
                self.tx.width4(),
                self.tx.height4(),
                usize::from(self.tx_size),
                coeffs.eob(),
            );
        } else {
            deblock.record_chroma_unit(
                self.plane_id,
                self.x as usize,
                self.y as usize,
                usize::from(self.tx_size),
            );
        }
        Ok(ParsedResidualPlaneKind::Single {
            coeffs,
            palette_color_map,
        })
    }

    #[allow(clippy::too_many_arguments)]
    #[inline(never)]
    fn parse_lossless_transform_units(
        &self,
        frame: FrameTileCtx,
        unit_tx_size: usize,
        work_unit: &mut DecodeTileWorkUnit,
        symbols: &mut SymbolDecoder<'_>,
        coeff_ctx: &mut TileCoeffContextState,
        coeff_block: &PlaneCoeffBlock,
        palette_color_map: Option<&[u8]>,
        deblock: &mut DeblockRecorder<'_>,
        coeffs_arena: &mut Vec<i32>,
        units: &mut Vec<Option<ParsedTransformUnit>>,
    ) -> core::result::Result<ParsedResidualPlaneKind, GeneralIntraResidualError> {
        let (log2_width, log2_height) = tx_size_log2(unit_tx_size)?;
        let unit_width4 = (1usize << log2_width) >> 2;
        let unit_height4 = (1usize << log2_height) >> 2;
        let start = units.len();
        for y4 in (0..self.tx.height4()).step_by(unit_height4) {
            for x4 in (0..self.tx.width4()).step_by(unit_width4) {
                let x = self.x as usize + x4 * 4;
                let y = self.y as usize + y4 * 4;
                if !self.lossless_unit_starts_in_frame(frame, x, y) {
                    continue;
                }
                let mut coeffs = decode_general_intra_plane_coeffs(
                    work_unit,
                    symbols,
                    coeff_ctx,
                    coeffs_arena,
                    &PlaneCoeffBlock {
                        tx_size: unit_tx_size,
                        x,
                        y,
                        tx_fills_block: false,
                        ..*coeff_block
                    },
                )?;
                self.apply_reconstruction_tx_type(&mut coeffs);
                let block = PositionedLumaCoeffBlock {
                    x,
                    y,
                    tx_size: unit_tx_size,
                    middle: false,
                    coeffs,
                };
                let unit = self.transform_unit_plan(frame, &block)?;
                let unit_palette_color_map =
                    self.palette_color_map_for_unit(palette_color_map, &block)?;
                if unit.plane_id == PlaneId::Y {
                    let row4 = block.y / 4;
                    let col4 = block.x / 4;
                    deblock.record_luma_unit(
                        row4,
                        col4,
                        unit_width4,
                        unit_height4,
                        block.tx_size,
                        block.coeffs.eob(),
                    );
                } else {
                    deblock.record_chroma_unit(unit.plane_id, block.x, block.y, block.tx_size);
                }
                units.push(Some(ParsedTransformUnit {
                    block,
                    palette_color_map: unit_palette_color_map,
                }));
            }
        }
        Ok(ParsedResidualPlaneKind::Lossless(start..units.len()))
    }

    #[allow(clippy::too_many_arguments)]
    #[inline(never)]
    fn parse_partitioned_luma(
        &self,
        frame: FrameTileCtx,
        work_unit: &mut DecodeTileWorkUnit,
        symbols: &mut SymbolDecoder<'_>,
        coeff_ctx: &mut TileCoeffContextState,
        tx_partition_context: LumaTransformPartitionContext,
        uv_mode: usize,
        angle_delta_uv: i32,
        policy: TransformToolResidualPolicy,
        palette_color_map: Option<&[u8]>,
        deblock: &mut DeblockRecorder<'_>,
        coeffs_arena: &mut Vec<i32>,
        units: &mut Vec<Option<ParsedTransformUnit>>,
    ) -> core::result::Result<ParsedResidualPlaneKind, GeneralIntraResidualError> {
        let blocks = decode_general_intra_luma_partition_coeffs(
            work_unit,
            symbols,
            coeff_ctx,
            coeffs_arena,
            self.x as usize,
            self.y as usize,
            frame.frame_mi_cols().saturating_mul(4),
            frame.frame_mi_rows().saturating_mul(4),
            self.tx_fills_residual_block(),
            tx_partition_context,
            uv_mode,
            angle_delta_uv,
            self.fsc_mode,
            policy,
        )?;
        self.retain_partitioned_luma(frame, blocks, palette_color_map, deblock, units)
    }

    pub(super) fn retain_partitioned_luma(
        &self,
        frame: FrameTileCtx,
        blocks: LumaTransformPartitionUnits<PositionedLumaCoeffBlock>,
        palette_color_map: Option<&[u8]>,
        deblock: &mut DeblockRecorder<'_>,
        units: &mut Vec<Option<ParsedTransformUnit>>,
    ) -> core::result::Result<ParsedResidualPlaneKind, GeneralIntraResidualError> {
        let start = units.len();
        for block in blocks {
            self.transform_unit_plan(frame, &block)?;
            let unit_palette_color_map =
                self.palette_color_map_for_unit(palette_color_map, &block)?;
            let (log2_width, log2_height) = tx_size_log2(block.tx_size)?;
            let width4 = ((1usize << log2_width) >> 2).max(1);
            let height4 = ((1usize << log2_height) >> 2).max(1);
            deblock.record_luma_unit(
                block.y / 4,
                block.x / 4,
                width4,
                height4,
                block.tx_size,
                block.coeffs.eob(),
            );
            units.push(Some(ParsedTransformUnit {
                block,
                palette_color_map: unit_palette_color_map,
            }));
        }
        Ok(ParsedResidualPlaneKind::PartitionedLuma(start..units.len()))
    }
}

impl ParsedGeneralIntraResidual {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn reconstruct<T: ReconSample>(
        &self,
        arena: &mut ResidualPlaneArena,
        coeffs_arena: &[i32],
        scratch: &mut crate::pipeline::general_intra::GeneralIntraReconScratch<T>,
        workspace: &mut CurrentFrameWorkspace<T>,
        block_decoded: &mut TileBlockDecodedState,
        qindex: u32,
        intra_edge: crate::prediction::intra_edge::IntraEdgeCtx,
        luma_context: LumaTransformTypeContext,
    ) -> core::result::Result<(), GeneralIntraResidualError> {
        let ResidualPlaneArena {
            planes,
            units,
            frame,
        } = arena;
        let frame = *frame;
        let range = self.planes.start as usize..self.planes.end as usize;
        let planes = planes.get_mut(range).unwrap_or_default();
        let mut pending_u = None;
        let mut deferred = false;
        for plane in planes.iter_mut() {
            match plane.cctx_role {
                CctxRole::HoldU => {
                    pending_u = Some(&*plane);
                    continue;
                }
                CctxRole::PairV => {
                    let Some(u) = pending_u.take() else {
                        return Err(GeneralIntraResidualError::InvalidReconstructionState {
                            context: "CCTX pending U plane",
                        });
                    };
                    reconstruct_chroma_pair(
                        frame,
                        scratch,
                        workspace,
                        block_decoded,
                        u,
                        Some(plane),
                        coeffs_arena,
                        qindex,
                        intra_edge,
                        luma_context,
                    )?;
                    continue;
                }
                CctxRole::None => {}
            }
            if plane.plane.defer_reconstruction {
                deferred = true;
            } else {
                plane.reconstruct(
                    frame,
                    scratch,
                    workspace,
                    block_decoded,
                    coeffs_arena,
                    qindex,
                    intra_edge,
                    luma_context,
                    units,
                )?;
            }
        }
        if let Some(u) = pending_u {
            reconstruct_chroma_pair(
                frame,
                scratch,
                workspace,
                block_decoded,
                u,
                None,
                coeffs_arena,
                qindex,
                intra_edge,
                luma_context,
            )?;
        }
        if deferred {
            reconstruct_deferred_planes(
                frame,
                scratch,
                workspace,
                block_decoded,
                planes,
                qindex,
                intra_edge,
                luma_context,
                coeffs_arena,
                units,
            )?;
        }
        Ok(())
    }
}

impl ResidualPlanePlan {
    pub(super) fn publish_luma_transform(&self, block_decoded: &mut TileBlockDecodedState) {
        if self.plane_id == PlaneId::Y {
            block_decoded.set_luma_transform(
                self.x as usize,
                self.y as usize,
                self.tx.width4(),
                self.tx.height4(),
            );
        }
    }
}

impl ParsedResidualPlane {
    pub(super) fn u_nonzero(&self, arena: &[Option<ParsedTransformUnit>]) -> bool {
        match &self.kind {
            ParsedResidualPlaneKind::Unparsed => false,
            ParsedResidualPlaneKind::Single { coeffs, .. } => coeffs.eob != 0,
            ParsedResidualPlaneKind::Lossless(units) => arena
                .get(units.clone())
                .and_then(|units| units.last())
                .and_then(Option::as_ref)
                .is_some_and(|unit| unit.block.coeffs.eob != 0),
            ParsedResidualPlaneKind::PartitionedLuma(units) => arena
                .get(units.clone())
                .unwrap_or_default()
                .iter()
                .flatten()
                .any(|unit| unit.block.coeffs.eob != 0),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn reconstruct<T: ReconSample>(
        &mut self,
        frame: FrameTileCtx,
        scratch: &mut crate::pipeline::general_intra::GeneralIntraReconScratch<T>,
        workspace: &mut CurrentFrameWorkspace<T>,
        block_decoded: &mut TileBlockDecodedState,
        coeffs_arena: &[i32],
        qindex: u32,
        intra_edge: crate::prediction::intra_edge::IntraEdgeCtx,
        luma_context: LumaTransformTypeContext,
        arena: &mut [Option<ParsedTransformUnit>],
    ) -> core::result::Result<(), GeneralIntraResidualError> {
        let Self { plane, kind, .. } = self;
        match kind {
            ParsedResidualPlaneKind::Unparsed => {
                Err(GeneralIntraResidualError::InvalidReconstructionState {
                    context: "unparsed residual plane",
                })
            }
            ParsedResidualPlaneKind::Single {
                coeffs,
                palette_color_map,
            } => {
                plane.reconstruct(
                    frame,
                    scratch,
                    workspace,
                    CoeffBlock::new(coeffs, coeffs_arena)?,
                    block_decoded,
                    palette_color_map.as_deref(),
                    qindex,
                    intra_edge,
                    luma_context,
                )?;
                super::palette::recycle_palette_map(palette_color_map.take());
                plane.publish_luma_transform(block_decoded);
                Ok(())
            }
            ParsedResidualPlaneKind::Lossless(units) => {
                let Some(units) = arena.get_mut(units.clone()) else {
                    return Err(GeneralIntraResidualError::InvalidReconstructionState {
                        context: "residual transform unit range",
                    });
                };
                for unit in units.iter_mut().filter_map(Option::take) {
                    let plan = plane.transform_unit_plan(frame, &unit.block)?;
                    plan.reconstruct(
                        frame,
                        scratch,
                        workspace,
                        CoeffBlock::new(&unit.block.coeffs, coeffs_arena)?,
                        block_decoded,
                        unit.palette_color_map.as_deref(),
                        qindex,
                        intra_edge,
                        luma_context,
                    )?;
                    super::palette::recycle_palette_map(unit.palette_color_map);
                    let (log2_width, log2_height) = tx_size_log2(unit.block.tx_size)?;
                    let width4 = (1usize << log2_width) >> 2;
                    let height4 = (1usize << log2_height) >> 2;
                    let (sub_x, sub_y) = frame.chroma().subsampling(plan.plane_id);
                    let sb_mask = block_decoded.sb_size4().saturating_sub(1);
                    let row4 = ((unit.block.y >> 2) << sub_y) & sb_mask;
                    let col4 = ((unit.block.x >> 2) << sub_x) & sb_mask;
                    block_decoded.set_block(plan.plane_id.index(), row4, col4, width4, height4);
                }
                Ok(())
            }
            ParsedResidualPlaneKind::PartitionedLuma(units) => {
                let Some(units) = arena.get_mut(units.clone()) else {
                    return Err(GeneralIntraResidualError::InvalidReconstructionState {
                        context: "residual transform unit range",
                    });
                };
                for unit in units.iter_mut().filter_map(Option::take) {
                    let plan = plane.transform_unit_plan(frame, &unit.block)?;
                    plan.reconstruct(
                        frame,
                        scratch,
                        workspace,
                        CoeffBlock::new(&unit.block.coeffs, coeffs_arena)?,
                        block_decoded,
                        unit.palette_color_map.as_deref(),
                        qindex,
                        intra_edge,
                        luma_context,
                    )?;
                    super::palette::recycle_palette_map(unit.palette_color_map);
                    plan.publish_luma_transform(block_decoded);
                }
                Ok(())
            }
        }
    }

    fn chroma_pair_parts(
        &self,
    ) -> core::result::Result<(&ResidualPlanePlan, &LumaCoeffBlock), GeneralIntraResidualError>
    {
        match &self.kind {
            ParsedResidualPlaneKind::Single {
                coeffs,
                palette_color_map: None,
            } => Ok((&self.plane, coeffs)),
            _ => Err(GeneralIntraResidualError::InvalidReconstructionState {
                context: "CCTX chroma plane kind",
            }),
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn reconstruct_chroma_pair<T: ReconSample>(
    frame: FrameTileCtx,
    scratch: &mut crate::pipeline::general_intra::GeneralIntraReconScratch<T>,
    workspace: &mut CurrentFrameWorkspace<T>,
    block_decoded: &TileBlockDecodedState,
    u: &ParsedResidualPlane,
    v: Option<&ParsedResidualPlane>,
    coeffs_arena: &[i32],
    qindex: u32,
    intra_edge: crate::prediction::intra_edge::IntraEdgeCtx,
    luma_context: LumaTransformTypeContext,
) -> core::result::Result<(), GeneralIntraResidualError> {
    let (u_plane, u_coeffs) = u.chroma_pair_parts()?;
    let v = match v {
        Some(v) => {
            let (v_plane, v_coeffs) = v.chroma_pair_parts()?;
            Some((v_plane, CoeffBlock::new(v_coeffs, coeffs_arena)?))
        }
        None => None,
    };
    chroma_pair::reconstruct_chroma_pair_or_planes(
        frame,
        scratch,
        workspace,
        block_decoded,
        (u_plane, CoeffBlock::new(u_coeffs, coeffs_arena)?),
        v,
        qindex,
        intra_edge,
        luma_context,
    )
}

#[allow(clippy::too_many_arguments)]
fn reconstruct_deferred_planes<T: ReconSample>(
    frame: FrameTileCtx,
    scratch: &mut crate::pipeline::general_intra::GeneralIntraReconScratch<T>,
    workspace: &mut CurrentFrameWorkspace<T>,
    block_decoded: &mut TileBlockDecodedState,
    planes: &mut [ParsedResidualPlane],
    qindex: u32,
    intra_edge: crate::prediction::intra_edge::IntraEdgeCtx,
    luma_context: LumaTransformTypeContext,
    coeffs_arena: &[i32],
    units: &mut [Option<ParsedTransformUnit>],
) -> core::result::Result<(), GeneralIntraResidualError> {
    let mut pending_u = None;
    for plane in planes
        .iter_mut()
        .filter(|plane| plane.cctx_role == CctxRole::None && plane.plane.defer_reconstruction)
    {
        if plane.plane.plane_id == PlaneId::U {
            pending_u = Some(&*plane);
            continue;
        }
        if plane.plane.plane_id == PlaneId::V
            && let Some(u) = pending_u.take()
        {
            reconstruct_chroma_pair(
                frame,
                scratch,
                workspace,
                block_decoded,
                u,
                Some(plane),
                coeffs_arena,
                qindex,
                intra_edge,
                luma_context,
            )?;
            continue;
        }
        plane.reconstruct(
            frame,
            scratch,
            workspace,
            block_decoded,
            coeffs_arena,
            qindex,
            intra_edge,
            luma_context,
            units,
        )?;
    }
    if let Some(u) = pending_u {
        reconstruct_chroma_pair(
            frame,
            scratch,
            workspace,
            block_decoded,
            u,
            None,
            coeffs_arena,
            qindex,
            intra_edge,
            luma_context,
        )?;
    }
    Ok(())
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
    match plane_id {
        PlaneId::Y => TransformToolResidualPolicy { luma: Some(luma) },
        _ => policy,
    }
}
