// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Block reconstruction sinks: DC, palette, and inter-residual entries plus the shared luma write tail.

#[cfg(feature = "block-evidence")]
use crate::block_evidence::{DecodeStage, stages};

use splot_core::simd::SimdBackend;
use splot_recon::{
    BitDepth, CurrentFrameWorkspace, DpcmDirection, IntraDcEdges, IntraPredictionScratchBuffer,
    IntraRectBlockSize, PlaneId, ReconSample, predict_intra_dc_rect_value,
    predict_intra_ibp_dc_rect_into_with,
};
#[cfg(test)]
use splot_recon::{
    CurrentFrameSurface, DecodedFrameInfo, OutputIndex, PixelFormat, PlaneRect, PlaneSize,
};

#[cfg(test)]
use crate::Result;
use crate::bitstream::tile_payload::{
    CoeffBlock, GeneralIntraResidualError, LumaPalette, LumaTransformTypeContext,
    add_general_intra_coeff_block_residual_in_place_with,
    reconstruct_general_intra_coeff_block_rect_into_frame_with,
    reconstruct_general_intra_coeff_block_rect_with_prediction_into_with,
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

    pub(crate) const fn read_edge_available(self, above: bool) -> bool {
        if above { self.above } else { self.left }
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
    let luma_size = PlaneSize::new(luma_width, luma_height)?;
    let info = DecodedFrameInfo::new(
        OutputIndex::new(0),
        bit_depth,
        pixel_format,
        luma_size,
        luma_rect,
    )?;
    Ok(CurrentFrameWorkspace::<T>::new(info, T::default())?)
}

#[expect(clippy::too_many_arguments)]
pub(crate) fn reconstruct_general_intra_block_rect_into<B: SimdBackend, T: ReconSample>(
    backend: B,
    workspace: &mut CurrentFrameWorkspace<T>,
    block: CoeffBlock<'_>,
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
) -> core::result::Result<(), GeneralIntraResidualError> {
    let bit_depth = workspace.info().bit_depth();
    let width = 1usize << log2_width;
    let log2_w = u8::try_from(log2_width).unwrap_or(u8::MAX);
    let log2_h = u8::try_from(log2_height).unwrap_or(u8::MAX);
    let block_size = IntraRectBlockSize::new(log2_w, log2_h)?;
    let in_frame = workspace.with_intra_block_frame(plane_id, x, y, block_size, |frame| {
        frame.predict_dc_with(
            backend,
            block_size,
            bit_depth,
            [availability.left, availability.above],
            ibp_dc,
        )
    })?;
    if in_frame.is_some() {
        #[cfg(feature = "block-evidence")]
        if let Some(rect) = stages::block_rect(x, y, block_size) {
            stages::surface_prediction(
                &splot_recon::CurrentFrameSurface::Frame(workspace),
                plane_id,
                rect,
            );
            if block.eob == 0 {
                stages::skip_residual(plane_id, Some(rect));
            }
        }
        if block.eob != 0 {
            add_general_intra_coeff_block_residual_in_place_with(
                backend,
                workspace,
                block,
                plane_id,
                x,
                y,
                block_size,
                qindex,
                use_tcq,
                luma_context,
                None,
                bit_depth,
            )?;
        }
        return Ok(());
    }
    let mut edges = splot_recon::CurrentFrameIntraEdges::default();
    workspace.intra_dc_edges_for_rect_into(plane_id, x, y, block_size, &mut edges)?;
    let dc_edges = IntraDcEdges::new(
        availability.left.then(|| edges.left_samples()).flatten(),
        availability.above.then(|| edges.above_samples()).flatten(),
    );
    let predict_dc = |output: &mut [T], stride: usize| {
        if ibp_dc {
            predict_intra_ibp_dc_rect_into_with(
                backend, bit_depth, block_size, dc_edges, output, stride,
            )?;
        } else {
            let dc = predict_intra_dc_rect_value(bit_depth, block_size, dc_edges)?;
            for row in output.chunks_mut(stride) {
                let filled = width.min(row.len());
                row[..filled].fill(dc);
            }
        }
        Ok(())
    };
    predict_intra_block(
        backend,
        workspace,
        block,
        plane_id,
        x,
        y,
        block_size,
        qindex,
        use_tcq,
        luma_context,
        None,
        bit_depth,
        predict_dc,
    )
}

#[expect(clippy::too_many_arguments)]
pub(crate) fn reconstruct_general_intra_luma_palette_block_into<B: SimdBackend, T: ReconSample>(
    backend: B,
    workspace: &mut CurrentFrameWorkspace<T>,
    block: CoeffBlock<'_>,
    palette: LumaPalette,
    color_map: &[u8],
    x: usize,
    y: usize,
    log2_width: u32,
    log2_height: u32,
    qindex: u32,
    use_tcq: bool,
    luma_context: LumaTransformTypeContext,
) -> core::result::Result<(), GeneralIntraResidualError> {
    let bit_depth = workspace.info().bit_depth();
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
    let mut prediction = workspace.take_intra_prediction_buffer(
        IntraPredictionScratchBuffer::Primary,
        PlaneId::Y,
        block_size.sample_count(),
    )?;
    let fill_result = splot_recon::predict_intra_palette_into_with(
        backend,
        &palette.colors(),
        color_map,
        &mut prediction,
    )
    .map_err(GeneralIntraResidualError::from)
    .and_then(|max_index| {
        if usize::from(max_index) < palette.size() {
            Ok(())
        } else {
            Err(GeneralIntraResidualError::PaletteColorIndex {
                color_index: usize::from(max_index),
                palette_size: palette.size(),
            })
        }
    });
    if let Err(error) = fill_result {
        workspace
            .recycle_intra_prediction_buffer(IntraPredictionScratchBuffer::Primary, prediction);
        return Err(error);
    }
    write_intra_prediction_block(
        backend,
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

#[cfg(test)]
#[expect(clippy::too_many_arguments)]
pub(crate) fn reconstruct_inter_block_residual_rect_into<T: ReconSample>(
    sink: &mut CurrentFrameSurface<'_, '_, T>,
    block: CoeffBlock<'_>,
    plane_id: PlaneId,
    x: usize,
    y: usize,
    log2_width: u32,
    log2_height: u32,
    qindex: u32,
    use_tcq: bool,
    use_ddt: bool,
) -> core::result::Result<(), GeneralIntraResidualError> {
    let bit_depth = sink.info().bit_depth();
    let block_size = IntraRectBlockSize::new(
        u8::try_from(log2_width).unwrap_or(u8::MAX),
        u8::try_from(log2_height).unwrap_or(u8::MAX),
    )?;
    if block.eob == 0 {
        return Ok(());
    }
    crate::bitstream::tile_payload::with_residual_scratch(|scratch| {
        crate::test_support::with_auto_backend!(backend => {
            crate::bitstream::tile_payload::reconstruct_inter_coeff_block_residual_rect_into_with(
                backend, sink, scratch, block, plane_id, x, y, block_size, qindex, use_tcq,
                use_ddt, bit_depth,
            )
        })
    })
}

pub(super) const fn invalid_directional_edge_state() -> GeneralIntraResidualError {
    GeneralIntraResidualError::InvalidDirectionalEdgeState
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

pub(super) fn build_mrl_luma_prediction<B: SimdBackend, T: ReconSample>(
    backend: B,
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
    )?;
    predict(workspace, false, &mut prediction)?;
    if blend_secondary && (block_size.width() != 4 || block_size.height() != 4) {
        let mut secondary = workspace.take_intra_prediction_buffer(
            IntraPredictionScratchBuffer::Secondary,
            PlaneId::Y,
            block_size.sample_count(),
        )?;
        let result = predict(workspace, true, &mut secondary).and_then(|()| {
            splot_recon::average_intra_predictions_with(backend, &mut prediction, &secondary)
                .map_err(Into::into)
        });
        workspace
            .recycle_intra_prediction_buffer(IntraPredictionScratchBuffer::Secondary, secondary);
        result?;
    }
    Ok(prediction)
}

#[expect(clippy::too_many_arguments)]
pub(crate) fn write_intra_prediction_block<B: SimdBackend, T: ReconSample>(
    backend: B,
    workspace: &mut CurrentFrameWorkspace<T>,
    block: CoeffBlock<'_>,
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
    let result = commit_intra_prediction(
        backend,
        workspace,
        block,
        &prediction,
        plane_id,
        x,
        y,
        block_size,
        qindex,
        use_tcq,
        luma_context,
        dpcm,
        bit_depth,
    );
    workspace.recycle_intra_prediction_buffer(IntraPredictionScratchBuffer::Primary, prediction);
    result
}

/// Predicts an in-frame block straight into its frame rows, then adds any
/// coded residual there, so no staging buffer is copied into the frame.
///
/// Returns `false` without predicting when the block overhangs the frame edge.
#[expect(clippy::too_many_arguments)]
pub(crate) fn predict_intra_in_frame<B: SimdBackend, T: ReconSample>(
    backend: B,
    workspace: &mut CurrentFrameWorkspace<T>,
    block: CoeffBlock<'_>,
    plane_id: PlaneId,
    x: usize,
    y: usize,
    block_size: IntraRectBlockSize,
    qindex: u32,
    use_tcq: bool,
    luma_context: Option<LumaTransformTypeContext>,
    dpcm: Option<DpcmDirection>,
    bit_depth: BitDepth,
    predict: impl FnOnce(&mut [T], usize) -> core::result::Result<(), GeneralIntraResidualError>,
) -> core::result::Result<bool, GeneralIntraResidualError> {
    let predicted = workspace.with_rect_block_rows_mut(plane_id, x, y, block_size, |rows| {
        let (samples, stride) = rows.prediction_target();
        #[cfg(not(feature = "block-evidence"))]
        {
            predict(samples, stride)
        }
        #[cfg(feature = "block-evidence")]
        {
            let predicted = predict(samples, stride);
            if predicted.is_ok()
                && let Some(rect) = stages::block_rect(x, y, block_size)
            {
                stages::transform_samples(
                    DecodeStage::Prediction,
                    plane_id,
                    Some(rect),
                    true,
                    |x, y| {
                        samples
                            .get(y * stride + x)
                            .map(|value| i32::from(value.to_u16()))
                    },
                );
                if block.eob == 0 {
                    stages::skip_residual(plane_id, Some(rect));
                }
            }
            predicted
        }
    })?;
    if predicted.is_none() {
        return Ok(false);
    }
    if block.eob != 0 {
        add_general_intra_coeff_block_residual_in_place_with(
            backend,
            workspace,
            block,
            plane_id,
            x,
            y,
            block_size,
            qindex,
            use_tcq,
            luma_context,
            dpcm,
            bit_depth,
        )?;
    }
    Ok(true)
}

/// [`predict_intra_in_frame`], or for a block it declines, `predict` into a
/// staging buffer that [`write_intra_prediction_block`] commits.
#[expect(clippy::too_many_arguments)]
pub(crate) fn predict_intra_block<B: SimdBackend, T: ReconSample>(
    backend: B,
    workspace: &mut CurrentFrameWorkspace<T>,
    block: CoeffBlock<'_>,
    plane_id: PlaneId,
    x: usize,
    y: usize,
    block_size: IntraRectBlockSize,
    qindex: u32,
    use_tcq: bool,
    luma_context: Option<LumaTransformTypeContext>,
    dpcm: Option<DpcmDirection>,
    bit_depth: BitDepth,
    predict: impl Fn(&mut [T], usize) -> core::result::Result<(), GeneralIntraResidualError>,
) -> core::result::Result<(), GeneralIntraResidualError> {
    if predict_intra_in_frame(
        backend,
        workspace,
        block,
        plane_id,
        x,
        y,
        block_size,
        qindex,
        use_tcq,
        luma_context,
        dpcm,
        bit_depth,
        &predict,
    )? {
        return Ok(());
    }
    let mut prediction = workspace.take_intra_prediction_buffer(
        IntraPredictionScratchBuffer::Primary,
        plane_id,
        block_size.sample_count(),
    )?;
    predict(&mut prediction, block_size.width())?;
    write_intra_prediction_block(
        backend,
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
        dpcm,
        bit_depth,
    )
}

/// Commits one intra block's prediction and residual into the current frame.
///
/// A coefficient-free block writes its prediction directly. Otherwise the block
/// reconstructs write-through, straight into the frame rows, and only a block
/// [`reconstruct_general_intra_coeff_block_rect_into_frame_with`] declines falls back
/// to reconstructing through a staging buffer and copying that into the frame.
#[expect(clippy::too_many_arguments)]
pub(crate) fn commit_intra_prediction<B: SimdBackend, T: ReconSample>(
    backend: B,
    workspace: &mut CurrentFrameWorkspace<T>,
    block: CoeffBlock<'_>,
    prediction: &[T],
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
    #[cfg(feature = "block-evidence")]
    if let Some(rect) = stages::block_rect(x, y, block_size) {
        stages::transform_samples(
            DecodeStage::Prediction,
            plane_id,
            Some(rect),
            true,
            |x, y| {
                prediction
                    .get(y * block_size.width() + x)
                    .map(|value| i32::from(value.to_u16()))
            },
        );
        if block.eob == 0 {
            stages::skip_residual(plane_id, Some(rect));
        }
    }
    if block.eob == 0 {
        return workspace
            .write_rect_block_with(backend, plane_id, x, y, block_size, prediction)
            .map_err(Into::into);
    }
    if reconstruct_general_intra_coeff_block_rect_into_frame_with(
        backend,
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
        dpcm,
        bit_depth,
    )? {
        return Ok(());
    }
    let mut out = workspace.take_intra_prediction_buffer(
        IntraPredictionScratchBuffer::Secondary,
        plane_id,
        block_size.sample_count(),
    )?;
    let result = reconstruct_general_intra_coeff_block_rect_with_prediction_into_with(
        backend,
        block,
        prediction,
        &mut out,
        qindex,
        plane_id,
        u32::from(block_size.log2_width()),
        u32::from(block_size.log2_height()),
        use_tcq,
        luma_context,
        dpcm,
        bit_depth,
        #[cfg(feature = "block-evidence")]
        Some((x, y)),
    )
    .and_then(|()| {
        workspace
            .write_rect_block_with(backend, plane_id, x, y, block_size, &out)
            .map_err(Into::into)
    });
    workspace.recycle_intra_prediction_buffer(IntraPredictionScratchBuffer::Secondary, out);
    result
}
