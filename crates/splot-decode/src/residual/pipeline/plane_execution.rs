// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Shared residual-plane parsing and reconstruction.
//!
//! Feature tracking: `INFRA-DECODE-PARALLEL-STAGES`.

use splot_core::symbol::SymbolDecoder;
use splot_recon::{CurrentFrameWorkspace, PlaneId, ReconSample};

use crate::bitstream::tile_payload::{
    CoeffBlock, DecodeTileWorkUnit, GeneralIntraResidualError, LumaCoeffBlock,
    LumaTransformPartitionContext, LumaTransformPartitionUnits, LumaTransformTypeContext,
    PositionedLumaCoeffBlock, TileBlockDecodedState, TileCoeffContextState,
    TransformToolResidualPolicy, decode_general_intra_luma_partition_coeffs,
    decode_general_intra_plane_coeffs,
};
use crate::pipeline::general_intra::inherited_chroma_angle_delta;

use super::transform_units::tx_size_log2;
use super::{DCT_DCT, DeblockRecorder, GeneralIntraResidualPlan, ResidualPlanePlan, chroma_pair};

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

/// The planes a row's blocks have parsed so far.
#[derive(Default)]
pub(crate) struct ResidualPlaneArena {
    planes: Vec<Option<ParsedResidualPlane>>,
    units: Vec<Option<ParsedTransformUnit>>,
}

impl ResidualPlaneArena {
    pub(crate) const fn new() -> Self {
        Self {
            planes: Vec::new(),
            units: Vec::new(),
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

impl GeneralIntraResidualPlan {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn parse(
        &self,
        work_unit: &mut DecodeTileWorkUnit,
        symbols: &mut SymbolDecoder<'_>,
        coeff_ctx: &mut TileCoeffContextState,
        uv_mode: usize,
        luma_transform_type_context: LumaTransformTypeContext,
        luma_tx_partition_context: Option<LumaTransformPartitionContext>,
        transform_tool_residual_policy: TransformToolResidualPolicy,
        deblock: &mut DeblockRecorder<'_>,
        arena: &mut ResidualPlaneArena,
        coeffs_arena: &mut Vec<i32>,
    ) -> core::result::Result<ParsedGeneralIntraResidual, GeneralIntraResidualError> {
        let mut u_nonzero = false;
        let mut pending_u = false;
        let start = u32::try_from(arena.planes.len()).unwrap_or(u32::MAX);
        for plane in &self.planes {
            let eob_u_nonzero = plane.plane_id == PlaneId::V && u_nonzero;
            let (role, pass_eob_u_nonzero) =
                if chroma_pair::can_hold_for_cctx_pair(plane, work_unit) {
                    (CctxRole::HoldU, false)
                } else if plane.plane_id == PlaneId::V && !plane.defer_reconstruction && pending_u {
                    (CctxRole::PairV, eob_u_nonzero)
                } else {
                    (CctxRole::None, eob_u_nonzero)
                };
            plane.parse(
                work_unit,
                symbols,
                coeff_ctx,
                uv_mode,
                luma_transform_type_context,
                luma_tx_partition_context,
                transform_tool_residual_policy,
                pass_eob_u_nonzero,
                deblock,
                coeffs_arena,
                role,
                arena,
            )?;
            if plane.plane_id == PlaneId::U && role != CctxRole::PairV {
                u_nonzero = arena
                    .planes
                    .last()
                    .and_then(Option::as_ref)
                    .is_some_and(|parsed| parsed.u_nonzero(&arena.units));
            }
            match role {
                CctxRole::HoldU => pending_u = true,
                CctxRole::PairV => pending_u = false,
                CctxRole::None => {}
            }
        }
        let end = u32::try_from(arena.planes.len()).unwrap_or(u32::MAX);
        Ok(ParsedGeneralIntraResidual { planes: start..end })
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
        work_unit: &mut DecodeTileWorkUnit,
        symbols: &mut SymbolDecoder<'_>,
        coeff_ctx: &mut TileCoeffContextState,
        uv_mode: usize,
        luma_transform_type_context: LumaTransformTypeContext,
        luma_tx_partition_context: Option<LumaTransformPartitionContext>,
        transform_tool_residual_policy: TransformToolResidualPolicy,
        eob_u_nonzero: bool,
        deblock: &mut DeblockRecorder<'_>,
        coeffs_arena: &mut Vec<i32>,
        cctx_role: CctxRole,
        arena: &mut ResidualPlaneArena,
    ) -> core::result::Result<(), GeneralIntraResidualError> {
        let units = &mut arena.units;
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
        let palette_color_map = self.read_palette_color_map(work_unit, symbols)?;
        if let Some(unit_tx_size) = self.lossless_transform_unit_tx_size(work_unit) {
            let parsed = self.parse_lossless_transform_units(
                unit_tx_size,
                work_unit,
                symbols,
                coeff_ctx,
                uv_mode,
                angle_delta_uv,
                policy,
                eob_u_nonzero,
                palette_color_map.as_deref(),
                deblock,
                coeffs_arena,
                units,
            );
            super::palette::recycle_palette_map(palette_color_map);
            arena.planes.push(Some(parsed?.with_cctx_role(cctx_role)));
            return Ok(());
        }
        if let Some(tx_partition_context) = tx_partition_context {
            let parsed = self.parse_partitioned_luma(
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
            arena.planes.push(Some(parsed?.with_cctx_role(cctx_role)));
            return Ok(());
        }
        let mut coeffs = crate::bitstream::tile_payload::decode_general_intra_plane_coeffs(
            work_unit,
            symbols,
            coeff_ctx,
            coeffs_arena,
            usize::from(self.coeff_plane),
            self.tx_size,
            self.x,
            self.y,
            self.tx_fills_residual_block(),
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
                coeffs.eob(),
            );
        } else {
            deblock.record_chroma_unit(self.plane_id, self.x, self.y, self.tx_size);
        }
        arena.planes.push(Some(ParsedResidualPlane {
            plane: ResidualPlanePlan {
                defer_reconstruction: self.defer_reconstruction || cctx_role != CctxRole::None,
                ..*self
            },
            kind: ParsedResidualPlaneKind::Single {
                coeffs,
                palette_color_map,
            },
            cctx_role,
        }));
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn parse_lossless_transform_units(
        &self,
        unit_tx_size: usize,
        work_unit: &mut DecodeTileWorkUnit,
        symbols: &mut SymbolDecoder<'_>,
        coeff_ctx: &mut TileCoeffContextState,
        uv_mode: usize,
        angle_delta_uv: i32,
        policy: TransformToolResidualPolicy,
        eob_u_nonzero: bool,
        palette_color_map: Option<&[u8]>,
        deblock: &mut DeblockRecorder<'_>,
        coeffs_arena: &mut Vec<i32>,
        units: &mut Vec<Option<ParsedTransformUnit>>,
    ) -> core::result::Result<ParsedResidualPlane, GeneralIntraResidualError> {
        let (log2_width, log2_height) = tx_size_log2(unit_tx_size)?;
        let unit_width4 = (1usize << log2_width) >> 2;
        let unit_height4 = (1usize << log2_height) >> 2;
        let start = units.len();
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
                    coeffs_arena,
                    usize::from(self.coeff_plane),
                    unit_tx_size,
                    x,
                    y,
                    false,
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
        Ok(ParsedResidualPlane {
            plane: *self,
            kind: ParsedResidualPlaneKind::Lossless(start..units.len()),
            cctx_role: CctxRole::None,
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn parse_partitioned_luma(
        &self,
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
    ) -> core::result::Result<ParsedResidualPlane, GeneralIntraResidualError> {
        let blocks = decode_general_intra_luma_partition_coeffs(
            work_unit,
            symbols,
            coeff_ctx,
            coeffs_arena,
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
        self.retain_partitioned_luma(blocks, palette_color_map, deblock, units)
    }

    pub(super) fn retain_partitioned_luma(
        &self,
        blocks: LumaTransformPartitionUnits<PositionedLumaCoeffBlock>,
        palette_color_map: Option<&[u8]>,
        deblock: &mut DeblockRecorder<'_>,
        units: &mut Vec<Option<ParsedTransformUnit>>,
    ) -> core::result::Result<ParsedResidualPlane, GeneralIntraResidualError> {
        let start = units.len();
        for block in blocks {
            self.transform_unit_plan(&block)?;
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
        Ok(ParsedResidualPlane {
            plane: *self,
            kind: ParsedResidualPlaneKind::PartitionedLuma(start..units.len()),
            cctx_role: CctxRole::None,
        })
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
        let ResidualPlaneArena { planes, units } = arena;
        let range = self.planes.start as usize..self.planes.end as usize;
        let planes = planes.get_mut(range).unwrap_or_default();
        let mut pending_u = None;
        let mut deferred = false;
        for plane in planes.iter_mut().filter_map(Option::as_mut) {
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
        planes.fill_with(|| None);
        Ok(())
    }
}

impl ResidualPlanePlan {
    pub(super) fn publish_luma_transform(&self, block_decoded: &mut TileBlockDecodedState) {
        if self.plane_id == PlaneId::Y {
            block_decoded.set_luma_transform(self.x, self.y, self.tx.width4(), self.tx.height4());
        }
    }
}

impl ParsedResidualPlane {
    fn with_cctx_role(mut self, cctx_role: CctxRole) -> Self {
        if cctx_role != CctxRole::None {
            self.plane.defer_reconstruction = true;
            self.cctx_role = cctx_role;
        }
        self
    }

    pub(super) fn u_nonzero(&self, arena: &[Option<ParsedTransformUnit>]) -> bool {
        match &self.kind {
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
            ParsedResidualPlaneKind::Single {
                coeffs,
                palette_color_map,
            } => {
                plane.reconstruct(
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
                    let plan = plane.transform_unit_plan(&unit.block)?;
                    plan.reconstruct(
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
                    let (sub_x, sub_y) = plane.block_ctx.chroma().subsampling(plan.plane_id);
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
                    let plan = plane.transform_unit_plan(&unit.block)?;
                    plan.reconstruct(
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
    scratch: &mut crate::pipeline::general_intra::GeneralIntraReconScratch<T>,
    workspace: &mut CurrentFrameWorkspace<T>,
    block_decoded: &mut TileBlockDecodedState,
    planes: &mut [Option<ParsedResidualPlane>],
    qindex: u32,
    intra_edge: crate::prediction::intra_edge::IntraEdgeCtx,
    luma_context: LumaTransformTypeContext,
    coeffs_arena: &[i32],
    units: &mut [Option<ParsedTransformUnit>],
) -> core::result::Result<(), GeneralIntraResidualError> {
    let mut pending_u = None;
    for plane in planes
        .iter_mut()
        .filter_map(Option::as_mut)
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
