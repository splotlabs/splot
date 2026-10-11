// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! First-block, DIP, and Paeth intra entries with their edge helpers.

use splot_recon::{
    BitDepth, CurrentFrameWorkspace, IntraDipEdges, IntraMiddleDirectionalAngleEdges,
    IntraPaethEdges, IntraPredictionScratchBuffer, IntraRectBlockSize, PlaneId, ReconSample,
    predict_intra_dip_rect_into, predict_intra_middle_directional_angle_rect_into,
    predict_intra_paeth_rect_into,
};

use super::middle::middle_directional_angle;
use super::one_sided::{gather_column_clamped, gather_row_clamped};
use super::sink::{
    IntraEdgeAvailability, invalid_directional_edge_state, noneighbour_above, noneighbour_corner,
    noneighbour_left, write_intra_prediction_block,
};
use crate::bitstream::tile_payload::{
    CoeffBlock, GeneralIntraResidualError, LumaTransformTypeContext, SupportedDirectionalLumaMode,
};
use crate::pipeline::general_intra::RecycledIntraSamples;

const MI_SIZE: usize = 4;

#[allow(clippy::too_many_arguments)]
pub(crate) fn reconstruct_general_intra_luma_dip_rect_block_into<T: ReconSample>(
    workspace: &mut CurrentFrameWorkspace<T>,
    block: CoeffBlock<'_>,
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

pub(crate) fn predict_directional_noneighbour_into<T: ReconSample>(
    mode: SupportedDirectionalLumaMode,
    block_size: IntraRectBlockSize,
    side: usize,
    bit_depth: BitDepth,
    prediction: &mut [T],
) -> core::result::Result<(), GeneralIntraResidualError> {
    let angle = middle_directional_angle(mode);
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
    let (above_row, left_col) = neighbour_origin(availability, x, y)?;
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
    } else if let Some(left_col) = left_col {
        let seed = workspace.reconstructed_sample(PlaneId::Y, left_col, y)?;
        RecycledIntraSamples::filled(above_len, seed)
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
    } else if let Some(above_row) = above_row {
        let seed = workspace.reconstructed_sample(PlaneId::Y, x, above_row)?;
        RecycledIntraSamples::filled(left_len, seed)
    } else {
        RecycledIntraSamples::filled(left_len, noneighbour_left::<T>(bit_depth))
    };
    let top_left = match (above_row, left_col) {
        (Some(above_row), Some(left_col)) => {
            workspace.reconstructed_sample(PlaneId::Y, left_col, above_row)?
        }
        (Some(above_row), None) => workspace.reconstructed_sample(PlaneId::Y, x, above_row)?,
        (None, Some(left_col)) => workspace.reconstructed_sample(PlaneId::Y, left_col, y)?,
        (None, None) => noneighbour_corner::<T>(bit_depth),
    };
    Ok((left, above, top_left))
}

#[derive(Clone, Copy)]
enum DipEdge {
    Above,
    Left,
}

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
    .ok_or_else(invalid_directional_edge_state)?;
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
    let Some(limit) = start
        .checked_add(readable)
        .and_then(|end| end.min(storage_len).checked_sub(1))
        .filter(|&limit| limit >= start)
    else {
        return Err(invalid_directional_edge_state());
    };
    let mut edge = RecycledIntraSamples::filled(edge_len, T::default());
    match direction {
        DipEdge::Above => {
            gather_row_clamped(workspace, PlaneId::Y, (start, fixed), limit, &mut edge)
        }
        DipEdge::Left => {
            gather_column_clamped(workspace, PlaneId::Y, (fixed, start), limit, &mut edge)
        }
    }?;
    Ok(edge)
}

/// Resolves the neighbour sample origins an available edge points at.
///
/// `BlockCtx::neighbours` derives availability as `row4 > tile_mi_row_start` /
/// `col4 > tile_mi_col_start`, so an available edge always sits at a non-zero
/// coordinate and the subtraction is total. Doing it once here lets every use
/// site read the origin straight out of the `Option` instead of re-deriving it.
fn neighbour_origin(
    availability: IntraEdgeAvailability,
    x: usize,
    y: usize,
) -> core::result::Result<(Option<usize>, Option<usize>), GeneralIntraResidualError> {
    let resolve = |available: bool, coord: usize| {
        if available {
            coord.checked_sub(1).map(Some)
        } else {
            Some(None)
        }
    };
    resolve(availability.above, y)
        .zip(resolve(availability.left, x))
        .ok_or_else(invalid_directional_edge_state)
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
        return Err(GeneralIntraResidualError::InvalidDirectionalEdgeState);
    }
    let top_left = match (above_in, left_in) {
        (Some(_), Some(_)) => {
            let (Some(corner_x), Some(corner_y)) = (x.checked_sub(1), y.checked_sub(1)) else {
                return Err(GeneralIntraResidualError::InvalidDirectionalEdgeState);
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
    block: CoeffBlock<'_>,
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

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use splot_recon::PixelFormat;

    /// The gathered DIP edge must equal the per-sample read of every readable
    /// position inside the storage, extended with the last one, and must fail
    /// when no position is readable.
    #[test]
    fn available_dip_edge_matches_the_per_sample_reads() {
        let (width, height) = (24, 20);
        let mut ws = super::super::new_general_intra_workspace::<u16>(
            width,
            height,
            BitDepth::Ten,
            PixelFormat::Yuv420,
        )
        .unwrap();
        for row in 0..height {
            for column in 0..width {
                ws.set_reconstructed_sample(PlaneId::Y, column, row, (row * 40 + column) as u16)
                    .unwrap();
            }
        }
        for direction in [DipEdge::Above, DipEdge::Left] {
            for (start, initial_len, edge_len, extra_units) in [
                (4, 4, 5, 0),
                (4, 8, 10, 1),
                (16, 8, 10, 4),
                (19, 4, 5, 3),
                (23, 4, 5, 3),
                (24, 4, 5, 0),
                (8, 0, 5, 0),
            ] {
                let (x, y) = match direction {
                    DipEdge::Above => (start, 3),
                    DipEdge::Left => (3, start),
                };
                let storage_len = match direction {
                    DipEdge::Above => width,
                    DipEdge::Left => height,
                };
                let readable = (initial_len + extra_units * MI_SIZE).min(edge_len);
                let reads: Vec<u16> = (start..(start + readable).min(storage_len))
                    .map(|at| match direction {
                        DipEdge::Above => ws.reconstructed_sample(PlaneId::Y, at, 2).unwrap(),
                        DipEdge::Left => ws.reconstructed_sample(PlaneId::Y, 2, at).unwrap(),
                    })
                    .collect();
                let gathered = collect_available_dip_edge(
                    &ws,
                    direction,
                    x,
                    y,
                    initial_len,
                    edge_len,
                    extra_units,
                );
                let Some(&last) = reads.last() else {
                    assert!(gathered.is_err(), "start {start} readable {readable}");
                    continue;
                };
                let mut expected = reads.clone();
                expected.resize(edge_len, last);
                assert_eq!(gathered.unwrap().as_slice(), expected.as_slice());
            }
        }
    }
}
