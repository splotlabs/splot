// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! First-block, DIP, and Paeth intra entries with their edge helpers.

use splot_recon::{
    BitDepth, CurrentFrameWorkspace, IntraDipEdges, IntraMiddleDirectionalAngleEdges,
    IntraPaethEdges, IntraPredictionScratchBuffer, IntraRectBlockSize, IntraSmoothEdges,
    IntraSmoothMode, PlaneId, ReconSample, predict_intra_dip_rect_into,
    predict_intra_middle_directional_angle_rect_into, predict_intra_paeth_rect_into,
    predict_intra_smooth_rect_into,
};

use super::middle::middle_directional_angle;
use super::sink::{
    IntraEdgeAvailability, noneighbour_above, noneighbour_corner, noneighbour_left,
    write_intra_prediction_block,
};
use crate::bitstream::tile_payload::{
    GeneralIntraResidualError, LumaCoeffBlock, LumaTransformTypeContext,
    SupportedDirectionalLumaMode, SupportedNonDcLumaMode,
};
use crate::pipeline::general_intra::RecycledIntraSamples;

const MI_SIZE: usize = 4;

#[allow(clippy::too_many_arguments)]
pub(crate) fn reconstruct_general_intra_luma_dip_rect_block_into<T: ReconSample>(
    workspace: &mut CurrentFrameWorkspace<T>,
    block: &LumaCoeffBlock,
    dip_mode: u8,
    dip_transpose: bool,
    x: usize,
    y: usize,
    log2_width: u32,
    log2_height: u32,
    qindex: u32,
    use_tcq: bool,
    num4_above_right: usize,
    num4_below_left: usize,
    luma_context: LumaTransformTypeContext,
    availability: IntraEdgeAvailability,
    bit_depth: BitDepth,
) -> core::result::Result<(), GeneralIntraResidualError> {
    let width = 1usize << log2_width;
    let height = 1usize << log2_height;
    let log2_w = u8::try_from(log2_width).unwrap_or(u8::MAX);
    let log2_h = u8::try_from(log2_height).unwrap_or(u8::MAX);
    let block_size = IntraRectBlockSize::new(log2_w, log2_h)?;
    let (left, above, top_left) = dip_reference_edges(
        workspace,
        x,
        y,
        width,
        height,
        num4_above_right,
        num4_below_left,
        availability,
        bit_depth,
    )?;
    let mut prediction = workspace.take_intra_prediction_buffer(
        IntraPredictionScratchBuffer::Primary,
        PlaneId::Y,
        width * height,
        T::default(),
    )?;
    predict_intra_dip_rect_into(
        bit_depth,
        block_size,
        usize::from(dip_mode),
        dip_transpose,
        IntraDipEdges::new(&left, &above, top_left),
        &mut prediction,
        width,
    )?;
    write_intra_prediction_block(
        workspace,
        block,
        prediction,
        PlaneId::Y,
        x,
        y,
        block_size,
        qindex,
        use_tcq,
        Some(luma_context),
        None,
        bit_depth,
    )
}

#[allow(clippy::too_many_arguments)]
fn reconstruct_general_intra_luma_first_block_with<T: ReconSample, F>(
    workspace: &mut CurrentFrameWorkspace<T>,
    block: &LumaCoeffBlock,
    x: usize,
    y: usize,
    log2_side: u32,
    qindex: u32,
    use_tcq: bool,
    luma_context: LumaTransformTypeContext,
    bit_depth: BitDepth,
    build_prediction: F,
) -> core::result::Result<(), GeneralIntraResidualError>
where
    F: FnOnce(
        IntraRectBlockSize,
        usize,
        BitDepth,
        &mut [T],
    ) -> core::result::Result<(), GeneralIntraResidualError>,
{
    let (side, block_size) = luma_square_prediction_geometry(log2_side)?;
    let mut prediction = workspace.take_intra_prediction_buffer(
        IntraPredictionScratchBuffer::Primary,
        PlaneId::Y,
        side * side,
        T::default(),
    )?;
    build_prediction(block_size, side, bit_depth, &mut prediction)?;
    write_intra_prediction_block(
        workspace,
        block,
        prediction,
        PlaneId::Y,
        x,
        y,
        block_size,
        qindex,
        use_tcq,
        Some(luma_context),
        None,
        bit_depth,
    )
}

fn luma_square_prediction_geometry(
    log2_side: u32,
) -> core::result::Result<(usize, IntraRectBlockSize), GeneralIntraResidualError> {
    let side = 1usize << log2_side;
    let log2 = u8::try_from(log2_side).unwrap_or(u8::MAX);
    Ok((side, IntraRectBlockSize::new(log2, log2)?))
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn reconstruct_general_intra_luma_nondc_first_block_into<T: ReconSample>(
    workspace: &mut CurrentFrameWorkspace<T>,
    block: &LumaCoeffBlock,
    mode: SupportedNonDcLumaMode,
    x: usize,
    y: usize,
    log2_side: u32,
    qindex: u32,
    use_tcq: bool,
    luma_context: LumaTransformTypeContext,
    bit_depth: BitDepth,
) -> core::result::Result<(), GeneralIntraResidualError> {
    reconstruct_general_intra_luma_first_block_with(
        workspace,
        block,
        x,
        y,
        log2_side,
        qindex,
        use_tcq,
        luma_context,
        bit_depth,
        |block_size, side, bit_depth, prediction| {
            predict_nondc_noneighbour_smooth(mode, block_size, side, bit_depth, prediction)
        },
    )
}

fn predict_nondc_noneighbour_smooth<T: ReconSample>(
    mode: SupportedNonDcLumaMode,
    block_size: IntraRectBlockSize,
    side: usize,
    bit_depth: BitDepth,
    prediction: &mut [T],
) -> core::result::Result<(), GeneralIntraResidualError> {
    let smooth_mode = match mode {
        SupportedNonDcLumaMode::Smooth => IntraSmoothMode::Smooth,
        SupportedNonDcLumaMode::SmoothVertical => IntraSmoothMode::SmoothVertical,
        SupportedNonDcLumaMode::SmoothHorizontal => IntraSmoothMode::SmoothHorizontal,
    };
    let above = [noneighbour_above::<T>(bit_depth); 65];
    let left = [noneighbour_left::<T>(bit_depth); 65];
    let edges = IntraSmoothEdges::new(&left[..=side], &above[..=side]);
    predict_intra_smooth_rect_into(bit_depth, block_size, smooth_mode, edges, prediction, side)?;
    Ok(())
}

pub(crate) fn predict_directional_noneighbour_into<T: ReconSample>(
    mode: SupportedDirectionalLumaMode,
    block_size: IntraRectBlockSize,
    side: usize,
    bit_depth: BitDepth,
    prediction: &mut [T],
) -> core::result::Result<(), GeneralIntraResidualError> {
    let angle = middle_directional_angle(mode)?;
    let mut above = [noneighbour_above::<T>(bit_depth); 65];
    let mut left = [noneighbour_left::<T>(bit_depth); 65];
    above[0] = noneighbour_corner::<T>(bit_depth);
    left[0] = noneighbour_corner::<T>(bit_depth);
    let edges = IntraMiddleDirectionalAngleEdges::both(&left[..=side], &above[..=side]);
    predict_intra_middle_directional_angle_rect_into(
        bit_depth, block_size, angle, edges, prediction, side,
    )?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn dip_reference_edges<T: ReconSample>(
    workspace: &CurrentFrameWorkspace<T>,
    x: usize,
    y: usize,
    width: usize,
    height: usize,
    num4_above_right: usize,
    num4_below_left: usize,
    availability: IntraEdgeAvailability,
    bit_depth: BitDepth,
) -> core::result::Result<
    (RecycledIntraSamples<T>, RecycledIntraSamples<T>, T),
    GeneralIntraResidualError,
> {
    let above_len = width + (width >> 2);
    let left_len = height + (height >> 2);
    let above = if availability.above {
        collect_available_dip_edge(
            workspace,
            DipEdge::Above,
            x,
            y,
            width,
            above_len,
            num4_above_right,
        )?
    } else if availability.left {
        RecycledIntraSamples::filled(above_len, left_seed(workspace, x, y)?)
    } else {
        RecycledIntraSamples::filled(above_len, noneighbour_above::<T>(bit_depth))
    };
    let left = if availability.left {
        collect_available_dip_edge(
            workspace,
            DipEdge::Left,
            x,
            y,
            height,
            left_len,
            num4_below_left,
        )?
    } else if availability.above {
        RecycledIntraSamples::filled(left_len, above_seed(workspace, x, y)?)
    } else {
        RecycledIntraSamples::filled(left_len, noneighbour_left::<T>(bit_depth))
    };
    let top_left = match (availability.above, availability.left) {
        (true, true) => {
            let corner_x = x
                .checked_sub(1)
                .ok_or(GeneralIntraResidualError::UnsupportedDirectionalAboveEdge)?;
            let corner_y = y
                .checked_sub(1)
                .ok_or(GeneralIntraResidualError::UnsupportedDirectionalAboveEdge)?;
            workspace.reconstructed_sample(PlaneId::Y, corner_x, corner_y)?
        }
        (true, false) => above_seed(workspace, x, y)?,
        (false, true) => left_seed(workspace, x, y)?,
        (false, false) => noneighbour_corner::<T>(bit_depth),
    };
    Ok((left, above, top_left))
}

#[derive(Clone, Copy)]
enum DipEdge {
    Above,
    Left,
}

#[allow(clippy::too_many_arguments)]
fn collect_available_dip_edge<T: ReconSample>(
    workspace: &CurrentFrameWorkspace<T>,
    direction: DipEdge,
    x: usize,
    y: usize,
    initial_len: usize,
    edge_len: usize,
    extra_units: usize,
) -> core::result::Result<RecycledIntraSamples<T>, GeneralIntraResidualError> {
    let fixed = match direction {
        DipEdge::Above => y,
        DipEdge::Left => x,
    }
    .checked_sub(1)
    .ok_or(GeneralIntraResidualError::UnsupportedDirectionalAboveEdge)?;
    let storage_size = workspace.plane(PlaneId::Y)?.storage_size();
    let storage_len = match direction {
        DipEdge::Above => storage_size.width(),
        DipEdge::Left => storage_size.height(),
    };
    let start = match direction {
        DipEdge::Above => x,
        DipEdge::Left => y,
    };
    let readable = initial_len
        .saturating_add(extra_units.saturating_mul(MI_SIZE))
        .min(edge_len);
    let mut edge = RecycledIntraSamples::with_capacity(edge_len);
    for offset in 0..readable {
        let Some(variable) = start.checked_add(offset) else {
            break;
        };
        if variable >= storage_len {
            break;
        }
        let (sample_x, sample_y) = match direction {
            DipEdge::Above => (variable, fixed),
            DipEdge::Left => (fixed, variable),
        };
        edge.push(workspace.reconstructed_sample(PlaneId::Y, sample_x, sample_y)?);
    }
    extend_edge_with_last(edge, edge_len)
}

fn extend_edge_with_last<T: ReconSample>(
    mut edge: RecycledIntraSamples<T>,
    edge_len: usize,
) -> core::result::Result<RecycledIntraSamples<T>, GeneralIntraResidualError> {
    let last = edge
        .last()
        .copied()
        .ok_or(GeneralIntraResidualError::UnsupportedDirectionalAboveEdge)?;
    edge.resize(edge_len, last);
    Ok(edge)
}

fn left_seed<T: ReconSample>(
    workspace: &CurrentFrameWorkspace<T>,
    x: usize,
    y: usize,
) -> core::result::Result<T, GeneralIntraResidualError> {
    let sample_x = x
        .checked_sub(1)
        .ok_or(GeneralIntraResidualError::UnsupportedDirectionalAboveEdge)?;
    Ok(workspace.reconstructed_sample(PlaneId::Y, sample_x, y)?)
}

fn above_seed<T: ReconSample>(
    workspace: &CurrentFrameWorkspace<T>,
    x: usize,
    y: usize,
) -> core::result::Result<T, GeneralIntraResidualError> {
    let sample_y = y
        .checked_sub(1)
        .ok_or(GeneralIntraResidualError::UnsupportedDirectionalAboveEdge)?;
    Ok(workspace.reconstructed_sample(PlaneId::Y, x, sample_y)?)
}

#[allow(clippy::too_many_arguments)]
fn paeth_reference_edges_into<T: ReconSample>(
    workspace: &CurrentFrameWorkspace<T>,
    plane_id: PlaneId,
    x: usize,
    y: usize,
    width: usize,
    height: usize,
    bit_depth: BitDepth,
    above_in: Option<&[T]>,
    left_in: Option<&[T]>,
    above: &mut Vec<T>,
    left: &mut Vec<T>,
) -> core::result::Result<T, GeneralIntraResidualError> {
    above.clear();
    match (above_in, left_in) {
        (Some(a), _) => {
            above.extend(a.iter().take(width).copied());
        }
        (None, Some(l)) => {
            above.resize(
                width,
                *l.first().unwrap_or(&noneighbour_above::<T>(bit_depth)),
            );
        }
        (None, None) => {
            above.resize(width, noneighbour_above::<T>(bit_depth));
        }
    }
    left.clear();
    match (left_in, above_in) {
        (Some(l), _) => {
            left.extend(l.iter().take(height).copied());
        }
        (None, Some(a)) => {
            left.resize(
                height,
                *a.first().unwrap_or(&noneighbour_left::<T>(bit_depth)),
            );
        }
        (None, None) => {
            left.resize(height, noneighbour_left::<T>(bit_depth));
        }
    }
    if above.len() != width || left.len() != height {
        return Err(GeneralIntraResidualError::UnsupportedDirectionalAboveEdge);
    }
    let top_left = match (above_in, left_in) {
        (Some(_), Some(_)) => {
            let (Some(corner_x), Some(corner_y)) = (x.checked_sub(1), y.checked_sub(1)) else {
                return Err(GeneralIntraResidualError::UnsupportedDirectionalAboveEdge);
            };
            workspace.reconstructed_sample(plane_id, corner_x, corner_y)?
        }
        (Some(above), None) => *above.first().unwrap_or(&noneighbour_corner::<T>(bit_depth)),
        (None, Some(left)) => *left.first().unwrap_or(&noneighbour_corner::<T>(bit_depth)),
        (None, None) => noneighbour_corner::<T>(bit_depth),
    };
    Ok(top_left)
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn reconstruct_general_intra_luma_paeth_neighbour_block_into<T: ReconSample>(
    scratch: &mut crate::pipeline::general_intra::GeneralIntraReconScratch<T>,
    workspace: &mut CurrentFrameWorkspace<T>,
    block: &LumaCoeffBlock,
    plane_id: PlaneId,
    x: usize,
    y: usize,
    log2_width: u32,
    log2_height: u32,
    qindex: u32,
    use_tcq: bool,
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

    let [above, left] = &mut scratch.paeth_edges;
    {
        let top_left = paeth_reference_edges_into(
            workspace,
            plane_id,
            x,
            y,
            width,
            height,
            bit_depth,
            above_samples,
            left_samples,
            above,
            left,
        )?;
        let mut prediction = workspace.take_intra_prediction_buffer(
            IntraPredictionScratchBuffer::Primary,
            plane_id,
            width * height,
            T::default(),
        )?;
        predict_intra_paeth_rect_into(
            bit_depth,
            block_size,
            IntraPaethEdges::new(left, above, top_left),
            &mut prediction,
            width,
        )?;
        write_intra_prediction_block(
            workspace, block, prediction, plane_id, x, y, block_size, qindex, use_tcq, None, None,
            bit_depth,
        )
    }
}
