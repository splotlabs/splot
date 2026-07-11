// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! One-sided IBP dual-direction blend entry.

use splot_recon::{
    BitDepth, CurrentFrameWorkspace, IntraDirectionalAngle, IntraDirectionalAngleEdges,
    IntraDirectionalAngleIdifEdges, IntraRectBlockSize, PlaneId, ReconSample,
    apply_ibp_dr_blend_rect, predict_intra_directional_angle_rect_into,
    predict_intra_directional_angle_rect_one_sided_idif_into,
};

use super::one_sided::{
    OneSidedEdgeFilter, build_one_sided_above_idif_edge, build_one_sided_left_idif_edge,
};
use super::sink::IntraEdgeAvailability;
use crate::bitstream::tile_payload::{
    GeneralIntraResidualError, LumaCoeffBlock, LumaTransformTypeContext,
    reconstruct_general_intra_coeff_block_rect_with_prediction,
    reconstruct_general_intra_luma_block_rect_with_prediction_and_ist,
};

#[derive(Clone, Copy, Debug)]
pub(crate) struct IbpSecondary {
    pub second_angle: u16,
    pub edge_filter: OneSidedEdgeFilter,
    pub num4_far: usize,
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn reconstruct_general_intra_one_sided_ibp_luma_block_into<T: ReconSample>(
    workspace: &mut CurrentFrameWorkspace<T>,
    block: &LumaCoeffBlock,
    p_angle: u16,
    plane_id: PlaneId,
    x: usize,
    y: usize,
    log2_width: u32,
    log2_height: u32,
    qindex: u32,
    primary_num4_far: usize,
    primary_edge_filter: OneSidedEdgeFilter,
    secondary: IbpSecondary,
    availability: IntraEdgeAvailability,
    use_tcq: bool,
    luma_context: Option<LumaTransformTypeContext>,
    bit_depth: BitDepth,
) -> core::result::Result<(), GeneralIntraResidualError> {
    let width = 1usize << log2_width;
    let height = 1usize << log2_height;
    let log2_w = u8::try_from(log2_width).unwrap_or(u8::MAX);
    let log2_h = u8::try_from(log2_height).unwrap_or(u8::MAX);
    let block_size = IntraRectBlockSize::new(log2_w, log2_h)?;
    let zone1 = p_angle < 90;
    let mut primary = vec![T::default(); width * height];
    if zone1 {
        let above_idif = build_one_sided_above_idif_edge(
            workspace,
            plane_id,
            x,
            y,
            width,
            height,
            primary_num4_far,
            0, // mrl_index: the §7.13.2.7 IBP blend is gated to the immediate edge
            0, // above_mrl_index
            availability,
            bit_depth,
            primary_edge_filter,
        )?;
        let angle = IntraDirectionalAngle::try_from_p_angle(p_angle)?;
        if matches!(plane_id, PlaneId::Y) {
            predict_intra_directional_angle_rect_one_sided_idif_into(
                bit_depth,
                block_size,
                angle,
                IntraDirectionalAngleIdifEdges::above(&above_idif),
                &mut primary,
                width,
            )?;
        } else {
            let above = &above_idif[2..2 + width + height];
            predict_intra_directional_angle_rect_into(
                bit_depth,
                block_size,
                angle,
                IntraDirectionalAngleEdges::above(above),
                &mut primary,
                width,
            )?;
        }
    } else {
        let left_idif = build_one_sided_left_idif_edge(
            workspace,
            plane_id,
            x,
            y,
            width,
            height,
            primary_num4_far,
            availability.above,
            0, // mrl_index: the §7.13.2.7 IBP blend is gated to the immediate edge
            availability.left,
            bit_depth,
            primary_edge_filter,
        )?;
        let angle = IntraDirectionalAngle::try_from_p_angle(p_angle)?;
        if matches!(plane_id, PlaneId::Y) {
            predict_intra_directional_angle_rect_one_sided_idif_into(
                bit_depth,
                block_size,
                angle,
                IntraDirectionalAngleIdifEdges::left(&left_idif),
                &mut primary,
                width,
            )?;
        } else {
            let left = &left_idif[2..2 + width + height];
            predict_intra_directional_angle_rect_into(
                bit_depth,
                block_size,
                angle,
                IntraDirectionalAngleEdges::left(left),
                &mut primary,
                width,
            )?;
        }
    }
    let mut second = vec![T::default(); width * height];
    let second_angle = IntraDirectionalAngle::try_from_p_angle(secondary.second_angle)?;
    if zone1 {
        let left_idif = build_one_sided_left_idif_edge(
            workspace,
            plane_id,
            x,
            y,
            width,
            height,
            secondary.num4_far,
            availability.above,
            0, // mrl_index: the §7.13.2.7 IBP blend is gated to the immediate edge
            availability.left,
            bit_depth,
            secondary.edge_filter,
        )?;
        if matches!(plane_id, PlaneId::Y) {
            predict_intra_directional_angle_rect_one_sided_idif_into(
                bit_depth,
                block_size,
                second_angle,
                IntraDirectionalAngleIdifEdges::left(&left_idif),
                &mut second,
                width,
            )?;
        } else {
            let left = &left_idif[2..2 + width + height];
            predict_intra_directional_angle_rect_into(
                bit_depth,
                block_size,
                second_angle,
                IntraDirectionalAngleEdges::left(left),
                &mut second,
                width,
            )?;
        }
    } else {
        let above_idif = build_one_sided_above_idif_edge(
            workspace,
            plane_id,
            x,
            y,
            width,
            height,
            secondary.num4_far,
            0, // mrl_index: the §7.13.2.7 IBP blend is gated to the immediate edge
            0, // above_mrl_index
            availability,
            bit_depth,
            secondary.edge_filter,
        )?;
        if matches!(plane_id, PlaneId::Y) {
            predict_intra_directional_angle_rect_one_sided_idif_into(
                bit_depth,
                block_size,
                second_angle,
                IntraDirectionalAngleIdifEdges::above(&above_idif),
                &mut second,
                width,
            )?;
        } else {
            let above = &above_idif[2..2 + width + height];
            predict_intra_directional_angle_rect_into(
                bit_depth,
                block_size,
                second_angle,
                IntraDirectionalAngleEdges::above(above),
                &mut second,
                width,
            )?;
        }
    }
    apply_ibp_dr_blend_rect(block_size, p_angle, &mut primary, &second)?;
    let out = if block.all_zero {
        primary
    } else if let Some(luma_context) = luma_context {
        reconstruct_general_intra_luma_block_rect_with_prediction_and_ist(
            block,
            &primary,
            qindex,
            log2_width,
            log2_height,
            use_tcq,
            bit_depth,
            luma_context,
        )?
    } else {
        reconstruct_general_intra_coeff_block_rect_with_prediction(
            block,
            &primary,
            qindex,
            plane_id,
            log2_width,
            log2_height,
            use_tcq,
            None,
            bit_depth,
        )?
    };
    workspace.write_rect_block(plane_id, x, y, block_size, &out)?;
    Ok(())
}
