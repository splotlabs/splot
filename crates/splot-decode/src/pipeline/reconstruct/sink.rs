// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Block reconstruction sinks: DC, palette, and inter-residual entries plus the shared luma write tail.

use splot_recon::{
    BitDepth, CurrentFrameWorkspace, DecodedFrameInfo, DpcmDirection, IntraDcEdges,
    IntraPredictionScratchBuffer, IntraRectBlockSize, OutputIndex, PixelFormat, PlaneId, PlaneRect,
    PlaneSize, ReconSample, apply_intra_ibp_dc_rect, predict_intra_dc_rect_value,
};

use crate::Result;
use crate::bitstream::tile_payload::{
    GeneralIntraResidualError, LumaCoeffBlock, LumaPalette, LumaTransformTypeContext,
    reconstruct_general_intra_coeff_block_rect_with_prediction,
    reconstruct_general_intra_coeff_block_rect_with_prediction_and_ddt,
    reconstruct_general_intra_luma_block_rect_with_prediction_and_ist,
};

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

#[cfg(test)]
pub(crate) fn new_general_intra_workspace<T: ReconSample>(
    luma_width: usize,
    luma_height: usize,
    bit_depth: BitDepth,
    pixel_format: PixelFormat,
) -> Result<CurrentFrameWorkspace<T>> {
    let luma_rect = PlaneRect::new(0, 0, luma_width, luma_height)?;
    new_general_intra_workspace_with_visible_rect(
        luma_width,
        luma_height,
        bit_depth,
        pixel_format,
        luma_rect,
    )
}

pub(crate) fn new_general_intra_workspace_with_visible_rect<T: ReconSample>(
    luma_width: usize,
    luma_height: usize,
    bit_depth: BitDepth,
    pixel_format: PixelFormat,
    visible_luma_rect: PlaneRect,
) -> Result<CurrentFrameWorkspace<T>> {
    let luma_size = PlaneSize::new(luma_width, luma_height)?;
    let info = DecodedFrameInfo::new(
        OutputIndex::new(0),
        bit_depth,
        pixel_format,
        luma_size,
        visible_luma_rect,
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
    let mut prediction = workspace.take_intra_prediction_buffer(
        IntraPredictionScratchBuffer::Primary,
        plane_id,
        width * height,
        dc,
    )?;
    if ibp_dc {
        apply_intra_ibp_dc_rect(bit_depth, block_size, dc_edges, &mut prediction, width)?;
    }
    write_intra_prediction_block(
        workspace,
        block,
        prediction,
        plane_id,
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
    sink: &mut crate::prediction::inter::mc::WorkspaceSink<'_, T>,
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
    let storage = sink.plane_storage_size(plane_id)?;
    let in_frame_width = width.min(storage.width().saturating_sub(x));
    let in_frame_height = height.min(storage.height().saturating_sub(y));
    let rect = PlaneRect::new(x, y, in_frame_width, in_frame_height)?;
    let mut prediction = vec![T::default(); width * height];
    for (row_index, row) in sink.rect_rows(plane_id, rect)?.enumerate() {
        let start = row_index * width;
        prediction[start..start + in_frame_width].copy_from_slice(row);
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
    sink.write_rect_block(plane_id, x, y, block_size, &out)?;
    Ok(())
}

pub(super) fn noneighbour_above<T: ReconSample>(bit_depth: BitDepth) -> T {
    let half = 1u16 << (bit_depth.bits() - 1);
    noneighbour_sample::<T>(half - 1)
}

pub(crate) fn noneighbour_left<T: ReconSample>(bit_depth: BitDepth) -> T {
    let half = 1u16 << (bit_depth.bits() - 1);
    noneighbour_sample::<T>(half + 1)
}

pub(super) fn noneighbour_corner<T: ReconSample>(bit_depth: BitDepth) -> T {
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

pub(super) fn average_luma_prediction_with<T: ReconSample>(
    prediction: &mut [T],
    secondary: &[T],
) -> core::result::Result<(), GeneralIntraResidualError> {
    for (primary, secondary) in prediction.iter_mut().zip(secondary) {
        let average = (u32::from(primary.to_u16()) + u32::from(secondary.to_u16()) + 1) >> 1;
        let average = u16::try_from(average)
            .map_err(|_| GeneralIntraResidualError::UnsupportedDirectionalAboveEdge)?;
        *primary = T::try_from_u16(average)?;
    }
    Ok(())
}

pub(super) fn build_mrl_luma_prediction<T: ReconSample>(
    workspace: &mut CurrentFrameWorkspace<T>,
    block_size: IntraRectBlockSize,
    blend_secondary: bool,
    mut predict: impl FnMut(
        &CurrentFrameWorkspace<T>,
        bool,
        &mut [T],
    ) -> core::result::Result<(), GeneralIntraResidualError>,
) -> core::result::Result<Vec<T>, GeneralIntraResidualError> {
    let mut prediction = workspace.take_intra_prediction_buffer(
        IntraPredictionScratchBuffer::Primary,
        PlaneId::Y,
        block_size.sample_count(),
        T::default(),
    )?;
    predict(workspace, false, &mut prediction)?;
    if blend_secondary {
        let mut secondary = workspace.take_intra_prediction_buffer(
            IntraPredictionScratchBuffer::Secondary,
            PlaneId::Y,
            block_size.sample_count(),
            T::default(),
        )?;
        let result = predict(workspace, true, &mut secondary)
            .and_then(|()| average_luma_prediction_with(&mut prediction, &secondary));
        workspace
            .recycle_intra_prediction_buffer(IntraPredictionScratchBuffer::Secondary, secondary);
        result?;
    }
    Ok(prediction)
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn write_intra_prediction_block<T: ReconSample>(
    workspace: &mut CurrentFrameWorkspace<T>,
    block: &LumaCoeffBlock,
    prediction: Vec<T>,
    plane_id: PlaneId,
    x: usize,
    y: usize,
    block_size: IntraRectBlockSize,
    qindex: u32,
    use_tcq: bool,
    luma_context: Option<LumaTransformTypeContext>,
    dpcm: Option<DpcmDirection>,
    bit_depth: BitDepth,
) -> core::result::Result<(), GeneralIntraResidualError> {
    let log2_width = u32::from(block_size.log2_width());
    let log2_height = u32::from(block_size.log2_height());
    if block.all_zero {
        let result = workspace.write_rect_block(plane_id, x, y, block_size, &prediction);
        workspace
            .recycle_intra_prediction_buffer(IntraPredictionScratchBuffer::Primary, prediction);
        return result.map_err(Into::into);
    }
    let reconstructed = if let Some(luma_context) = luma_context {
        reconstruct_general_intra_luma_block_rect_with_prediction_and_ist(
            block,
            &prediction,
            qindex,
            log2_width,
            log2_height,
            use_tcq,
            bit_depth,
            luma_context,
        )
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
        )
    };
    workspace.recycle_intra_prediction_buffer(IntraPredictionScratchBuffer::Primary, prediction);
    let out = reconstructed?;
    workspace.write_rect_block(plane_id, x, y, block_size, &out)?;
    Ok(())
}
