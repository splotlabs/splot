// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Smooth intra prediction entries, edge assembly, and sentinel resolution.

use splot_recon::{
    BitDepth, CurrentFrameWorkspace, IntraRectBlockSize, IntraSmoothEdges, IntraSmoothMode,
    PlaneId, ReconSample, predict_intra_smooth_rect_into,
};

use super::sink::{IntraEdgeAvailability, noneighbour_above, noneighbour_left};
use crate::bitstream::tile_payload::{
    GeneralIntraResidualError, LumaCoeffBlock, LumaTransformTypeContext, SupportedNonDcLumaMode,
    reconstruct_general_intra_coeff_block_rect_with_prediction,
    reconstruct_general_intra_luma_block_rect_with_prediction_and_ist,
};

#[allow(clippy::too_many_arguments)]
pub(crate) fn reconstruct_general_intra_luma_smooth_rect_block_with_availability_into<
    T: ReconSample,
>(
    workspace: &mut CurrentFrameWorkspace<T>,
    block: &LumaCoeffBlock,
    mode: SupportedNonDcLumaMode,
    x: usize,
    y: usize,
    log2_width: u32,
    log2_height: u32,
    qindex: u32,
    use_tcq: bool,
    num4_above_right: usize,
    num4_below_left: usize,
    luma_context: Option<LumaTransformTypeContext>,
    availability: IntraEdgeAvailability,
    bit_depth: BitDepth,
) -> core::result::Result<(), GeneralIntraResidualError> {
    let smooth_mode = match mode {
        SupportedNonDcLumaMode::Smooth => IntraSmoothMode::Smooth,
        SupportedNonDcLumaMode::SmoothVertical => IntraSmoothMode::SmoothVertical,
        SupportedNonDcLumaMode::SmoothHorizontal => IntraSmoothMode::SmoothHorizontal,
    };
    let (available_left_samples, available_above_samples) = availability.available_sample_limits();
    reconstruct_general_intra_smooth_over_available_edges_into(
        workspace,
        block,
        PlaneId::Y,
        x,
        y,
        log2_width,
        log2_height,
        qindex,
        smooth_mode,
        available_left_samples,
        available_above_samples,
        num4_above_right,
        num4_below_left,
        use_tcq,
        luma_context,
        bit_depth,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn reconstruct_general_intra_smooth_over_available_edges_into<T: ReconSample>(
    workspace: &mut CurrentFrameWorkspace<T>,
    block: &LumaCoeffBlock,
    plane_id: PlaneId,
    x: usize,
    y: usize,
    log2_width: u32,
    log2_height: u32,
    qindex: u32,
    smooth_mode: IntraSmoothMode,
    available_left_samples: Option<usize>,
    available_above_samples: Option<usize>,
    num4_above_right: usize,
    num4_below_left: usize,
    use_tcq: bool,
    luma_context: Option<LumaTransformTypeContext>,
    bit_depth: BitDepth,
) -> core::result::Result<(), GeneralIntraResidualError> {
    let log2_w = u8::try_from(log2_width).unwrap_or(u8::MAX);
    let log2_h = u8::try_from(log2_height).unwrap_or(u8::MAX);
    let block_size = IntraRectBlockSize::new(log2_w, log2_h)?;
    let prediction = predict_intra_smooth_over_available_edges(
        workspace,
        SmoothIntraPredictionRequest {
            plane_id,
            x,
            y,
            block_size,
            mode: smooth_mode,
            available_left_samples,
            available_above_samples,
            num4_above_right,
            num4_below_left,
            bit_depth,
        },
    )?;
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
            None,
            bit_depth,
        )?
    };
    workspace.write_rect_block(plane_id, x, y, block_size, &out)?;
    Ok(())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct SmoothIntraPredictionRequest {
    pub(crate) plane_id: PlaneId,
    pub(crate) x: usize,
    pub(crate) y: usize,
    pub(crate) block_size: IntraRectBlockSize,
    pub(crate) mode: IntraSmoothMode,
    pub(crate) available_left_samples: Option<usize>,
    pub(crate) available_above_samples: Option<usize>,
    pub(crate) num4_above_right: usize,
    pub(crate) num4_below_left: usize,
    pub(crate) bit_depth: BitDepth,
}

pub(crate) fn predict_intra_smooth_over_available_edges<T: ReconSample>(
    workspace: &CurrentFrameWorkspace<T>,
    request: SmoothIntraPredictionRequest,
) -> core::result::Result<Vec<T>, GeneralIntraResidualError> {
    let SmoothIntraPredictionRequest {
        plane_id,
        x,
        y,
        block_size,
        mode,
        available_left_samples,
        available_above_samples,
        num4_above_right,
        num4_below_left,
        bit_depth,
    } = request;
    let width = block_size.width();
    let height = block_size.height();
    let edges = workspace.intra_dc_edges_for_rect(plane_id, x, y, block_size)?;
    let left_len =
        available_left_samples.unwrap_or_else(|| edges.left_samples().map_or(0, <[T]>::len));
    let above_len =
        available_above_samples.unwrap_or_else(|| edges.above_samples().map_or(0, <[T]>::len));
    let left = edges
        .left_samples()
        .map(|samples| &samples[..left_len.min(samples.len())]);
    let above = edges
        .above_samples()
        .map(|samples| &samples[..above_len.min(samples.len())]);
    let have_left = left.is_some_and(|samples| !samples.is_empty());
    let have_above = above.is_some_and(|samples| !samples.is_empty());
    let above_right_sentinel = if above_len >= width {
        resolve_smooth_above_right_sentinel(
            workspace,
            plane_id,
            x,
            y,
            width,
            have_above,
            num4_above_right,
        )?
    } else {
        None
    };
    let bottom_left_sentinel = if left_len >= height {
        resolve_smooth_bottom_left_sentinel(
            workspace,
            plane_id,
            x,
            y,
            height,
            have_left,
            num4_below_left,
        )?
    } else {
        None
    };
    let (left, above) = build_smooth_edges(
        left,
        above,
        have_left,
        have_above,
        width,
        height,
        above_right_sentinel,
        bottom_left_sentinel,
        bit_depth,
    );
    let smooth_edges = IntraSmoothEdges::new(&left, &above);
    let mut prediction = vec![T::default(); width * height];
    predict_intra_smooth_rect_into(
        bit_depth,
        block_size,
        mode,
        smooth_edges,
        &mut prediction,
        width,
    )?;
    Ok(prediction)
}

#[allow(clippy::too_many_arguments)]
fn build_smooth_edges<T: ReconSample>(
    left_neighbour: Option<&[T]>,
    above_neighbour: Option<&[T]>,
    have_left: bool,
    have_above: bool,
    width: usize,
    height: usize,
    above_right_sentinel: Option<T>,
    bottom_left_sentinel: Option<T>,
    bit_depth: BitDepth,
) -> (Vec<T>, Vec<T>) {
    let left_len = height + 1;
    let above_len = width + 1;
    let left = match (have_left, left_neighbour) {
        (true, Some(samples)) => fill_edge_from_neighbour(samples, left_len, bit_depth),
        _ if have_above => {
            let seed = above_neighbour
                .and_then(|samples| samples.first().copied())
                .unwrap_or(noneighbour_left::<T>(bit_depth));
            vec![seed; left_len]
        }
        _ => vec![noneighbour_left::<T>(bit_depth); left_len],
    };
    let mut above = match (have_above, above_neighbour) {
        (true, Some(samples)) => fill_edge_from_neighbour(samples, above_len, bit_depth),
        _ if have_left => {
            let seed = left_neighbour
                .and_then(|samples| samples.first().copied())
                .unwrap_or(noneighbour_above::<T>(bit_depth));
            vec![seed; above_len]
        }
        _ => vec![noneighbour_above::<T>(bit_depth); above_len],
    };
    if let Some(sentinel) = above_right_sentinel
        && let Some(slot) = above.get_mut(width)
    {
        *slot = sentinel;
    }
    let mut left = left;
    if let Some(sentinel) = bottom_left_sentinel
        && let Some(slot) = left.get_mut(height)
    {
        *slot = sentinel;
    }
    (left, above)
}

fn resolve_smooth_above_right_sentinel<T: ReconSample>(
    workspace: &CurrentFrameWorkspace<T>,
    plane_id: PlaneId,
    x: usize,
    y: usize,
    side: usize,
    have_above: bool,
    num4_above_right: usize,
) -> core::result::Result<Option<T>, GeneralIntraResidualError> {
    resolve_smooth_sentinel(
        workspace,
        SmoothSentinelRequest {
            plane_id,
            x,
            y,
            side,
            have_edge: have_above,
            extension: num4_above_right,
            kind: SmoothSentinelKind::AboveRight,
        },
    )
}

fn resolve_smooth_bottom_left_sentinel<T: ReconSample>(
    workspace: &CurrentFrameWorkspace<T>,
    plane_id: PlaneId,
    x: usize,
    y: usize,
    height: usize,
    have_left: bool,
    num4_below_left: usize,
) -> core::result::Result<Option<T>, GeneralIntraResidualError> {
    resolve_smooth_sentinel(
        workspace,
        SmoothSentinelRequest {
            plane_id,
            x,
            y,
            side: height,
            have_edge: have_left,
            extension: num4_below_left,
            kind: SmoothSentinelKind::BottomLeft,
        },
    )
}

#[derive(Clone, Copy)]
enum SmoothSentinelKind {
    AboveRight,
    BottomLeft,
}

#[derive(Clone, Copy)]
struct SmoothSentinelRequest {
    plane_id: PlaneId,
    x: usize,
    y: usize,
    side: usize,
    have_edge: bool,
    extension: usize,
    kind: SmoothSentinelKind,
}

fn resolve_smooth_sentinel<T: ReconSample>(
    workspace: &CurrentFrameWorkspace<T>,
    request: SmoothSentinelRequest,
) -> core::result::Result<Option<T>, GeneralIntraResidualError> {
    let SmoothSentinelRequest {
        plane_id,
        x,
        y,
        side,
        have_edge,
        extension,
        kind,
    } = request;
    if !have_edge || extension == 0 {
        return Ok(None);
    }
    let storage = workspace.plane(plane_id)?.storage_size();
    match kind {
        SmoothSentinelKind::AboveRight => {
            let Some(max_x) = storage.width().checked_sub(1) else {
                return Ok(None);
            };
            let Some(sample_y) = y.checked_sub(1) else {
                return Ok(None);
            };
            let sample_x = x.saturating_add(side);
            if sample_x > max_x {
                return Ok(None);
            }
            Ok(Some(
                workspace.reconstructed_sample(plane_id, sample_x, sample_y)?,
            ))
        }
        SmoothSentinelKind::BottomLeft => {
            let Some(max_y) = storage.height().checked_sub(1) else {
                return Ok(None);
            };
            let Some(sample_x) = x.checked_sub(1) else {
                return Ok(None);
            };
            let sample_y = y.saturating_add(side);
            if sample_y > max_y {
                return Ok(None);
            }
            Ok(Some(
                workspace.reconstructed_sample(plane_id, sample_x, sample_y)?,
            ))
        }
    }
}

fn fill_edge_from_neighbour<T: ReconSample>(
    samples: &[T],
    edge_len: usize,
    bit_depth: BitDepth,
) -> Vec<T> {
    let mut edge = Vec::with_capacity(edge_len);
    for i in 0..edge_len {
        let sample = samples
            .get(i)
            .or_else(|| samples.last())
            .copied()
            .unwrap_or(noneighbour_left::<T>(bit_depth));
        edge.push(sample);
    }
    edge
}
