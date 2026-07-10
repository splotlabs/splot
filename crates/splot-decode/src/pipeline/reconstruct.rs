// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Prediction and residual reconstruction handoffs for decoded frames.
//!
//! Feature tracking: `DECODE-GENERAL-INTRA-FRAME-FRONTIER`.

use splot_recon::{
    BitDepth, CurrentFrameWorkspace, DecodedFrameInfo, DpcmDirection, IntraCardinalDirection,
    IntraDcEdges, IntraDipEdges, IntraDirectionalAngle, IntraDirectionalAngleEdges,
    IntraDirectionalAngleIdifEdges, IntraMiddleDirectionalAngle, IntraMiddleDirectionalAngleEdges,
    IntraMiddleDirectionalAngleIdifEdges, IntraMiddleDirectionalAngleIdifMrlEdges, IntraPaethEdges,
    IntraRectBlockSize, IntraSmoothEdges, IntraSmoothMode, OutputIndex, PixelFormat, PlaneId,
    PlaneRect, PlaneSize, ReconSample, apply_ibp_dr_blend_rect, apply_intra_edge_filter,
    apply_intra_ibp_dc_rect, filter_intra_edge_corner,
    predict_intra_cardinal_directional_rect_into, predict_intra_dc_rect_value,
    predict_intra_dip_rect_into, predict_intra_directional_angle_rect_into,
    predict_intra_directional_angle_rect_one_sided_idif_into,
    predict_intra_directional_angle_rect_one_sided_idif_mrl_into,
    predict_intra_middle_directional_angle_rect_idif_into,
    predict_intra_middle_directional_angle_rect_idif_mrl_into,
    predict_intra_middle_directional_angle_rect_into, predict_intra_paeth_rect_into,
    predict_intra_smooth_rect_into,
};

use crate::Result;
use crate::bitstream::tile_payload::{
    GeneralIntraResidualError, LumaCoeffBlock, LumaPalette, LumaTransformTypeContext,
    SupportedDirectionalLumaMode, SupportedNonDcLumaMode,
    reconstruct_general_intra_coeff_block_rect_with_prediction,
    reconstruct_general_intra_coeff_block_rect_with_prediction_and_ddt,
    reconstruct_general_intra_coeff_block_with_prediction,
    reconstruct_general_intra_luma_block_rect_with_prediction_and_ist,
};
pub(crate) use crate::prediction::chroma::cfl::reconstruct_general_intra_chroma_cfl_block_into;
pub(crate) use crate::prediction::chroma::directional::reconstruct_general_intra_chroma_block_into;

const MI_SIZE: usize = 4;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct IntraEdgeAvailability {
    pub(crate) above: bool,
    pub(crate) left: bool,
}

impl IntraEdgeAvailability {
    pub(crate) const fn new(above: bool, left: bool) -> Self {
        Self { above, left }
    }

    pub(crate) const fn available_sample_limits(self) -> (Option<usize>, Option<usize>) {
        (
            if self.left { None } else { Some(0) },
            if self.above { None } else { Some(0) },
        )
    }
}

pub(crate) fn new_general_intra_workspace<T: ReconSample>(
    luma_width: usize,
    luma_height: usize,
    bit_depth: BitDepth,
    pixel_format: PixelFormat,
) -> Result<CurrentFrameWorkspace<T>> {
    let luma_size = PlaneSize::new(luma_width, luma_height)?;
    let luma_rect = PlaneRect::new(0, 0, luma_width, luma_height)?;
    let info = DecodedFrameInfo::new(
        OutputIndex::new(0),
        bit_depth,
        pixel_format,
        luma_size,
        luma_rect,
    )?;
    Ok(CurrentFrameWorkspace::<T>::new(info, T::default())?)
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn reconstruct_general_intra_block_rect_with_availability_into<T: ReconSample>(
    workspace: &mut CurrentFrameWorkspace<T>,
    block: &LumaCoeffBlock,
    plane_id: PlaneId,
    x: usize,
    y: usize,
    log2_width: u32,
    log2_height: u32,
    qindex: u32,
    use_tcq: bool,
    ibp_dc: bool,
    luma_context: Option<LumaTransformTypeContext>,
    availability: IntraEdgeAvailability,
    bit_depth: BitDepth,
) -> core::result::Result<(), GeneralIntraResidualError> {
    let width = 1usize << log2_width;
    let height = 1usize << log2_height;
    let log2_w = u8::try_from(log2_width).unwrap_or(u8::MAX);
    let log2_h = u8::try_from(log2_height).unwrap_or(u8::MAX);
    let block_size = IntraRectBlockSize::new(log2_w, log2_h)?;
    let edges = workspace.intra_dc_edges_for_rect(plane_id, x, y, block_size)?;
    let dc_edges = IntraDcEdges::new(
        availability.left.then(|| edges.left_samples()).flatten(),
        availability.above.then(|| edges.above_samples()).flatten(),
    );
    let dc = predict_intra_dc_rect_value(bit_depth, block_size, dc_edges)?;
    let prediction = if ibp_dc {
        let mut pred = vec![dc; width * height];
        apply_intra_ibp_dc_rect(bit_depth, block_size, dc_edges, &mut pred, width)?;
        pred
    } else {
        vec![dc; width * height]
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
            None,
            bit_depth,
        )?
    };
    workspace.write_rect_block(plane_id, x, y, block_size, &out)?;
    Ok(())
}

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
    let mut prediction = vec![T::default(); width * height];
    predict_intra_dip_rect_into(
        bit_depth,
        block_size,
        usize::from(dip_mode),
        dip_transpose,
        IntraDipEdges::new(&left, &above, top_left),
        &mut prediction,
        width,
    )?;
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
        Some(luma_context),
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn reconstruct_general_intra_luma_palette_block_into<T: ReconSample>(
    workspace: &mut CurrentFrameWorkspace<T>,
    block: &LumaCoeffBlock,
    palette: LumaPalette,
    color_map: &[u8],
    x: usize,
    y: usize,
    log2_width: u32,
    log2_height: u32,
    qindex: u32,
    use_tcq: bool,
    luma_context: LumaTransformTypeContext,
    bit_depth: BitDepth,
) -> core::result::Result<(), GeneralIntraResidualError> {
    let width = 1usize << log2_width;
    let height = 1usize << log2_height;
    let log2_w = u8::try_from(log2_width).unwrap_or(u8::MAX);
    let log2_h = u8::try_from(log2_height).unwrap_or(u8::MAX);
    let block_size = IntraRectBlockSize::new(log2_w, log2_h)?;
    if color_map.len() != width.saturating_mul(height) {
        return Err(GeneralIntraResidualError::PredictionLength {
            expected: width.saturating_mul(height),
            actual: color_map.len(),
        });
    }
    let mut prediction = Vec::with_capacity(color_map.len());
    for &color_index in color_map {
        let sample =
            palette
                .sample(color_index)
                .ok_or(GeneralIntraResidualError::PaletteColorIndex {
                    color_index: usize::from(color_index),
                    palette_size: palette.size(),
                })?;
        prediction.push(T::try_from_u16(sample)?);
    }
    let out = if block.all_zero {
        prediction
    } else {
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
    };
    workspace.write_rect_block(PlaneId::Y, x, y, block_size, &out)?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn reconstruct_inter_block_residual_rect_into<T: ReconSample>(
    workspace: &mut CurrentFrameWorkspace<T>,
    block: &LumaCoeffBlock,
    plane_id: PlaneId,
    x: usize,
    y: usize,
    log2_width: u32,
    log2_height: u32,
    qindex: u32,
    use_tcq: bool,
    use_ddt: bool,
    bit_depth: BitDepth,
) -> core::result::Result<(), GeneralIntraResidualError> {
    let width = 1usize << log2_width;
    let height = 1usize << log2_height;
    let log2_w = u8::try_from(log2_width).unwrap_or(u8::MAX);
    let log2_h = u8::try_from(log2_height).unwrap_or(u8::MAX);
    let block_size = IntraRectBlockSize::new(log2_w, log2_h)?;
    if block.all_zero {
        return Ok(());
    }
    let rect = PlaneRect::new(x, y, width, height)?;
    let mut prediction = Vec::with_capacity(width * height);
    for row in workspace.rect_rows(plane_id, rect)? {
        prediction.extend_from_slice(row);
    }
    let out = reconstruct_general_intra_coeff_block_rect_with_prediction_and_ddt(
        block,
        &prediction,
        qindex,
        plane_id,
        log2_width,
        log2_height,
        use_tcq,
        use_ddt,
        bit_depth,
    )?;
    workspace.write_rect_block(plane_id, x, y, block_size, &out)?;
    Ok(())
}

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

fn noneighbour_above<T: ReconSample>(bit_depth: BitDepth) -> T {
    let half = 1u16 << (bit_depth.bits() - 1);
    noneighbour_sample::<T>(half - 1)
}

pub(crate) fn noneighbour_left<T: ReconSample>(bit_depth: BitDepth) -> T {
    let half = 1u16 << (bit_depth.bits() - 1);
    noneighbour_sample::<T>(half + 1)
}

fn noneighbour_corner<T: ReconSample>(bit_depth: BitDepth) -> T {
    let half = 1u16 << (bit_depth.bits() - 1);
    noneighbour_sample::<T>(half)
}

fn noneighbour_sample<T: ReconSample>(value: u16) -> T {
    debug_assert!(
        T::try_from_u16(value).is_ok(),
        "§7.13.2.1 no-neighbour fallback {value} does not fit the sample storage type for the active bit depth",
    );
    T::try_from_u16(value).unwrap_or_default()
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
    ) -> core::result::Result<Vec<T>, GeneralIntraResidualError>,
{
    let (side, block_size) = luma_square_prediction_geometry(log2_side)?;
    let prediction = build_prediction(block_size, side, bit_depth)?;
    write_luma_prediction_block(
        workspace,
        block,
        prediction,
        x,
        y,
        log2_side,
        log2_side,
        qindex,
        use_tcq,
        bit_depth,
        Some(luma_context),
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
        |block_size, side, bit_depth| {
            predict_nondc_noneighbour_smooth(mode, block_size, side, bit_depth)
        },
    )
}

fn predict_nondc_noneighbour_smooth<T: ReconSample>(
    mode: SupportedNonDcLumaMode,
    block_size: IntraRectBlockSize,
    side: usize,
    bit_depth: BitDepth,
) -> core::result::Result<Vec<T>, GeneralIntraResidualError> {
    let smooth_mode = match mode {
        SupportedNonDcLumaMode::Smooth => IntraSmoothMode::Smooth,
        SupportedNonDcLumaMode::SmoothVertical => IntraSmoothMode::SmoothVertical,
        SupportedNonDcLumaMode::SmoothHorizontal => IntraSmoothMode::SmoothHorizontal,
    };
    let above = vec![noneighbour_above::<T>(bit_depth); side + 1];
    let left = vec![noneighbour_left::<T>(bit_depth); side + 1];
    let edges = IntraSmoothEdges::new(&left, &above);
    let mut out = vec![T::default(); side * side];
    predict_intra_smooth_rect_into(bit_depth, block_size, smooth_mode, edges, &mut out, side)?;
    Ok(out)
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn reconstruct_general_intra_luma_directional_first_block_into<T: ReconSample>(
    workspace: &mut CurrentFrameWorkspace<T>,
    block: &LumaCoeffBlock,
    mode: SupportedDirectionalLumaMode,
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
        |block_size, side, bit_depth| {
            predict_directional_noneighbour(mode, block_size, side, bit_depth)
        },
    )
}

pub(crate) fn predict_directional_noneighbour<T: ReconSample>(
    mode: SupportedDirectionalLumaMode,
    block_size: IntraRectBlockSize,
    side: usize,
    bit_depth: BitDepth,
) -> core::result::Result<Vec<T>, GeneralIntraResidualError> {
    let angle = middle_directional_angle(mode)?;
    let mut above = vec![noneighbour_above::<T>(bit_depth); side + 1];
    let mut left = vec![noneighbour_left::<T>(bit_depth); side + 1];
    above[0] = noneighbour_corner::<T>(bit_depth);
    left[0] = noneighbour_corner::<T>(bit_depth);
    let edges = IntraMiddleDirectionalAngleEdges::both(&left, &above);
    let mut out = vec![T::default(); side * side];
    predict_intra_middle_directional_angle_rect_into(
        bit_depth, block_size, angle, edges, &mut out, side,
    )?;
    Ok(out)
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
) -> core::result::Result<(Vec<T>, Vec<T>, T), GeneralIntraResidualError> {
    let above_len = width + (width >> 2);
    let left_len = height + (height >> 2);
    let above = if availability.above {
        collect_available_dip_above_edge(workspace, x, y, width, above_len, num4_above_right)?
    } else if availability.left {
        vec![left_seed(workspace, x, y)?; above_len]
    } else {
        vec![noneighbour_above::<T>(bit_depth); above_len]
    };
    let left = if availability.left {
        collect_available_dip_left_edge(workspace, x, y, height, left_len, num4_below_left)?
    } else if availability.above {
        vec![above_seed(workspace, x, y)?; left_len]
    } else {
        vec![noneighbour_left::<T>(bit_depth); left_len]
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

fn collect_available_dip_above_edge<T: ReconSample>(
    workspace: &CurrentFrameWorkspace<T>,
    x: usize,
    y: usize,
    width: usize,
    edge_len: usize,
    num4_above_right: usize,
) -> core::result::Result<Vec<T>, GeneralIntraResidualError> {
    let sample_y = y
        .checked_sub(1)
        .ok_or(GeneralIntraResidualError::UnsupportedDirectionalAboveEdge)?;
    let storage_width = workspace.plane(PlaneId::Y)?.storage_size().width();
    let readable = width
        .saturating_add(num4_above_right.saturating_mul(MI_SIZE))
        .min(edge_len);
    let mut edge = Vec::with_capacity(edge_len);
    for offset in 0..readable {
        let Some(sample_x) = x.checked_add(offset) else {
            break;
        };
        if sample_x >= storage_width {
            break;
        }
        edge.push(workspace.reconstructed_sample(PlaneId::Y, sample_x, sample_y)?);
    }
    extend_edge_with_last(edge, edge_len)
}

fn collect_available_dip_left_edge<T: ReconSample>(
    workspace: &CurrentFrameWorkspace<T>,
    x: usize,
    y: usize,
    height: usize,
    edge_len: usize,
    num4_below_left: usize,
) -> core::result::Result<Vec<T>, GeneralIntraResidualError> {
    let sample_x = x
        .checked_sub(1)
        .ok_or(GeneralIntraResidualError::UnsupportedDirectionalAboveEdge)?;
    let storage_height = workspace.plane(PlaneId::Y)?.storage_size().height();
    let readable = height
        .saturating_add(num4_below_left.saturating_mul(MI_SIZE))
        .min(edge_len);
    let mut edge = Vec::with_capacity(edge_len);
    for offset in 0..readable {
        let Some(sample_y) = y.checked_add(offset) else {
            break;
        };
        if sample_y >= storage_height {
            break;
        }
        edge.push(workspace.reconstructed_sample(PlaneId::Y, sample_x, sample_y)?);
    }
    extend_edge_with_last(edge, edge_len)
}

fn extend_edge_with_last<T: ReconSample>(
    mut edge: Vec<T>,
    edge_len: usize,
) -> core::result::Result<Vec<T>, GeneralIntraResidualError> {
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
    let mut prediction = vec![T::default(); side * side];
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
    let out = if block.all_zero {
        prediction
    } else if let Some(luma_context) = luma_context {
        reconstruct_general_intra_luma_block_rect_with_prediction_and_ist(
            block,
            &prediction,
            qindex,
            log2_side,
            log2_side,
            use_tcq,
            bit_depth,
            luma_context,
        )?
    } else {
        reconstruct_general_intra_coeff_block_with_prediction(
            block,
            &prediction,
            qindex,
            plane_id,
            log2_side,
            use_tcq,
            bit_depth,
        )?
    };
    workspace.write_rect_block(plane_id, x, y, block_size, &out)?;
    Ok(())
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
    let mut prediction = vec![T::default(); width * height];

    if left_samples.is_some() && above_samples.is_some() {
        let above_row = y
            .checked_sub(1)
            .ok_or(GeneralIntraResidualError::UnsupportedDirectionalAboveEdge)?;
        let left_col = x
            .checked_sub(1)
            .ok_or(GeneralIntraResidualError::UnsupportedDirectionalAboveEdge)?;
        let corner = workspace.reconstructed_sample(plane_id, left_col, above_row)?;
        let above_idif = build_two_sided_middle_idif_edge(width, filters.above, corner, |i| {
            workspace.reconstructed_sample(plane_id, x.saturating_add(i), above_row)
        })?;
        let left_idif = build_two_sided_middle_idif_edge(height, filters.left, corner, |i| {
            workspace.reconstructed_sample(plane_id, left_col, y.saturating_add(i))
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
fn paeth_reference_edges<T: ReconSample>(
    workspace: &CurrentFrameWorkspace<T>,
    plane_id: PlaneId,
    x: usize,
    y: usize,
    width: usize,
    height: usize,
    bit_depth: BitDepth,
    above_in: Option<&[T]>,
    left_in: Option<&[T]>,
) -> core::result::Result<(Vec<T>, Vec<T>, T), GeneralIntraResidualError> {
    let above = match (above_in, left_in) {
        (Some(above), _) => above.iter().take(width).copied().collect::<Vec<T>>(),
        (None, Some(left)) => {
            vec![*left.first().unwrap_or(&noneighbour_above::<T>(bit_depth)); width]
        }
        (None, None) => vec![noneighbour_above::<T>(bit_depth); width],
    };
    let left = match (left_in, above_in) {
        (Some(left), _) => left.iter().take(height).copied().collect::<Vec<T>>(),
        (None, Some(above)) => {
            vec![*above.first().unwrap_or(&noneighbour_left::<T>(bit_depth)); height]
        }
        (None, None) => vec![noneighbour_left::<T>(bit_depth); height],
    };
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
    Ok((above, left, top_left))
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn reconstruct_general_intra_luma_paeth_neighbour_block_into<T: ReconSample>(
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
    let (above, left, top_left) = paeth_reference_edges(
        workspace,
        plane_id,
        x,
        y,
        width,
        height,
        bit_depth,
        above_samples,
        left_samples,
    )?;
    let mut prediction = vec![T::default(); width * height];
    predict_intra_paeth_rect_into(
        bit_depth,
        block_size,
        IntraPaethEdges::new(&left, &above, top_left),
        &mut prediction,
        width,
    )?;
    let out = if block.all_zero {
        prediction
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

fn average_luma_prediction_with<T: ReconSample>(
    prediction: &mut [T],
    secondary: Vec<T>,
) -> core::result::Result<(), GeneralIntraResidualError> {
    for (primary, secondary) in prediction.iter_mut().zip(secondary) {
        let average = (u32::from(primary.to_u16()) + u32::from(secondary.to_u16()) + 1) >> 1;
        let average = u16::try_from(average)
            .map_err(|_| GeneralIntraResidualError::UnsupportedDirectionalAboveEdge)?;
        *primary = T::try_from_u16(average)?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn write_luma_prediction_block<T: ReconSample>(
    workspace: &mut CurrentFrameWorkspace<T>,
    block: &LumaCoeffBlock,
    prediction: Vec<T>,
    x: usize,
    y: usize,
    log2_width: u32,
    log2_height: u32,
    qindex: u32,
    use_tcq: bool,
    bit_depth: BitDepth,
    luma_context: Option<LumaTransformTypeContext>,
) -> core::result::Result<(), GeneralIntraResidualError> {
    let log2_w = u8::try_from(log2_width).unwrap_or(u8::MAX);
    let log2_h = u8::try_from(log2_height).unwrap_or(u8::MAX);
    let block_size = IntraRectBlockSize::new(log2_w, log2_h)?;
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
    )?;
    if secondary_mrl {
        let secondary =
            cardinal_mrl_luma_prediction(workspace, direction, x, y, width, height, 0, 0)?;
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
) -> core::result::Result<Vec<T>, GeneralIntraResidualError> {
    let mut prediction = vec![T::default(); width * height];
    match direction {
        IntraCardinalDirection::Vertical => {
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
        IntraCardinalDirection::Horizontal => {
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
    }
    Ok(prediction)
}

#[allow(clippy::too_many_arguments)]
fn build_one_sided_above_idif_edge<T: ReconSample>(
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
) -> core::result::Result<Vec<T>, GeneralIntraResidualError> {
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
    let corner_col = x.checked_sub(1).unwrap_or(x);
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
) -> core::result::Result<Vec<T>, GeneralIntraResidualError> {
    let max_base = width
        .checked_add(height)
        .and_then(|v| v.checked_sub(1))
        .and_then(|v| v.checked_add(mrl_index << 1))
        .ok_or(GeneralIntraResidualError::UnsupportedDirectionalAboveEdge)?;
    let edge_len = max_base
        .checked_add(5)
        .ok_or(GeneralIntraResidualError::UnsupportedDirectionalAboveEdge)?;
    let mut edge = vec![T::default(); edge_len];
    edge[1] = corner()?;
    for i in 0..=max_base {
        let slot = i
            .checked_add(2)
            .ok_or(GeneralIntraResidualError::UnsupportedDirectionalAboveEdge)?;
        edge[slot] = in_edge(i)?;
    }
    finalize_one_sided_idif_edge(&mut edge, max_base, edge_filter)?;
    Ok(edge)
}

fn finalize_one_sided_idif_edge<T: ReconSample>(
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
fn build_one_sided_left_idif_edge<T: ReconSample>(
    workspace: &CurrentFrameWorkspace<T>,
    plane_id: PlaneId,
    x: usize,
    y: usize,
    width: usize,
    height: usize,
    num4_below_left: usize,
    have_above: bool,
    mrl_index: usize,
    have_left: bool,
    bit_depth: BitDepth,
    edge_filter: OneSidedEdgeFilter,
) -> core::result::Result<Vec<T>, GeneralIntraResidualError> {
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
        if mrl_index != 0 {
            return Err(GeneralIntraResidualError::UnsupportedDirectionalAboveEdge);
        }
        let fallback_row = y
            .checked_sub(1)
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
    let log2_w = u8::try_from(log2_width).unwrap_or(u8::MAX);
    let log2_h = u8::try_from(log2_height).unwrap_or(u8::MAX);
    let block_size = IntraRectBlockSize::new(log2_w, log2_h)?;
    let mut prediction = predict_two_sided_middle_luma_mrl(
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
    )?;
    if secondary_mrl {
        let secondary = predict_two_sided_middle_luma_mrl(
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
        )?;
        average_luma_prediction_with(&mut prediction, secondary)?;
    }
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
fn predict_two_sided_middle_luma_mrl<T: ReconSample>(
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
) -> core::result::Result<Vec<T>, GeneralIntraResidualError> {
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
        (false, false) => return Err(GeneralIntraResidualError::UnsupportedDirectionalAboveEdge),
    };
    let mut prediction = vec![T::default(); width * height];
    predict_intra_middle_directional_angle_rect_idif_mrl_into(
        bit_depth,
        block_size,
        angle,
        IntraMiddleDirectionalAngleIdifMrlEdges::both(&left_idif, &above_idif),
        mrl_index,
        &mut prediction,
        width,
    )?;
    Ok(prediction)
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
    let max_logical = i64::try_from(height)
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
    let max_logical = i64::try_from(height)
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
    let max_logical = i64::try_from(width)
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
    let max_logical = i64::try_from(height)
        .map_err(|_| GeneralIntraResidualError::UnsupportedDirectionalAboveEdge)?
        + 1;
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
        };
        Ok(workspace.reconstructed_sample(PlaneId::Y, left_col, row)?)
    })
}

fn build_two_sided_middle_mrl_idif_edge<T: ReconSample>(
    side: usize,
    mrl_index: usize,
    max_logical: i64,
    sample: impl Fn(i64) -> core::result::Result<T, GeneralIntraResidualError>,
) -> core::result::Result<Vec<T>, GeneralIntraResidualError> {
    let len = side
        .checked_add(mrl_index)
        .and_then(|v| v.checked_add(4))
        .ok_or(GeneralIntraResidualError::UnsupportedDirectionalAboveEdge)?;
    let mrl = i64::try_from(mrl_index)
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
    let mut prediction = vec![T::default(); width * height];
    predict_intra_cardinal_directional_rect_into(
        bit_depth,
        block_size,
        direction,
        cardinal_edges,
        &mut prediction,
        width,
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
            dpcm,
            bit_depth,
        )?
    };
    workspace.write_rect_block(plane_id, x, y, block_size, &out)?;
    Ok(())
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

fn middle_directional_angle(
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

#[cfg(test)]
#[path = "reconstruct_tests.rs"]
mod tests;
