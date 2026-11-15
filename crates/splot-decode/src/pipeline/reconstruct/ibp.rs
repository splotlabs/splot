// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! One-sided IBP dual-direction blend entry.

use splot_recon::{
    BitDepth, CurrentFrameWorkspace, IntraDirectionalAngle, IntraDirectionalAngleIdifEdges,
    IntraPredictionScratchBuffer, IntraRectBlockSize, PlaneId, ReconSample,
    apply_ibp_dr_blend_rect, predict_intra_directional_angle_rect_one_sided_idif_into,
};

use super::one_sided::{
    OneSidedEdgeFilter, build_one_sided_above_idif_edge, build_one_sided_left_idif_edge,
};
use super::sink::IntraEdgeAvailability;
use super::sink::write_intra_prediction_block;
use crate::bitstream::tile_payload::{
    CoeffBlock, GeneralIntraResidualError, LumaTransformTypeContext,
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
    block: CoeffBlock<'_>,
    p_angle: u16,
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
    let log2_w = u8::try_from(log2_width).unwrap_or(u8::MAX);
    let log2_h = u8::try_from(log2_height).unwrap_or(u8::MAX);
    let block_size = IntraRectBlockSize::new(log2_w, log2_h)?;
    let (width, height) = (block_size.width(), block_size.height());
    let zone1 = p_angle < 90;
    let primary_edge = (primary_num4_far, primary_edge_filter);
    let second_edge = (secondary.num4_far, secondary.edge_filter);
    let ((above_far, above_filter), (left_far, left_filter)) = if zone1 {
        (primary_edge, second_edge)
    } else {
        (second_edge, primary_edge)
    };
    let above_idif = build_one_sided_above_idif_edge(
        workspace,
        PlaneId::Y,
        x,
        y,
        width,
        height,
        above_far,
        0, // mrl_index: the §7.13.2.7 IBP blend is gated to the immediate edge
        0,
        availability,
        bit_depth,
        above_filter,
    )?;
    let left_idif = build_one_sided_left_idif_edge(
        workspace,
        PlaneId::Y,
        x,
        y,
        width,
        height,
        left_far,
        availability.above,
        0,
        0,
        availability.left,
        bit_depth,
        left_filter,
    )?;
    let above = IntraDirectionalAngleIdifEdges::above(&above_idif);
    let left = IntraDirectionalAngleIdifEdges::left(&left_idif);
    let (primary_edges, second_edges) = if zone1 { (above, left) } else { (left, above) };
    let mut primary = workspace.take_intra_prediction_buffer(
        IntraPredictionScratchBuffer::Primary,
        PlaneId::Y,
        width * height,
    )?;
    predict_intra_directional_angle_rect_one_sided_idif_into(
        bit_depth,
        block_size,
        IntraDirectionalAngle::try_from_p_angle(p_angle)?,
        primary_edges,
        &mut primary,
        width,
    )?;
    let mut second = workspace.take_intra_prediction_buffer(
        IntraPredictionScratchBuffer::Secondary,
        PlaneId::Y,
        width * height,
    )?;
    predict_intra_directional_angle_rect_one_sided_idif_into(
        bit_depth,
        block_size,
        IntraDirectionalAngle::try_from_p_angle(secondary.second_angle)?,
        second_edges,
        &mut second,
        width,
    )?;
    apply_ibp_dr_blend_rect(block_size, p_angle, &mut primary, &second)?;
    workspace.recycle_intra_prediction_buffer(IntraPredictionScratchBuffer::Secondary, second);
    write_intra_prediction_block(
        workspace,
        block,
        primary,
        PlaneId::Y,
        x,
        y,
        block_size,
        qindex,
        use_tcq,
        luma_context,
        None,
        bit_depth,
    )
}
