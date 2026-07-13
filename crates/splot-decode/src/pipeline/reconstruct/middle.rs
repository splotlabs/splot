// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Middle directional, two-sided, and cardinal neighbour intra entries with middle edge assembly.

use splot_recon::{
    BitDepth, CurrentFrameWorkspace, DpcmDirection, IntraCardinalDirection,
    IntraDirectionalAngleEdges, IntraMiddleDirectionalAngle, IntraMiddleDirectionalAngleEdges,
    IntraMiddleDirectionalAngleIdifEdges, IntraMiddleDirectionalAngleIdifMrlEdges,
    IntraPredictionScratchBuffer, IntraRectBlockSize, PlaneId, ReconSample,
    predict_intra_cardinal_directional_rect_into,
    predict_intra_middle_directional_angle_rect_idif_into,
    predict_intra_middle_directional_angle_rect_idif_mrl_into,
    predict_intra_middle_directional_angle_rect_into,
};

use super::one_sided::{OneSidedEdgeFilter, finalize_one_sided_idif_edge};
use super::sink::{
    IntraEdgeAvailability, average_luma_prediction_with, noneighbour_above, noneighbour_corner,
    noneighbour_left, write_intra_prediction_block,
};
use crate::bitstream::tile_payload::{
    GeneralIntraResidualError, LumaCoeffBlock, LumaTransformTypeContext,
    SupportedDirectionalLumaMode,
};

#[derive(Clone, Copy, Debug)]
pub(crate) struct TwoSidedMiddleEdgeFilters {
    pub above: OneSidedEdgeFilter,
    pub left: OneSidedEdgeFilter,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct MiddleEdgeAvailability {
    pub above: bool,
    pub left: bool,
}

impl MiddleEdgeAvailability {
    pub(crate) const fn new(above: bool, left: bool) -> Self {
        Self { above, left }
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn reconstruct_general_intra_directional_neighbour_block_into<T: ReconSample>(
    workspace: &mut CurrentFrameWorkspace<T>,
    block: &LumaCoeffBlock,
    mode: SupportedDirectionalLumaMode,
    plane_id: PlaneId,
    x: usize,
    y: usize,
    log2_side: u32,
    qindex: u32,
    use_tcq: bool,
    luma_context: Option<LumaTransformTypeContext>,
    bit_depth: BitDepth,
    availability: MiddleEdgeAvailability,
) -> core::result::Result<(), GeneralIntraResidualError> {
    let side = 1usize << log2_side;
    let log2 = u8::try_from(log2_side).unwrap_or(u8::MAX);
    let block_size = IntraRectBlockSize::new(log2, log2)?;
    let edges = workspace.intra_dc_edges_for_rect(plane_id, x, y, block_size)?;
    let left_samples = availability.left.then(|| edges.left_samples()).flatten();
    let above_samples = availability.above.then(|| edges.above_samples()).flatten();
    if (availability.left && left_samples.is_none())
        || (availability.above && above_samples.is_none())
    {
        return Err(GeneralIntraResidualError::UnsupportedDirectionalAboveEdge);
    }
    let have_left = left_samples.is_some();
    let have_above = above_samples.is_some();
    let above_corner = if have_left && have_above {
        match (x.checked_sub(1), y.checked_sub(1)) {
            (Some(cx), Some(cy)) => Some(workspace.reconstructed_sample(plane_id, cx, cy)?),
            _ => None,
        }
    } else {
        None
    };
    let (left, above) = build_directional_middle_edges(
        left_samples,
        above_samples,
        above_corner,
        have_left,
        have_above,
        side,
        bit_depth,
    )?;
    let angle = middle_directional_angle(mode)?;
    let mut prediction = workspace.take_intra_prediction_buffer(
        IntraPredictionScratchBuffer::Primary,
        plane_id,
        side * side,
        T::default(),
    )?;
    if matches!(plane_id, PlaneId::Y) {
        let (left_idif, above_idif) =
            extend_directional_middle_idif_edges(&left, &above, bit_depth);
        predict_intra_middle_directional_angle_rect_idif_into(
            bit_depth,
            block_size,
            angle,
            IntraMiddleDirectionalAngleIdifEdges::both(&left_idif, &above_idif),
            &mut prediction,
            side,
        )?;
    } else {
        predict_intra_middle_directional_angle_rect_into(
            bit_depth,
            block_size,
            angle,
            IntraMiddleDirectionalAngleEdges::both(&left, &above),
            &mut prediction,
            side,
        )?;
    }
    write_intra_prediction_block(
        workspace,
        block,
        prediction,
        IntraPredictionScratchBuffer::Primary,
        plane_id,
        x,
        y,
        log2_side,
        log2_side,
        qindex,
        use_tcq,
        luma_context,
        None,
        bit_depth,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn reconstruct_general_intra_middle_neighbour_rect_block_into<T: ReconSample>(
    workspace: &mut CurrentFrameWorkspace<T>,
    block: &LumaCoeffBlock,
    p_angle: u16,
    plane_id: PlaneId,
    x: usize,
    y: usize,
    log2_width: u32,
    log2_height: u32,
    qindex: u32,
    use_tcq: bool,
    luma_context: Option<LumaTransformTypeContext>,
    dpcm: Option<DpcmDirection>,
    bit_depth: BitDepth,
    availability: MiddleEdgeAvailability,
    filters: TwoSidedMiddleEdgeFilters,
) -> core::result::Result<(), GeneralIntraResidualError> {
    let width = 1usize << log2_width;
    let height = 1usize << log2_height;
    let log2_w = u8::try_from(log2_width).unwrap_or(u8::MAX);
    let log2_h = u8::try_from(log2_height).unwrap_or(u8::MAX);
    let block_size = IntraRectBlockSize::new(log2_w, log2_h)?;
    let angle = IntraMiddleDirectionalAngle::try_from_p_angle(p_angle)?;
    let edges = workspace.intra_dc_edges_for_rect(plane_id, x, y, block_size)?;
    let left_samples = availability.left.then(|| edges.left_samples()).flatten();
    let above_samples = availability.above.then(|| edges.above_samples()).flatten();
    if (availability.left && left_samples.is_none())
        || (availability.above && above_samples.is_none())
    {
        return Err(GeneralIntraResidualError::UnsupportedDirectionalAboveEdge);
    }
    let mut prediction = workspace.take_intra_prediction_buffer(
        IntraPredictionScratchBuffer::Primary,
        plane_id,
        width * height,
        T::default(),
    )?;

    if left_samples.is_some() && above_samples.is_some() {
        let above_row = y
            .checked_sub(1)
            .ok_or(GeneralIntraResidualError::UnsupportedDirectionalAboveEdge)?;
        let left_col = x
            .checked_sub(1)
            .ok_or(GeneralIntraResidualError::UnsupportedDirectionalAboveEdge)?;
        let max_y = workspace
            .plane(plane_id)?
            .storage_size()
            .height()
            .checked_sub(1)
            .ok_or(GeneralIntraResidualError::UnsupportedDirectionalAboveEdge)?;
        let corner = workspace.reconstructed_sample(plane_id, left_col, above_row)?;
        let above_idif = build_two_sided_middle_idif_edge(width, filters.above, corner, |i| {
            workspace.reconstructed_sample(plane_id, x.saturating_add(i), above_row)
        })?;
        let left_idif = build_two_sided_middle_idif_edge(height, filters.left, corner, |i| {
            workspace.reconstructed_sample(plane_id, left_col, y.saturating_add(i).min(max_y))
        })?;
        if matches!(plane_id, PlaneId::Y) {
            predict_intra_middle_directional_angle_rect_idif_into(
                bit_depth,
                block_size,
                angle,
                IntraMiddleDirectionalAngleIdifEdges::both(&left_idif, &above_idif),
                &mut prediction,
                width,
            )?;
        } else {
            let left = &left_idif[1..height + 2];
            let above = &above_idif[1..width + 2];
            predict_intra_middle_directional_angle_rect_into(
                bit_depth,
                block_size,
                angle,
                IntraMiddleDirectionalAngleEdges::both(left, above),
                &mut prediction,
                width,
            )?;
        }
    } else {
        let (left, above) = build_directional_middle_rect_edges(
            left_samples,
            above_samples,
            None,
            availability.left,
            availability.above,
            width,
            height,
            bit_depth,
        )?;
        if matches!(plane_id, PlaneId::Y) {
            let left_idif = if availability.left {
                build_two_sided_middle_idif_edge(height, filters.left, left[0], |i| {
                    Ok::<T, splot_recon::ReconError>(left[i + 1])
                })?
            } else {
                extend_one_middle_idif_edge(&left, bit_depth)
            };
            let above_idif = if availability.above {
                build_two_sided_middle_idif_edge(width, filters.above, above[0], |i| {
                    Ok::<T, splot_recon::ReconError>(above[i + 1])
                })?
            } else {
                extend_one_middle_idif_edge(&above, bit_depth)
            };
            predict_intra_middle_directional_angle_rect_idif_into(
                bit_depth,
                block_size,
                angle,
                IntraMiddleDirectionalAngleIdifEdges::both(&left_idif, &above_idif),
                &mut prediction,
                width,
            )?;
        } else {
            predict_intra_middle_directional_angle_rect_into(
                bit_depth,
                block_size,
                angle,
                IntraMiddleDirectionalAngleEdges::both(&left, &above),
                &mut prediction,
                width,
            )?;
        }
    }
    write_intra_prediction_block(
        workspace,
        block,
        prediction,
        IntraPredictionScratchBuffer::Primary,
        plane_id,
        x,
        y,
        log2_width,
        log2_height,
        qindex,
        use_tcq,
        luma_context,
        dpcm,
        bit_depth,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn reconstruct_general_intra_two_sided_middle_luma_mrl_block_into<T: ReconSample>(
    workspace: &mut CurrentFrameWorkspace<T>,
    block: &LumaCoeffBlock,
    p_angle: u16,
    x: usize,
    y: usize,
    log2_width: u32,
    log2_height: u32,
    qindex: u32,
    mrl_index: usize,
    above_mrl_index: usize,
    is_sb_boundary: bool,
    secondary_mrl: bool,
    use_tcq: bool,
    luma_context: Option<LumaTransformTypeContext>,
    availability: MiddleEdgeAvailability,
    bit_depth: BitDepth,
) -> core::result::Result<(), GeneralIntraResidualError> {
    let width = 1usize << log2_width;
    let height = 1usize << log2_height;
    let mut prediction = workspace.take_intra_prediction_buffer(
        IntraPredictionScratchBuffer::Primary,
        PlaneId::Y,
        width * height,
        T::default(),
    )?;
    predict_two_sided_middle_luma_mrl_into(
        workspace,
        p_angle,
        x,
        y,
        log2_width,
        log2_height,
        mrl_index,
        above_mrl_index,
        is_sb_boundary,
        availability,
        bit_depth,
        &mut prediction,
    )?;
    if secondary_mrl {
        let mut secondary = workspace.take_intra_prediction_buffer(
            IntraPredictionScratchBuffer::Secondary,
            PlaneId::Y,
            width * height,
            T::default(),
        )?;
        predict_two_sided_middle_luma_mrl_into(
            workspace,
            p_angle,
            x,
            y,
            log2_width,
            log2_height,
            0,
            0,
            false,
            availability,
            bit_depth,
            &mut secondary,
        )?;
        let blend = average_luma_prediction_with(&mut prediction, &secondary);
        workspace
            .recycle_intra_prediction_buffer(IntraPredictionScratchBuffer::Secondary, secondary);
        blend?;
    }
    write_intra_prediction_block(
        workspace,
        block,
        prediction,
        IntraPredictionScratchBuffer::Primary,
        PlaneId::Y,
        x,
        y,
        log2_width,
        log2_height,
        qindex,
        use_tcq,
        luma_context,
        None,
        bit_depth,
    )
}

#[allow(clippy::too_many_arguments)]
fn predict_two_sided_middle_luma_mrl_into<T: ReconSample>(
    workspace: &CurrentFrameWorkspace<T>,
    p_angle: u16,
    x: usize,
    y: usize,
    log2_width: u32,
    log2_height: u32,
    mrl_index: usize,
    above_mrl_index: usize,
    is_sb_boundary: bool,
    availability: MiddleEdgeAvailability,
    bit_depth: BitDepth,
    prediction: &mut [T],
) -> core::result::Result<(), GeneralIntraResidualError> {
    let width = 1usize << log2_width;
    let height = 1usize << log2_height;
    let log2_w = u8::try_from(log2_width).unwrap_or(u8::MAX);
    let log2_h = u8::try_from(log2_height).unwrap_or(u8::MAX);
    let block_size = IntraRectBlockSize::new(log2_w, log2_h)?;
    let angle = IntraMiddleDirectionalAngle::try_from_p_angle(p_angle)?;
    let (above_idif, left_idif) = match (availability.above, availability.left) {
        (true, true) => (
            build_two_sided_middle_mrl_above_idif_edge(
                workspace,
                x,
                y,
                width,
                mrl_index,
                above_mrl_index,
                true,
            )?,
            build_two_sided_middle_mrl_left_idif_edge(
                workspace,
                x,
                y,
                height,
                mrl_index,
                is_sb_boundary,
            )?,
        ),
        (true, false) => (
            build_two_sided_middle_mrl_above_idif_edge(
                workspace,
                x,
                y,
                width,
                mrl_index,
                above_mrl_index,
                false,
            )?,
            build_above_only_middle_mrl_left_idif_edge(
                workspace,
                x,
                y,
                height,
                mrl_index,
                above_mrl_index,
            )?,
        ),
        (false, true) => (
            build_top_row_left_only_middle_mrl_above_idif_edge(workspace, x, y, width, mrl_index)?,
            build_top_row_left_only_middle_mrl_left_idif_edge(workspace, x, y, height, mrl_index)?,
        ),
        (false, false) => {
            let corner = noneighbour_corner::<T>(bit_depth);
            (
                build_two_sided_middle_mrl_idif_edge(
                    width,
                    mrl_index,
                    i32::try_from(width)
                        .ok()
                        .and_then(|width| width.checked_add(1))
                        .ok_or(GeneralIntraResidualError::UnsupportedDirectionalAboveEdge)?,
                    |logical| {
                        Ok(if logical < 0 {
                            corner
                        } else {
                            noneighbour_above::<T>(bit_depth)
                        })
                    },
                )?,
                build_two_sided_middle_mrl_idif_edge(
                    height,
                    mrl_index,
                    i32::try_from(height)
                        .ok()
                        .and_then(|height| height.checked_add(1))
                        .ok_or(GeneralIntraResidualError::UnsupportedDirectionalAboveEdge)?,
                    |logical| {
                        Ok(if logical < 0 {
                            corner
                        } else {
                            noneighbour_left::<T>(bit_depth)
                        })
                    },
                )?,
            )
        }
    };
    predict_intra_middle_directional_angle_rect_idif_mrl_into(
        bit_depth,
        block_size,
        angle,
        IntraMiddleDirectionalAngleIdifMrlEdges::both(&left_idif, &above_idif),
        mrl_index,
        prediction,
        width,
    )?;
    Ok(())
}

fn build_top_row_left_only_middle_mrl_above_idif_edge<T: ReconSample>(
    workspace: &CurrentFrameWorkspace<T>,
    x: usize,
    y: usize,
    width: usize,
    mrl_index: usize,
) -> core::result::Result<Vec<T>, GeneralIntraResidualError> {
    let left_col = x
        .checked_sub(1)
        .and_then(|col| col.checked_sub(mrl_index))
        .ok_or(GeneralIntraResidualError::UnsupportedDirectionalAboveEdge)?;
    let seed = workspace.reconstructed_sample(PlaneId::Y, left_col, y)?;
    let len = width
        .checked_add(mrl_index)
        .and_then(|v| v.checked_add(4))
        .ok_or(GeneralIntraResidualError::UnsupportedDirectionalAboveEdge)?;
    Ok(vec![seed; len])
}

fn build_top_row_left_only_middle_mrl_left_idif_edge<T: ReconSample>(
    workspace: &CurrentFrameWorkspace<T>,
    x: usize,
    y: usize,
    height: usize,
    mrl_index: usize,
) -> core::result::Result<Vec<T>, GeneralIntraResidualError> {
    let left_col = x
        .checked_sub(1)
        .and_then(|col| col.checked_sub(mrl_index))
        .ok_or(GeneralIntraResidualError::UnsupportedDirectionalAboveEdge)?;
    let seed = workspace.reconstructed_sample(PlaneId::Y, left_col, y)?;
    let max_logical = i32::try_from(height)
        .map_err(|_| GeneralIntraResidualError::UnsupportedDirectionalAboveEdge)?
        + 1;
    build_two_sided_middle_mrl_idif_edge(height, mrl_index, max_logical, |logical| {
        if logical < 0 {
            return Ok(seed);
        }
        let logical = usize::try_from(logical)
            .map_err(|_| GeneralIntraResidualError::UnsupportedDirectionalAboveEdge)?;
        let row = y.saturating_add(logical.min(height.saturating_sub(1)));
        Ok(workspace.reconstructed_sample(PlaneId::Y, left_col, row)?)
    })
}

fn build_above_only_middle_mrl_left_idif_edge<T: ReconSample>(
    workspace: &CurrentFrameWorkspace<T>,
    x: usize,
    y: usize,
    height: usize,
    mrl_index: usize,
    above_mrl_index: usize,
) -> core::result::Result<Vec<T>, GeneralIntraResidualError> {
    let above_row = y
        .checked_sub(1)
        .and_then(|row| row.checked_sub(above_mrl_index))
        .ok_or(GeneralIntraResidualError::UnsupportedDirectionalAboveEdge)?;
    let seed = workspace.reconstructed_sample(PlaneId::Y, x, above_row)?;
    let max_logical = i32::try_from(height)
        .map_err(|_| GeneralIntraResidualError::UnsupportedDirectionalAboveEdge)?
        + 1;
    build_two_sided_middle_mrl_idif_edge(height, mrl_index, max_logical, |_| Ok(seed))
}

fn build_two_sided_middle_mrl_above_idif_edge<T: ReconSample>(
    workspace: &CurrentFrameWorkspace<T>,
    x: usize,
    y: usize,
    width: usize,
    mrl_index: usize,
    above_mrl_index: usize,
    have_left: bool,
) -> core::result::Result<Vec<T>, GeneralIntraResidualError> {
    let above_row = y
        .checked_sub(1)
        .and_then(|row| row.checked_sub(above_mrl_index))
        .ok_or(GeneralIntraResidualError::UnsupportedDirectionalAboveEdge)?;
    let max_logical = i32::try_from(width)
        .map_err(|_| GeneralIntraResidualError::UnsupportedDirectionalAboveEdge)?
        + 1;
    build_two_sided_middle_mrl_idif_edge(width, mrl_index, max_logical, |logical| {
        let column = if logical < 0 {
            if have_left {
                let back = usize::try_from(-logical)
                    .map_err(|_| GeneralIntraResidualError::UnsupportedDirectionalAboveEdge)?;
                x.checked_sub(back)
                    .ok_or(GeneralIntraResidualError::UnsupportedDirectionalAboveEdge)?
            } else {
                x
            }
        } else {
            let logical = usize::try_from(logical)
                .map_err(|_| GeneralIntraResidualError::UnsupportedDirectionalAboveEdge)?;
            x.saturating_add(logical.min(width.saturating_sub(1)))
        };
        Ok(workspace.reconstructed_sample(PlaneId::Y, column, above_row)?)
    })
}

fn build_two_sided_middle_mrl_left_idif_edge<T: ReconSample>(
    workspace: &CurrentFrameWorkspace<T>,
    x: usize,
    y: usize,
    height: usize,
    mrl_index: usize,
    is_sb_boundary: bool,
) -> core::result::Result<Vec<T>, GeneralIntraResidualError> {
    let left_col = x
        .checked_sub(1)
        .and_then(|col| col.checked_sub(mrl_index))
        .ok_or(GeneralIntraResidualError::UnsupportedDirectionalAboveEdge)?;
    let max_logical = i32::try_from(height)
        .map_err(|_| GeneralIntraResidualError::UnsupportedDirectionalAboveEdge)?
        + 1;
    let max_y = workspace
        .plane(PlaneId::Y)?
        .storage_size()
        .height()
        .checked_sub(1)
        .ok_or(GeneralIntraResidualError::UnsupportedDirectionalAboveEdge)?;
    build_two_sided_middle_mrl_idif_edge(height, mrl_index, max_logical, |logical| {
        let row = if logical < 0 {
            if is_sb_boundary {
                y.checked_sub(1)
                    .ok_or(GeneralIntraResidualError::UnsupportedDirectionalAboveEdge)?
            } else {
                let back = usize::try_from(-logical)
                    .map_err(|_| GeneralIntraResidualError::UnsupportedDirectionalAboveEdge)?;
                y.checked_sub(back)
                    .ok_or(GeneralIntraResidualError::UnsupportedDirectionalAboveEdge)?
            }
        } else {
            let logical = usize::try_from(logical)
                .map_err(|_| GeneralIntraResidualError::UnsupportedDirectionalAboveEdge)?;
            y.saturating_add(logical.min(height.saturating_sub(1)))
                .min(max_y)
        };
        Ok(workspace.reconstructed_sample(PlaneId::Y, left_col, row)?)
    })
}

fn build_two_sided_middle_mrl_idif_edge<T: ReconSample>(
    side: usize,
    mrl_index: usize,
    max_logical: i32,
    sample: impl Fn(i32) -> core::result::Result<T, GeneralIntraResidualError>,
) -> core::result::Result<Vec<T>, GeneralIntraResidualError> {
    let len = side
        .checked_add(mrl_index)
        .and_then(|v| v.checked_add(4))
        .ok_or(GeneralIntraResidualError::UnsupportedDirectionalAboveEdge)?;
    let mrl = i32::try_from(mrl_index)
        .map_err(|_| GeneralIntraResidualError::UnsupportedDirectionalAboveEdge)?;
    let min_base = -1 - mrl;
    let mut edge = vec![T::default(); len];
    for logical in min_base..=max_logical {
        let offset = usize::try_from(logical + mrl + 2)
            .map_err(|_| GeneralIntraResidualError::UnsupportedDirectionalAboveEdge)?;
        edge[offset] = sample(logical)?;
    }
    edge[0] = edge[1];
    Ok(edge)
}

fn build_two_sided_middle_idif_edge<T: ReconSample>(
    side: usize,
    edge_filter: OneSidedEdgeFilter,
    corner: T,
    in_edge: impl Fn(usize) -> core::result::Result<T, splot_recon::ReconError>,
) -> core::result::Result<Vec<T>, GeneralIntraResidualError> {
    let max_base = side
        .checked_sub(1)
        .ok_or(GeneralIntraResidualError::UnsupportedDirectionalAboveEdge)?;
    let edge_len = side
        .checked_add(4)
        .ok_or(GeneralIntraResidualError::UnsupportedDirectionalAboveEdge)?;
    let mut edge = vec![T::default(); edge_len];
    edge[1] = corner;
    for i in 0..side {
        edge[i + 2] = in_edge(i)?;
    }
    finalize_one_sided_idif_edge(&mut edge, max_base, edge_filter)?;
    Ok(edge)
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn reconstruct_general_intra_cardinal_neighbour_block_into<T: ReconSample>(
    workspace: &mut CurrentFrameWorkspace<T>,
    block: &LumaCoeffBlock,
    direction: IntraCardinalDirection,
    plane_id: PlaneId,
    x: usize,
    y: usize,
    log2_width: u32,
    log2_height: u32,
    qindex: u32,
    use_tcq: bool,
    luma_context: Option<LumaTransformTypeContext>,
    dpcm: Option<DpcmDirection>,
    availability: IntraEdgeAvailability,
    bit_depth: BitDepth,
) -> core::result::Result<(), GeneralIntraResidualError> {
    let width = 1usize << log2_width;
    let height = 1usize << log2_height;
    let log2_w = u8::try_from(log2_width).unwrap_or(u8::MAX);
    let log2_h = u8::try_from(log2_height).unwrap_or(u8::MAX);
    let block_size = IntraRectBlockSize::new(log2_w, log2_h)?;
    let edges = workspace.intra_dc_edges_for_rect(plane_id, x, y, block_size)?;
    let above_samples = availability.above.then(|| edges.above_samples()).flatten();
    let left_samples = availability.left.then(|| edges.left_samples()).flatten();
    let synthesized_edge;
    let cardinal_edges = match direction {
        IntraCardinalDirection::Vertical => {
            if let Some(above) = above_samples {
                IntraDirectionalAngleEdges::above(above)
            } else if let Some(left) = left_samples {
                let fill = *left.first().unwrap_or(&noneighbour_above::<T>(bit_depth));
                synthesized_edge = vec![fill; width];
                IntraDirectionalAngleEdges::above(&synthesized_edge)
            } else {
                synthesized_edge = vec![noneighbour_above::<T>(bit_depth); width];
                IntraDirectionalAngleEdges::above(&synthesized_edge)
            }
        }
        IntraCardinalDirection::Horizontal => {
            if let Some(left) = left_samples {
                IntraDirectionalAngleEdges::left(left)
            } else if let Some(above) = above_samples {
                let fill = *above.first().unwrap_or(&noneighbour_left::<T>(bit_depth));
                synthesized_edge = vec![fill; height];
                IntraDirectionalAngleEdges::left(&synthesized_edge)
            } else {
                synthesized_edge = vec![noneighbour_left::<T>(bit_depth); height];
                IntraDirectionalAngleEdges::left(&synthesized_edge)
            }
        }
    };
    let mut prediction = workspace.take_intra_prediction_buffer(
        IntraPredictionScratchBuffer::Primary,
        plane_id,
        width * height,
        T::default(),
    )?;
    predict_intra_cardinal_directional_rect_into(
        bit_depth,
        block_size,
        direction,
        cardinal_edges,
        &mut prediction,
        width,
    )?;
    write_intra_prediction_block(
        workspace,
        block,
        prediction,
        IntraPredictionScratchBuffer::Primary,
        plane_id,
        x,
        y,
        log2_width,
        log2_height,
        qindex,
        use_tcq,
        luma_context,
        dpcm,
        bit_depth,
    )
}

fn build_directional_middle_edges<T: ReconSample>(
    left_neighbour: Option<&[T]>,
    above_neighbour: Option<&[T]>,
    above_corner: Option<T>,
    have_left: bool,
    have_above: bool,
    side: usize,
    bit_depth: BitDepth,
) -> core::result::Result<(Vec<T>, Vec<T>), GeneralIntraResidualError> {
    build_directional_middle_rect_edges(
        left_neighbour,
        above_neighbour,
        above_corner,
        have_left,
        have_above,
        side,
        side,
        bit_depth,
    )
}

#[allow(clippy::too_many_arguments)]
fn build_directional_middle_rect_edges<T: ReconSample>(
    left_neighbour: Option<&[T]>,
    above_neighbour: Option<&[T]>,
    above_corner: Option<T>,
    have_left: bool,
    have_above: bool,
    width: usize,
    height: usize,
    bit_depth: BitDepth,
) -> core::result::Result<(Vec<T>, Vec<T>), GeneralIntraResidualError> {
    let mut left = Vec::with_capacity(height + 1);
    let mut above = Vec::with_capacity(width + 1);
    match (have_left, have_above) {
        (true, true) => {
            let Some(corner) = above_corner else {
                return Err(GeneralIntraResidualError::UnsupportedDirectionalAboveEdge);
            };
            let left_samples = left_neighbour.unwrap_or(&[]);
            let above_samples = above_neighbour.unwrap_or(&[]);
            left.push(corner);
            above.push(corner);
            for i in 0..height {
                left.push(sample_or_last(
                    left_samples,
                    i,
                    noneighbour_left::<T>(bit_depth),
                ));
            }
            for i in 0..width {
                above.push(sample_or_last(
                    above_samples,
                    i,
                    noneighbour_above::<T>(bit_depth),
                ));
            }
        }
        (false, true) => {
            let above_samples = above_neighbour.unwrap_or(&[]);
            let seed = above_samples
                .first()
                .copied()
                .unwrap_or(noneighbour_above::<T>(bit_depth));
            left.push(seed);
            above.push(seed);
            for _ in 0..height {
                left.push(seed);
            }
            for i in 0..width {
                above.push(sample_or_last(
                    above_samples,
                    i,
                    noneighbour_above::<T>(bit_depth),
                ));
            }
        }
        (true, false) => {
            let left_samples = left_neighbour.unwrap_or(&[]);
            let seed = left_samples
                .first()
                .copied()
                .unwrap_or(noneighbour_left::<T>(bit_depth));
            left.push(seed);
            above.push(seed);
            for i in 0..height {
                left.push(sample_or_last(
                    left_samples,
                    i,
                    noneighbour_left::<T>(bit_depth),
                ));
            }
            for _ in 0..width {
                above.push(seed);
            }
        }
        (false, false) => {
            left.push(noneighbour_corner::<T>(bit_depth));
            above.push(noneighbour_corner::<T>(bit_depth));
            for _ in 0..height {
                left.push(noneighbour_left::<T>(bit_depth));
            }
            for _ in 0..width {
                above.push(noneighbour_above::<T>(bit_depth));
            }
        }
    }
    Ok((left, above))
}

fn sample_or_last<T: ReconSample>(samples: &[T], index: usize, fallback: T) -> T {
    samples
        .get(index)
        .or_else(|| samples.last())
        .copied()
        .unwrap_or(fallback)
}

pub(super) fn middle_directional_angle(
    mode: SupportedDirectionalLumaMode,
) -> core::result::Result<IntraMiddleDirectionalAngle, GeneralIntraResidualError> {
    match mode {
        SupportedDirectionalLumaMode::D113 => Ok(IntraMiddleDirectionalAngle::D113),
        SupportedDirectionalLumaMode::D135 => Ok(IntraMiddleDirectionalAngle::D135),
        SupportedDirectionalLumaMode::D157 => Ok(IntraMiddleDirectionalAngle::D157),
        SupportedDirectionalLumaMode::Vertical
        | SupportedDirectionalLumaMode::Horizontal
        | SupportedDirectionalLumaMode::D45
        | SupportedDirectionalLumaMode::D67
        | SupportedDirectionalLumaMode::D203 => {
            Err(GeneralIntraResidualError::CardinalModeInMiddleAnglePath)
        }
    }
}

fn extend_directional_middle_idif_edges<T: ReconSample>(
    left: &[T],
    above: &[T],
    bit_depth: BitDepth,
) -> (Vec<T>, Vec<T>) {
    (
        extend_one_middle_idif_edge(left, bit_depth),
        extend_one_middle_idif_edge(above, bit_depth),
    )
}

fn extend_one_middle_idif_edge<T: ReconSample>(edge: &[T], bit_depth: BitDepth) -> Vec<T> {
    let corner = edge
        .first()
        .copied()
        .unwrap_or(noneighbour_corner::<T>(bit_depth));
    let last = edge.last().copied().unwrap_or(corner);
    let mut out = Vec::with_capacity(edge.len() + 3);
    out.push(corner); // logical -2 == Edge[-1]
    out.extend_from_slice(edge); // logical -1..side-1
    out.push(last); // logical side == Edge[side - 1]
    out.push(last); // logical side + 1 == Edge[side - 1]
    out
}
