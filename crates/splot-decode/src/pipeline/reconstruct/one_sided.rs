// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! One-sided directional, MRL, and cardinal-MRL intra entries with idif edge builders.

use core::ops::Deref;

use splot_recon::{
    BitDepth, CurrentFrameWorkspace, DpcmDirection, IntraCardinalDirection, IntraDirectionalAngle,
    IntraDirectionalAngleEdges, IntraDirectionalAngleIdifEdges, IntraRectBlockSize, PlaneId,
    ReconSample, apply_intra_edge_filter, filter_intra_edge_corner,
    predict_intra_directional_angle_rect_into,
    predict_intra_directional_angle_rect_one_sided_idif_mrl_into,
};

use super::sink::{
    IntraEdgeAvailability, average_luma_prediction_with, noneighbour_above, noneighbour_corner,
    noneighbour_left, write_luma_prediction_block,
};
use crate::bitstream::tile_payload::{
    GeneralIntraResidualError, LumaCoeffBlock, LumaTransformTypeContext,
    reconstruct_general_intra_coeff_block_rect_with_prediction,
    reconstruct_general_intra_luma_block_rect_with_prediction_and_ist,
};

const ONE_SIDED_IDIF_EDGE_CAPACITY: usize = 138;

pub(crate) struct OneSidedIdifEdge<T: ReconSample> {
    samples: [T; ONE_SIDED_IDIF_EDGE_CAPACITY],
    len: usize,
}

impl<T: ReconSample> OneSidedIdifEdge<T> {
    fn new(len: usize) -> core::result::Result<Self, GeneralIntraResidualError> {
        if len > ONE_SIDED_IDIF_EDGE_CAPACITY {
            return Err(GeneralIntraResidualError::UnsupportedDirectionalAboveEdge);
        }
        Ok(Self {
            samples: [T::default(); ONE_SIDED_IDIF_EDGE_CAPACITY],
            len,
        })
    }
}

impl<T: ReconSample> Deref for OneSidedIdifEdge<T> {
    type Target = [T];

    fn deref(&self) -> &Self::Target {
        &self.samples[..self.len]
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct OneSidedEdgeFilter {
    pub strength: u8,
    pub num_px: usize,
    pub corner_opposite: Option<u16>,
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct OneSidedAboveMrl {
    pub mrl_index: usize,
    pub above_mrl_index: usize,
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn reconstruct_general_intra_one_sided_neighbour_block_into<T: ReconSample>(
    workspace: &mut CurrentFrameWorkspace<T>,
    block: &LumaCoeffBlock,
    p_angle: u16,
    plane_id: PlaneId,
    x: usize,
    y: usize,
    log2_width: u32,
    log2_height: u32,
    qindex: u32,
    num4_above_right: usize,
    mrl: OneSidedAboveMrl,
    use_tcq: bool,
    luma_context: Option<LumaTransformTypeContext>,
    dpcm: Option<DpcmDirection>,
    availability: IntraEdgeAvailability,
    bit_depth: BitDepth,
    edge_filter: OneSidedEdgeFilter,
) -> core::result::Result<(), GeneralIntraResidualError> {
    let width = 1usize << log2_width;
    let height = 1usize << log2_height;
    let log2_w = u8::try_from(log2_width).unwrap_or(u8::MAX);
    let log2_h = u8::try_from(log2_height).unwrap_or(u8::MAX);
    let block_size = IntraRectBlockSize::new(log2_w, log2_h)?;
    let prediction = if matches!(plane_id, PlaneId::Y) {
        predict_general_intra_luma_one_sided_above_mrl(
            workspace,
            p_angle,
            x,
            y,
            log2_width,
            log2_height,
            num4_above_right,
            mrl,
            availability,
            bit_depth,
            edge_filter,
        )?
    } else {
        if mrl.mrl_index != 0 {
            return Err(GeneralIntraResidualError::UnsupportedDirectionalAboveEdge);
        }
        let above_idif = build_one_sided_above_idif_edge(
            workspace,
            plane_id,
            x,
            y,
            width,
            height,
            num4_above_right,
            mrl.mrl_index,
            mrl.above_mrl_index,
            availability,
            bit_depth,
            edge_filter,
        )?;
        let mut prediction = vec![T::default(); width * height];
        let above_bilinear = &above_idif[2..2 + width + height];
        predict_intra_directional_angle_rect_into(
            bit_depth,
            block_size,
            IntraDirectionalAngle::try_from_p_angle(p_angle)?,
            IntraDirectionalAngleEdges::above(above_bilinear),
            &mut prediction,
            width,
        )?;
        prediction
    };
    let out = if block.all_zero {
        prediction
    } else if let Some(luma_context) = luma_context {
        reconstruct_general_intra_luma_block_rect_with_prediction_and_ist(
            block,
            &prediction,
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
            &prediction,
            qindex,
            plane_id,
            log2_width,
            log2_height,
            use_tcq,
            dpcm,
            bit_depth,
        )?
    };
    workspace.write_rect_block(plane_id, x, y, block_size, &out)?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn predict_general_intra_luma_one_sided_above_mrl<T: ReconSample>(
    workspace: &CurrentFrameWorkspace<T>,
    p_angle: u16,
    x: usize,
    y: usize,
    log2_width: u32,
    log2_height: u32,
    num4_above_right: usize,
    mrl: OneSidedAboveMrl,
    availability: IntraEdgeAvailability,
    bit_depth: BitDepth,
    edge_filter: OneSidedEdgeFilter,
) -> core::result::Result<Vec<T>, GeneralIntraResidualError> {
    let width = 1usize << log2_width;
    let height = 1usize << log2_height;
    let above_idif = build_one_sided_above_idif_edge(
        workspace,
        PlaneId::Y,
        x,
        y,
        width,
        height,
        num4_above_right,
        mrl.mrl_index,
        mrl.above_mrl_index,
        availability,
        bit_depth,
        edge_filter,
    )?;
    predict_general_intra_luma_one_sided_idif_mrl(
        bit_depth,
        p_angle,
        log2_width,
        log2_height,
        IntraDirectionalAngleIdifEdges::above(&above_idif),
        mrl.mrl_index,
    )
}

fn predict_general_intra_luma_one_sided_idif_mrl<T: ReconSample>(
    bit_depth: BitDepth,
    p_angle: u16,
    log2_width: u32,
    log2_height: u32,
    edges: IntraDirectionalAngleIdifEdges<'_, T>,
    mrl_index: usize,
) -> core::result::Result<Vec<T>, GeneralIntraResidualError> {
    let width = 1usize << log2_width;
    let height = 1usize << log2_height;
    let log2_w = u8::try_from(log2_width).unwrap_or(u8::MAX);
    let log2_h = u8::try_from(log2_height).unwrap_or(u8::MAX);
    let block_size = IntraRectBlockSize::new(log2_w, log2_h)?;
    let mut prediction = vec![T::default(); width * height];
    predict_intra_directional_angle_rect_one_sided_idif_mrl_into(
        bit_depth,
        block_size,
        IntraDirectionalAngle::try_from_p_angle(p_angle)?,
        edges,
        mrl_index,
        &mut prediction,
        width,
    )?;
    Ok(prediction)
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn reconstruct_general_intra_mrl_secondary_above_block_into<T: ReconSample>(
    workspace: &mut CurrentFrameWorkspace<T>,
    block: &LumaCoeffBlock,
    p_angle: u16,
    x: usize,
    y: usize,
    log2_width: u32,
    log2_height: u32,
    qindex: u32,
    num4_above_right: usize,
    primary_mrl: OneSidedAboveMrl,
    use_tcq: bool,
    availability: IntraEdgeAvailability,
    bit_depth: BitDepth,
) -> core::result::Result<(), GeneralIntraResidualError> {
    let mut prediction = predict_general_intra_luma_one_sided_above_mrl(
        workspace,
        p_angle,
        x,
        y,
        log2_width,
        log2_height,
        num4_above_right,
        primary_mrl,
        availability,
        bit_depth,
        OneSidedEdgeFilter::default(),
    )?;
    let secondary = predict_general_intra_luma_one_sided_above_mrl(
        workspace,
        p_angle,
        x,
        y,
        log2_width,
        log2_height,
        num4_above_right,
        OneSidedAboveMrl::default(),
        availability,
        bit_depth,
        OneSidedEdgeFilter::default(),
    )?;
    average_luma_prediction_with(&mut prediction, secondary)?;
    write_luma_prediction_block(
        workspace,
        block,
        prediction,
        x,
        y,
        log2_width,
        log2_height,
        qindex,
        use_tcq,
        bit_depth,
        None,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn reconstruct_general_intra_cardinal_mrl_luma_block_into<T: ReconSample>(
    workspace: &mut CurrentFrameWorkspace<T>,
    block: &LumaCoeffBlock,
    direction: IntraCardinalDirection,
    x: usize,
    y: usize,
    log2_width: u32,
    log2_height: u32,
    qindex: u32,
    mrl_index: usize,
    above_mrl_index: usize,
    secondary_mrl: bool,
    use_tcq: bool,
    availability: IntraEdgeAvailability,
    bit_depth: BitDepth,
) -> core::result::Result<(), GeneralIntraResidualError> {
    let width = 1usize << log2_width;
    let height = 1usize << log2_height;
    let log2_w = u8::try_from(log2_width).unwrap_or(u8::MAX);
    let log2_h = u8::try_from(log2_height).unwrap_or(u8::MAX);
    let block_size = IntraRectBlockSize::new(log2_w, log2_h)?;
    let mut prediction = cardinal_mrl_luma_prediction(
        workspace,
        direction,
        x,
        y,
        width,
        height,
        mrl_index,
        above_mrl_index,
        availability,
        bit_depth,
    )?;
    if secondary_mrl {
        let secondary = cardinal_mrl_luma_prediction(
            workspace,
            direction,
            x,
            y,
            width,
            height,
            0,
            0,
            availability,
            bit_depth,
        )?;
        average_luma_prediction_with(&mut prediction, secondary)?;
    }
    let out = if block.all_zero {
        prediction
    } else {
        reconstruct_general_intra_coeff_block_rect_with_prediction(
            block,
            &prediction,
            qindex,
            PlaneId::Y,
            log2_width,
            log2_height,
            use_tcq,
            None,
            bit_depth,
        )?
    };
    workspace.write_rect_block(PlaneId::Y, x, y, block_size, &out)?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn cardinal_mrl_luma_prediction<T: ReconSample>(
    workspace: &CurrentFrameWorkspace<T>,
    direction: IntraCardinalDirection,
    x: usize,
    y: usize,
    width: usize,
    height: usize,
    mrl_index: usize,
    above_mrl_index: usize,
    availability: IntraEdgeAvailability,
    bit_depth: BitDepth,
) -> core::result::Result<Vec<T>, GeneralIntraResidualError> {
    let mut prediction = vec![T::default(); width * height];
    match direction {
        IntraCardinalDirection::Vertical if availability.above => {
            let above_row = y
                .checked_sub(1)
                .and_then(|row| row.checked_sub(above_mrl_index))
                .ok_or(GeneralIntraResidualError::UnsupportedDirectionalAboveEdge)?;
            for column in 0..width {
                let sample = workspace.reconstructed_sample(
                    PlaneId::Y,
                    x.saturating_add(column),
                    above_row,
                )?;
                for row in 0..height {
                    prediction[row * width + column] = sample;
                }
            }
        }
        IntraCardinalDirection::Vertical => {
            let sample = if availability.left {
                let left_col = x
                    .checked_sub(1)
                    .and_then(|col| col.checked_sub(mrl_index))
                    .ok_or(GeneralIntraResidualError::UnsupportedDirectionalAboveEdge)?;
                workspace.reconstructed_sample(PlaneId::Y, left_col, y)?
            } else {
                noneighbour_above::<T>(bit_depth)
            };
            prediction.fill(sample);
        }
        IntraCardinalDirection::Horizontal if availability.left => {
            let left_col = x
                .checked_sub(1)
                .and_then(|col| col.checked_sub(mrl_index))
                .ok_or(GeneralIntraResidualError::UnsupportedDirectionalAboveEdge)?;
            for row in 0..height {
                let sample =
                    workspace.reconstructed_sample(PlaneId::Y, left_col, y.saturating_add(row))?;
                for column in 0..width {
                    prediction[row * width + column] = sample;
                }
            }
        }
        IntraCardinalDirection::Horizontal => {
            let sample = if availability.above {
                let above_row = y
                    .checked_sub(1)
                    .and_then(|row| row.checked_sub(above_mrl_index))
                    .ok_or(GeneralIntraResidualError::UnsupportedDirectionalAboveEdge)?;
                workspace.reconstructed_sample(PlaneId::Y, x, above_row)?
            } else {
                noneighbour_left::<T>(bit_depth)
            };
            prediction.fill(sample);
        }
    }
    Ok(prediction)
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn build_one_sided_above_idif_edge<T: ReconSample>(
    workspace: &CurrentFrameWorkspace<T>,
    plane_id: PlaneId,
    x: usize,
    y: usize,
    width: usize,
    height: usize,
    num4_above_right: usize,
    mrl_index: usize,
    above_mrl_index: usize,
    availability: IntraEdgeAvailability,
    bit_depth: BitDepth,
    edge_filter: OneSidedEdgeFilter,
) -> core::result::Result<OneSidedIdifEdge<T>, GeneralIntraResidualError> {
    if !availability.above {
        if !availability.left {
            return build_one_sided_idif_edge(
                width,
                height,
                mrl_index,
                edge_filter,
                || Ok(noneighbour_corner::<T>(bit_depth)),
                |_| Ok(noneighbour_above::<T>(bit_depth)),
            );
        }
        let fallback_col = x
            .checked_sub(mrl_index + 1)
            .ok_or(GeneralIntraResidualError::UnsupportedDirectionalAboveEdge)?;
        let fallback = workspace.reconstructed_sample(plane_id, fallback_col, y)?;
        return build_one_sided_idif_edge(
            width,
            height,
            mrl_index,
            edge_filter,
            || Ok(fallback),
            |_| Ok(fallback),
        );
    }
    let above_row = y
        .checked_sub(1)
        .and_then(|row| row.checked_sub(above_mrl_index))
        .ok_or(GeneralIntraResidualError::UnsupportedDirectionalAboveEdge)?;
    let max_x = workspace
        .plane(plane_id)?
        .storage_size()
        .width()
        .checked_sub(1)
        .ok_or(GeneralIntraResidualError::UnsupportedDirectionalAboveEdge)?;
    let above_limit = width
        .checked_add(num4_above_right.saturating_mul(4))
        .and_then(|v| v.checked_sub(1))
        .and_then(|v| x.checked_add(v))
        .map_or(max_x, |limit| limit.min(max_x));
    let corner_col = if availability.left {
        x.checked_sub(1).unwrap_or(x)
    } else {
        x
    };
    let edge = build_one_sided_idif_edge(
        width,
        height,
        mrl_index,
        edge_filter,
        || workspace.reconstructed_sample(plane_id, corner_col, above_row),
        |i| {
            let column = x.saturating_add(i).min(above_limit);
            workspace.reconstructed_sample(plane_id, column, above_row)
        },
    )?;
    Ok(edge)
}

fn build_one_sided_idif_edge<T: ReconSample>(
    width: usize,
    height: usize,
    mrl_index: usize,
    edge_filter: OneSidedEdgeFilter,
    corner: impl FnOnce() -> core::result::Result<T, splot_recon::ReconError>,
    in_edge: impl Fn(usize) -> core::result::Result<T, splot_recon::ReconError>,
) -> core::result::Result<OneSidedIdifEdge<T>, GeneralIntraResidualError> {
    let mrl_span = mrl_index
        .checked_mul(2)
        .ok_or(GeneralIntraResidualError::UnsupportedDirectionalAboveEdge)?;
    let max_base = width
        .checked_add(height)
        .and_then(|v| v.checked_sub(1))
        .and_then(|v| v.checked_add(mrl_span))
        .ok_or(GeneralIntraResidualError::UnsupportedDirectionalAboveEdge)?;
    let edge_len = max_base
        .checked_add(5)
        .ok_or(GeneralIntraResidualError::UnsupportedDirectionalAboveEdge)?;
    let mut edge = OneSidedIdifEdge::new(edge_len)?;
    edge.samples[1] = corner()?;
    for i in 0..=max_base {
        let slot = i
            .checked_add(2)
            .ok_or(GeneralIntraResidualError::UnsupportedDirectionalAboveEdge)?;
        edge.samples[slot] = in_edge(i)?;
    }
    finalize_one_sided_idif_edge(&mut edge.samples[..edge.len], max_base, edge_filter)?;
    Ok(edge)
}

pub(super) fn finalize_one_sided_idif_edge<T: ReconSample>(
    edge: &mut [T],
    max_base: usize,
    filter: OneSidedEdgeFilter,
) -> core::result::Result<(), GeneralIntraResidualError> {
    if let Some(opposite) = filter.corner_opposite {
        let corner = edge
            .get(1)
            .ok_or(GeneralIntraResidualError::UnsupportedDirectionalAboveEdge)?
            .to_u16();
        let own0 = edge
            .get(2)
            .ok_or(GeneralIntraResidualError::UnsupportedDirectionalAboveEdge)?
            .to_u16();
        let filtered = filter_intra_edge_corner(opposite, corner, own0);
        edge[1] = T::try_from_u16(filtered)?;
    }
    if filter.num_px > 0
        && let Some(window) = edge.get_mut(1..)
    {
        apply_intra_edge_filter(window, filter.num_px, filter.strength)?;
    }
    edge[0] = edge[1];
    let last_in_edge = edge[max_base + 2];
    if let Some(slot) = edge.get_mut(max_base + 3) {
        *slot = last_in_edge;
    }
    if let Some(slot) = edge.get_mut(max_base + 4) {
        *slot = last_in_edge;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn reconstruct_general_intra_one_sided_left_neighbour_block_into<T: ReconSample>(
    workspace: &mut CurrentFrameWorkspace<T>,
    block: &LumaCoeffBlock,
    p_angle: u16,
    plane_id: PlaneId,
    x: usize,
    y: usize,
    log2_width: u32,
    log2_height: u32,
    qindex: u32,
    num4_below_left: usize,
    have_above: bool,
    mrl_index: usize,
    above_mrl_index: usize,
    use_tcq: bool,
    luma_context: Option<LumaTransformTypeContext>,
    dpcm: Option<DpcmDirection>,
    availability: IntraEdgeAvailability,
    bit_depth: BitDepth,
    edge_filter: OneSidedEdgeFilter,
) -> core::result::Result<(), GeneralIntraResidualError> {
    let width = 1usize << log2_width;
    let height = 1usize << log2_height;
    let log2_w = u8::try_from(log2_width).unwrap_or(u8::MAX);
    let log2_h = u8::try_from(log2_height).unwrap_or(u8::MAX);
    let block_size = IntraRectBlockSize::new(log2_w, log2_h)?;
    let prediction = if matches!(plane_id, PlaneId::Y) {
        predict_general_intra_luma_one_sided_left_mrl(
            workspace,
            p_angle,
            x,
            y,
            log2_width,
            log2_height,
            num4_below_left,
            have_above,
            mrl_index,
            above_mrl_index,
            availability.left,
            bit_depth,
            edge_filter,
        )?
    } else {
        if mrl_index != 0 {
            return Err(GeneralIntraResidualError::UnsupportedDirectionalAboveEdge);
        }
        let left_idif = build_one_sided_left_idif_edge(
            workspace,
            plane_id,
            x,
            y,
            width,
            height,
            num4_below_left,
            have_above,
            mrl_index,
            0,
            availability.left,
            bit_depth,
            edge_filter,
        )?;
        let mut prediction = vec![T::default(); width * height];
        let angle = IntraDirectionalAngle::try_from_p_angle(p_angle)?;
        let left_bilinear = &left_idif[2..2 + width + height];
        predict_intra_directional_angle_rect_into(
            bit_depth,
            block_size,
            angle,
            IntraDirectionalAngleEdges::left(left_bilinear),
            &mut prediction,
            width,
        )?;
        prediction
    };
    let out = if block.all_zero {
        prediction
    } else if let Some(luma_context) = luma_context {
        reconstruct_general_intra_luma_block_rect_with_prediction_and_ist(
            block,
            &prediction,
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
            &prediction,
            qindex,
            plane_id,
            log2_width,
            log2_height,
            use_tcq,
            dpcm,
            bit_depth,
        )?
    };
    workspace.write_rect_block(plane_id, x, y, block_size, &out)?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn predict_general_intra_luma_one_sided_left_mrl<T: ReconSample>(
    workspace: &CurrentFrameWorkspace<T>,
    p_angle: u16,
    x: usize,
    y: usize,
    log2_width: u32,
    log2_height: u32,
    num4_below_left: usize,
    have_above: bool,
    mrl_index: usize,
    above_mrl_index: usize,
    have_left: bool,
    bit_depth: BitDepth,
    edge_filter: OneSidedEdgeFilter,
) -> core::result::Result<Vec<T>, GeneralIntraResidualError> {
    let width = 1usize << log2_width;
    let height = 1usize << log2_height;
    let left_idif = build_one_sided_left_idif_edge(
        workspace,
        PlaneId::Y,
        x,
        y,
        width,
        height,
        num4_below_left,
        have_above,
        mrl_index,
        above_mrl_index,
        have_left,
        bit_depth,
        edge_filter,
    )?;
    predict_general_intra_luma_one_sided_idif_mrl(
        bit_depth,
        p_angle,
        log2_width,
        log2_height,
        IntraDirectionalAngleIdifEdges::left(&left_idif),
        mrl_index,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn reconstruct_general_intra_mrl_secondary_left_block_into<T: ReconSample>(
    workspace: &mut CurrentFrameWorkspace<T>,
    block: &LumaCoeffBlock,
    p_angle: u16,
    x: usize,
    y: usize,
    log2_width: u32,
    log2_height: u32,
    qindex: u32,
    num4_below_left: usize,
    have_above: bool,
    mrl_index: usize,
    above_mrl_index: usize,
    use_tcq: bool,
    have_left: bool,
    bit_depth: BitDepth,
) -> core::result::Result<(), GeneralIntraResidualError> {
    let mut prediction = predict_general_intra_luma_one_sided_left_mrl(
        workspace,
        p_angle,
        x,
        y,
        log2_width,
        log2_height,
        num4_below_left,
        have_above,
        mrl_index,
        above_mrl_index,
        have_left,
        bit_depth,
        OneSidedEdgeFilter::default(),
    )?;
    let secondary = predict_general_intra_luma_one_sided_left_mrl(
        workspace,
        p_angle,
        x,
        y,
        log2_width,
        log2_height,
        num4_below_left,
        have_above,
        0,
        0,
        have_left,
        bit_depth,
        OneSidedEdgeFilter::default(),
    )?;
    average_luma_prediction_with(&mut prediction, secondary)?;
    write_luma_prediction_block(
        workspace,
        block,
        prediction,
        x,
        y,
        log2_width,
        log2_height,
        qindex,
        use_tcq,
        bit_depth,
        None,
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn build_one_sided_left_idif_edge<T: ReconSample>(
    workspace: &CurrentFrameWorkspace<T>,
    plane_id: PlaneId,
    x: usize,
    y: usize,
    width: usize,
    height: usize,
    num4_below_left: usize,
    have_above: bool,
    mrl_index: usize,
    above_mrl_index: usize,
    have_left: bool,
    bit_depth: BitDepth,
    edge_filter: OneSidedEdgeFilter,
) -> core::result::Result<OneSidedIdifEdge<T>, GeneralIntraResidualError> {
    if !have_left {
        if !have_above {
            return build_one_sided_idif_edge(
                width,
                height,
                mrl_index,
                edge_filter,
                || Ok(noneighbour_corner::<T>(bit_depth)),
                |_| Ok(noneighbour_left::<T>(bit_depth)),
            );
        }
        let fallback_row = y
            .checked_sub(1)
            .and_then(|row| row.checked_sub(above_mrl_index))
            .ok_or(GeneralIntraResidualError::UnsupportedDirectionalAboveEdge)?;
        let fallback = workspace.reconstructed_sample(plane_id, x, fallback_row)?;
        return build_one_sided_idif_edge(
            width,
            height,
            mrl_index,
            edge_filter,
            || Ok(fallback),
            |_| Ok(fallback),
        );
    }
    let left_col = x
        .checked_sub(1)
        .and_then(|col| col.checked_sub(mrl_index))
        .ok_or(GeneralIntraResidualError::UnsupportedDirectionalAboveEdge)?;
    let max_y = workspace
        .plane(plane_id)?
        .storage_size()
        .height()
        .checked_sub(1)
        .ok_or(GeneralIntraResidualError::UnsupportedDirectionalAboveEdge)?;
    let left_limit = height
        .checked_add(num4_below_left.saturating_mul(4))
        .and_then(|v| v.checked_sub(1))
        .and_then(|v| y.checked_add(v))
        .map_or(max_y, |limit| limit.min(max_y));
    let corner_row = if have_above {
        y.checked_sub(1)
            .ok_or(GeneralIntraResidualError::UnsupportedDirectionalAboveEdge)?
    } else {
        y
    };
    build_one_sided_idif_edge(
        width,
        height,
        mrl_index,
        edge_filter,
        || workspace.reconstructed_sample(plane_id, left_col, corner_row),
        |i| {
            let row = y.saturating_add(i).min(left_limit);
            workspace.reconstructed_sample(plane_id, left_col, row)
        },
    )
}
