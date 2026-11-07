// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Smooth intra prediction entries, edge assembly, and sentinel resolution.

use splot_recon::{
    BitDepth, CurrentFrameWorkspace, IntraBlockFrame, IntraPredictionScratchBuffer,
    IntraRectBlockSize, IntraSmoothEdges, IntraSmoothLanes, IntraSmoothMode, PlaneId, ReconSample,
    predict_intra_smooth_lanes_into, predict_intra_smooth_rect_into,
};

use super::sink::{
    IntraEdgeAvailability, noneighbour_above, noneighbour_left, predict_intra_in_frame,
    write_intra_prediction_block,
};
use crate::bitstream::tile_payload::{
    CoeffBlock, GeneralIntraResidualError, LumaTransformTypeContext, SupportedNonDcLumaMode,
};

const SMOOTH_EDGE_CAPACITY: usize = 65;

#[allow(clippy::too_many_arguments)]
pub(crate) fn reconstruct_general_intra_luma_smooth_rect_block_with_availability_into<
    T: ReconSample,
>(
    workspace: &mut CurrentFrameWorkspace<T>,
    block: CoeffBlock<'_>,
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
    bit_depth: BitDepth,
) -> core::result::Result<(), GeneralIntraResidualError> {
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
    if let Some(edges) = in_frame_smooth_edges(request)
        && bit_depth == workspace.info().bit_depth()
    {
        let mut lanes = IntraSmoothLanes::new(block_size);
        let write_through = block.eob == 0;
        let gathered = workspace.with_intra_block_frame(plane_id, x, y, block_size, |frame| {
            gather_smooth_lanes(frame, request, edges, &mut lanes);
            if write_through {
                let (output, stride) = frame.target();
                predict_intra_smooth_lanes_into(bit_depth, smooth_mode, &lanes, output, stride)?;
            }
            Ok::<_, GeneralIntraResidualError>(())
        })?;
        if gathered.is_some() {
            if write_through {
                return Ok(());
            }
            let width = block_size.width();
            let mut prediction = workspace.take_intra_prediction_buffer(
                IntraPredictionScratchBuffer::Primary,
                plane_id,
                width * block_size.height(),
                T::default(),
            )?;
            predict_intra_smooth_lanes_into(
                bit_depth,
                smooth_mode,
                &lanes,
                &mut prediction,
                width,
            )?;
            return write_intra_prediction_block(
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
            );
        }
    }
    let edges = smooth_prediction_edges(workspace, request)?;
    if predict_intra_in_frame(
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
        |output, stride| predict_smooth_from_edges(request, &edges, output, stride),
    )? {
        return Ok(());
    }
    let mut prediction = workspace.take_intra_prediction_buffer(
        IntraPredictionScratchBuffer::Primary,
        plane_id,
        block_size.width() * block_size.height(),
        T::default(),
    )?;
    predict_smooth_from_edges(request, &edges, &mut prediction, block_size.width())?;
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

/// Which edges a block reads from the frame, when its sample limits are all
/// or nothing.
fn in_frame_smooth_edges(request: SmoothIntraPredictionRequest) -> Option<IntraEdgeAvailability> {
    let wanted = |limit: Option<usize>| match limit {
        None => Some(true),
        Some(0) => Some(false),
        Some(_) => None,
    };
    Some(IntraEdgeAvailability::new(
        wanted(request.available_above_samples)?,
        wanted(request.available_left_samples)?,
    ))
}

/// Gathers the § 7.13.2.13 edges with their sentinels straight from the
/// frame, the in-storage counterpart of [`smooth_prediction_edges`].
fn gather_smooth_lanes<T: ReconSample>(
    frame: &IntraBlockFrame<'_, T>,
    request: SmoothIntraPredictionRequest,
    wanted: IntraEdgeAvailability,
    lanes: &mut IntraSmoothLanes<T>,
) {
    let (width, height) = (request.block_size.width(), request.block_size.height());
    let left_len = if wanted.left {
        frame.left_column(&mut lanes.left[..=height])
    } else {
        0
    };
    let above = if wanted.above { frame.above_row() } else { &[] };
    let have_left = left_len >= height;
    let have_above = above.len() >= width;
    if have_left && (left_len == height || request.num4_below_left == 0) {
        lanes.left[height] = lanes.left[height - 1];
    }
    let mut corners = [T::default(); 2];
    if have_above {
        lanes.set_above(above);
        corners[0] = if request.num4_above_right > 0 && above.len() > width {
            above[width]
        } else {
            above[width - 1]
        };
    }
    if !have_left {
        let fill = if have_above {
            above[0]
        } else {
            noneighbour_left::<T>(request.bit_depth)
        };
        lanes.left[..=height].fill(fill);
    }
    corners[1] = lanes.left[height];
    if !have_above {
        let fill = if have_left {
            lanes.left[0]
        } else {
            noneighbour_above::<T>(request.bit_depth)
        };
        lanes.fill_above(fill);
        corners[0] = fill;
    }
    lanes.corners = corners;
}

pub(crate) fn predict_intra_smooth_over_available_edges_into<T: ReconSample>(
    workspace: &CurrentFrameWorkspace<T>,
    request: SmoothIntraPredictionRequest,
    prediction: &mut [T],
) -> core::result::Result<(), GeneralIntraResidualError> {
    let edges = smooth_prediction_edges(workspace, request)?;
    predict_smooth_from_edges(request, &edges, prediction, request.block_size.width())
}

/// The `[left, above]` § 7.13.2.13 edges, sentinels included.
type SmoothEdgeArrays<T> = [[T; SMOOTH_EDGE_CAPACITY]; 2];

fn predict_smooth_from_edges<T: ReconSample>(
    request: SmoothIntraPredictionRequest,
    [left_edge, above_edge]: &SmoothEdgeArrays<T>,
    output: &mut [T],
    stride: usize,
) -> core::result::Result<(), GeneralIntraResidualError> {
    let size = request.block_size;
    let smooth_edges =
        IntraSmoothEdges::new(&left_edge[..=size.height()], &above_edge[..=size.width()]);
    predict_intra_smooth_rect_into(
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
    [left, above]: [&mut [T; SMOOTH_EDGE_CAPACITY]; 2],
) {
    let left_len = (height + 1).min(SMOOTH_EDGE_CAPACITY);
    let above_len = (width + 1).min(SMOOTH_EDGE_CAPACITY);
    match (have_left, left_neighbour) {
        (true, Some(samples)) => {
            fill_edge_from_neighbour(samples, &mut left[..left_len], bit_depth);
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
            fill_edge_from_neighbour(samples, &mut above[..above_len], bit_depth);
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

fn fill_edge_from_neighbour<T: ReconSample>(samples: &[T], edge: &mut [T], bit_depth: BitDepth) {
    let copied = samples.len().min(edge.len());
    let (head, tail) = edge.split_at_mut(copied);
    head.copy_from_slice(&samples[..copied]); // splot-copy-ok: smooth edge from its neighbour
    tail.fill(
        samples
            .last()
            .copied()
            .unwrap_or(noneighbour_left::<T>(bit_depth)),
    );
}
