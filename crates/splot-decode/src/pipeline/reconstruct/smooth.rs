// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Smooth intra prediction entries, edge assembly, and sentinel resolution.

use splot_core::simd::SimdBackend;
use splot_recon::{
    BitDepth, CurrentFrameWorkspace, IntraRectBlockSize, IntraSmoothEdges, IntraSmoothFrameRequest,
    IntraSmoothMode, PlaneId, ReconSample, predict_intra_smooth_rect_into_with,
};

use super::sink::{
    IntraEdgeAvailability, noneighbour_above, noneighbour_left, predict_intra_block,
};
use crate::bitstream::tile_payload::{
    CoeffBlock, GeneralIntraResidualError, LumaTransformTypeContext,
    add_general_intra_coeff_block_residual_in_place_with,
};

const SMOOTH_EDGE_CAPACITY: usize = 65;

#[expect(clippy::too_many_arguments)]
pub(crate) fn reconstruct_general_intra_luma_smooth_rect_block_into<T: ReconSample>(
    backend: SimdBackend,
    workspace: &mut CurrentFrameWorkspace<T>,
    block: CoeffBlock<'_>,
    mode: IntraSmoothMode,
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
) -> core::result::Result<(), GeneralIntraResidualError> {
    let bit_depth = workspace.info().bit_depth();
    let log2_w = u8::try_from(log2_width).unwrap_or(u8::MAX);
    let log2_h = u8::try_from(log2_height).unwrap_or(u8::MAX);
    let request = IntraSmoothFrameRequest {
        size: IntraRectBlockSize::new(log2_w, log2_h)?,
        mode,
        above: availability.above,
        left: availability.left,
        above_right: num4_above_right > 0,
        below_left: num4_below_left > 0,
        bit_depth,
    };
    if reconstruct_smooth_in_frame(
        backend,
        workspace,
        block,
        PlaneId::Y,
        x,
        y,
        request,
        qindex,
        use_tcq,
        luma_context,
    )? {
        return Ok(());
    }
    let (available_left_samples, available_above_samples) = availability.available_sample_limits();
    reconstruct_general_intra_smooth_over_available_edges_into(
        backend,
        workspace,
        block,
        PlaneId::Y,
        x,
        y,
        log2_width,
        log2_height,
        qindex,
        mode,
        available_left_samples,
        available_above_samples,
        num4_above_right,
        num4_below_left,
        use_tcq,
        luma_context,
    )
}

/// Reconstructs a smooth block whose edges the frame stores straight into
/// the frame; `false` when [`CurrentFrameWorkspace::predict_intra_smooth_in_frame_with`]
/// declines it.
#[expect(clippy::too_many_arguments)]
#[inline]
pub(crate) fn reconstruct_smooth_in_frame<T: ReconSample>(
    backend: SimdBackend,
    workspace: &mut CurrentFrameWorkspace<T>,
    block: CoeffBlock<'_>,
    plane_id: PlaneId,
    x: usize,
    y: usize,
    request: IntraSmoothFrameRequest,
    qindex: u32,
    use_tcq: bool,
    luma_context: Option<LumaTransformTypeContext>,
) -> core::result::Result<bool, GeneralIntraResidualError> {
    if !workspace.predict_intra_smooth_in_frame_with(backend, plane_id, x, y, request)? {
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
            request.size,
            qindex,
            use_tcq,
            luma_context,
            None,
            request.bit_depth,
        )?;
    }
    Ok(true)
}

#[expect(clippy::too_many_arguments)]
pub(crate) fn reconstruct_general_intra_smooth_over_available_edges_into<T: ReconSample>(
    backend: SimdBackend,
    workspace: &mut CurrentFrameWorkspace<T>,
    block: CoeffBlock<'_>,
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
) -> core::result::Result<(), GeneralIntraResidualError> {
    let bit_depth = workspace.info().bit_depth();
    let log2_w = u8::try_from(log2_width).unwrap_or(u8::MAX);
    let log2_h = u8::try_from(log2_height).unwrap_or(u8::MAX);
    let block_size = IntraRectBlockSize::new(log2_w, log2_h)?;
    let request = SmoothIntraPredictionRequest {
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
    };
    let edges = smooth_prediction_edges(workspace, request)?;
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
        |output, stride| predict_smooth_from_edges(backend, request, &edges, output, stride),
    )
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

pub(crate) fn predict_intra_smooth_over_available_edges_into<T: ReconSample>(
    backend: SimdBackend,
    workspace: &CurrentFrameWorkspace<T>,
    request: SmoothIntraPredictionRequest,
    prediction: &mut [T],
) -> core::result::Result<(), GeneralIntraResidualError> {
    let edges = smooth_prediction_edges(workspace, request)?;
    let size = request.block_size;
    let smooth_edges =
        IntraSmoothEdges::new(&edges[0][..=size.height()], &edges[1][..=size.width()]);
    predict_intra_smooth_rect_into_with(
        backend,
        request.bit_depth,
        size,
        request.mode,
        smooth_edges,
        prediction,
        size.width(),
    )?;
    Ok(())
}

/// The `[left, above]` § 7.13.2.13 edges, sentinels included.
type SmoothEdgeArrays<T> = [[T; SMOOTH_EDGE_CAPACITY]; 2];

fn predict_smooth_from_edges<T: ReconSample>(
    backend: SimdBackend,
    request: SmoothIntraPredictionRequest,
    [left_edge, above_edge]: &SmoothEdgeArrays<T>,
    output: &mut [T],
    stride: usize,
) -> core::result::Result<(), GeneralIntraResidualError> {
    let size = request.block_size;
    let smooth_edges =
        IntraSmoothEdges::new(&left_edge[..=size.height()], &above_edge[..=size.width()]);
    predict_intra_smooth_rect_into_with(
        backend,
        request.bit_depth,
        size,
        request.mode,
        smooth_edges,
        output,
        stride,
    )?;
    Ok(())
}

fn smooth_prediction_edges<T: ReconSample>(
    workspace: &CurrentFrameWorkspace<T>,
    request: SmoothIntraPredictionRequest,
) -> core::result::Result<SmoothEdgeArrays<T>, GeneralIntraResidualError> {
    let SmoothIntraPredictionRequest {
        plane_id,
        x,
        y,
        block_size,
        mode: _,
        available_left_samples,
        available_above_samples,
        num4_above_right,
        num4_below_left,
        bit_depth,
    } = request;
    let width = block_size.width();
    let height = block_size.height();
    let mut edges = splot_recon::CurrentFrameIntraEdges::default();
    workspace.intra_dc_edges_for_rect_into(plane_id, x, y, block_size, &mut edges)?;
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
        resolve_smooth_sentinel(
            workspace,
            SmoothSentinelRequest {
                plane_id,
                x,
                y,
                side: width,
                have_edge: have_above,
                extension: num4_above_right,
                kind: SmoothSentinelKind::AboveRight,
            },
        )?
    } else {
        None
    };
    let bottom_left_sentinel = if left_len >= height {
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
        )?
    } else {
        None
    };
    let mut left_edge = [T::default(); SMOOTH_EDGE_CAPACITY];
    let mut above_edge = [T::default(); SMOOTH_EDGE_CAPACITY];
    build_smooth_edges(
        left,
        above,
        have_left,
        have_above,
        width,
        height,
        above_right_sentinel,
        bottom_left_sentinel,
        bit_depth,
        [&mut left_edge, &mut above_edge],
    );
    Ok([left_edge, above_edge])
}

/// Fills the `[left, above]` edges in place, so that no edge array moves
/// after its sample stores.
#[expect(clippy::too_many_arguments)]
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
    [left, above]: [&mut [T; SMOOTH_EDGE_CAPACITY]; 2],
) {
    let left_len = (height + 1).min(SMOOTH_EDGE_CAPACITY);
    let above_len = (width + 1).min(SMOOTH_EDGE_CAPACITY);
    match (have_left, left_neighbour) {
        (true, Some(samples)) => {
            fill_edge_from_neighbour(
                samples,
                &mut left[..left_len],
                noneighbour_left::<T>(bit_depth),
            );
        }
        _ if have_above => left.fill(
            above_neighbour
                .and_then(|samples| samples.first().copied())
                .unwrap_or(noneighbour_left::<T>(bit_depth)),
        ),
        _ => left.fill(noneighbour_left::<T>(bit_depth)),
    }
    match (have_above, above_neighbour) {
        (true, Some(samples)) => {
            fill_edge_from_neighbour(
                samples,
                &mut above[..above_len],
                noneighbour_left::<T>(bit_depth),
            );
        }
        _ if have_left => above.fill(
            left_neighbour
                .and_then(|samples| samples.first().copied())
                .unwrap_or(noneighbour_above::<T>(bit_depth)),
        ),
        _ => above.fill(noneighbour_above::<T>(bit_depth)),
    }
    if let Some(sentinel) = above_right_sentinel {
        above[width] = sentinel;
    }
    if let Some(sentinel) = bottom_left_sentinel {
        left[height] = sentinel;
    }
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

pub(super) fn fill_edge_from_neighbour<T: ReconSample>(samples: &[T], edge: &mut [T], fallback: T) {
    let copied = samples.len().min(edge.len());
    let (head, tail) = edge.split_at_mut(copied);
    head.copy_from_slice(&samples[..copied]); // splot-copy-ok: intra edge from its neighbour
    tail.fill(samples.last().copied().unwrap_or(fallback));
}
