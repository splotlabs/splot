// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Residual reconstruction kernel dispatch.

use splot_recon::{CurrentFrameWorkspace, PlaneId, ReconSample};

use crate::bitstream::tile_payload::{
    GeneralIntraResidualError, LumaTransformTypeContext, TileBlockDecodedState,
};
use crate::pipeline::reconstruct::IntraEdgeAvailability as EdgeAvail;
use crate::pipeline::reconstruct::MiddleEdgeAvailability as MiddleAvail;
use crate::pipeline::reconstruct::OneSidedAboveMrl as AboveMrl;
use crate::tile::block_context::{BlockCtx, NeighbourAvailability};

use super::{ResidualPlanePlan, ResidualReconstructionPlan};

#[derive(Clone, Copy)]
struct UnitMrlReplan {
    mrl_index: usize,
    above_mrl_index: usize,
    is_sb_boundary: bool,
    secondary_mrl: bool,
}

impl ResidualPlanePlan {
    pub(super) fn unit_directional_replan(
        &self,
        luma_context: LumaTransformTypeContext,
    ) -> ResidualReconstructionPlan {
        if let ResidualReconstructionPlan::ChromaDirectional {
            mode,
            angle_delta_uv,
            dpcm,
        } = self.reconstruction
        {
            let Some(base) = mode.directional_base_angle() else {
                return ResidualReconstructionPlan::Chroma { mode, dpcm };
            };
            let nominal = base + i32::from(angle_delta_uv) * 3;
            let unit_w = 1usize << self.tx.width_log2();
            let unit_h = 1usize << self.tx.height_log2();
            let mapped =
                crate::pipeline::general_intra::wide_angle_mapped_p_angle(unit_w, unit_h, nominal);
            let Ok(p_angle) = u16::try_from(mapped) else {
                return ResidualReconstructionPlan::Chroma { mode, dpcm };
            };
            return match p_angle {
                1..=89 | 181..=270 => ResidualReconstructionPlan::ChromaOneSided(p_angle, dpcm),
                91..=179 => ResidualReconstructionPlan::ChromaMiddle(p_angle, dpcm),
                _ => ResidualReconstructionPlan::Chroma { mode, dpcm },
            };
        }
        let (use_tcq, mrl) = match self.reconstruction {
            ResidualReconstructionPlan::LumaRectOneSidedAbove { use_tcq, .. }
            | ResidualReconstructionPlan::LumaRectOneSidedLeft { use_tcq, .. }
            | ResidualReconstructionPlan::LumaRectMiddle { use_tcq, .. } => (use_tcq, None),
            ResidualReconstructionPlan::LumaRectOneSidedAboveMrl {
                mrl_index,
                above_mrl_index,
                secondary_mrl,
                use_tcq,
                ..
            } => (
                use_tcq,
                Some(UnitMrlReplan {
                    mrl_index,
                    above_mrl_index,
                    is_sb_boundary: above_mrl_index != mrl_index,
                    secondary_mrl,
                }),
            ),
            ResidualReconstructionPlan::LumaRectOneSidedLeftMrl {
                mrl_index,
                above_mrl_index,
                is_sb_boundary,
                secondary_mrl,
                use_tcq,
                ..
            }
            | ResidualReconstructionPlan::LumaRectMiddleMrl {
                mrl_index,
                above_mrl_index,
                is_sb_boundary,
                secondary_mrl,
                use_tcq,
                ..
            } => (
                use_tcq,
                Some(UnitMrlReplan {
                    mrl_index,
                    above_mrl_index,
                    is_sb_boundary,
                    secondary_mrl,
                }),
            ),
            _ => return self.reconstruction,
        };
        if self.plane_id != PlaneId::Y {
            return self.reconstruction;
        }
        let Some(base) = luma_context.y_mode().mode_to_angle() else {
            return self.reconstruction;
        };
        let Some(mrl_delta) = crate::pipeline::general_intra::MRL_INDEX_TO_DELTA
            .get(usize::from(luma_context.mrl_index()))
        else {
            return self.reconstruction;
        };
        let nominal = i32::from(base) + i32::from(luma_context.angle_delta_y()) * 3 + mrl_delta;
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
        if let Some(mrl) = mrl {
            if p_angle < 90 {
                return ResidualReconstructionPlan::LumaRectOneSidedAboveMrl {
                    p_angle,
                    mrl_index: mrl.mrl_index,
                    above_mrl_index: mrl.above_mrl_index,
                    secondary_mrl: mrl.secondary_mrl,
                    use_tcq,
                };
            }
            if p_angle > 180 {
                return ResidualReconstructionPlan::LumaRectOneSidedLeftMrl {
                    p_angle,
                    mrl_index: mrl.mrl_index,
                    above_mrl_index: mrl.above_mrl_index,
                    is_sb_boundary: mrl.is_sb_boundary,
                    secondary_mrl: mrl.secondary_mrl,
                    use_tcq,
                };
            }
            return ResidualReconstructionPlan::LumaRectMiddleMrl {
                p_angle,
                mrl_index: mrl.mrl_index,
                above_mrl_index: mrl.above_mrl_index,
                is_sb_boundary: mrl.is_sb_boundary,
                secondary_mrl: mrl.secondary_mrl,
                use_tcq,
            };
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

    pub(super) fn plane_neighbours(
        self,
        block_ctx: BlockCtx,
        block_decoded: &TileBlockDecodedState,
    ) -> NeighbourAvailability {
        block_ctx.neighbours_from_block_decoded(self.plane_id, block_decoded)
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn reconstruct<T: ReconSample>(
        self,
        scratch: &mut crate::pipeline::general_intra::GeneralIntraReconScratch<T>,
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
                    None,
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
                        luma_context,
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
                        None,
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
                        neighbours.num_above_right(),
                        edge_filter,
                        crate::pipeline::reconstruct::IbpSecondary {
                            second_angle: p_angle + 180,
                            edge_filter: secondary_filter,
                            num4_far: neighbours.num_below_left(),
                        },
                        EdgeAvail::new(edges.above, edges.left),
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
                        None,
                        availability,
                        block_ctx.bit_depth(),
                        edge_filter,
                    )
                }
            }
            ResidualReconstructionPlan::LumaRectOneSidedLeftMrl {
                p_angle,
                mrl_index,
                above_mrl_index,
                secondary_mrl,
                use_tcq,
                ..
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
                        above_mrl_index,
                        use_tcq,
                        luma_context,
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
                        above_mrl_index,
                        use_tcq,
                        Some(luma_context),
                        None,
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
                let edges = block_ctx.neighbours(PlaneId::Y);
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
                    luma_context,
                    EdgeAvail::new(edges.has_above(), edges.has_left()),
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
                        EdgeAvail::new(edges.above, edges.left),
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
                        0,
                        use_tcq,
                        Some(luma_context),
                        None,
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
                    scratch,
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
            ResidualReconstructionPlan::ChromaOneSided(p_angle, dpcm) => {
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
                        dpcm,
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
                        0,
                        false,
                        None,
                        dpcm,
                        availability,
                        block_ctx.bit_depth(),
                        edge_filter,
                    )
                }
            }
            ResidualReconstructionPlan::ChromaMiddle(p_angle, dpcm) => {
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
                    dpcm,
                    block_ctx.bit_depth(),
                    MiddleAvail { above: edges.above, left: edges.left },
                    edge_filters,
                )
            }
            ResidualReconstructionPlan::Chroma { mode, dpcm } => {
                let neighbours = self.plane_neighbours(block_ctx, block_decoded);
                crate::pipeline::reconstruct::reconstruct_general_intra_chroma_block_into(
                    scratch,
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
            ResidualReconstructionPlan::ChromaDirectional { .. } => {
                Err(GeneralIntraResidualError::UnexpectedBranch)
            }
            ResidualReconstructionPlan::ChromaCfl {
                params,
                cfl_ds_filter_index,
                sb_mib,
            } => {
                let neighbours = self.plane_neighbours(block_ctx, block_decoded);
                crate::pipeline::reconstruct::reconstruct_general_intra_chroma_cfl_block_into(
                    scratch,
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
