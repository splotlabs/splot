// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 § 7.13.5/§ 7.13.6 chroma-from-luma reconstruction helpers.
//!
//! § 7.13.5 applies 64-sample clamping to `L`; `lumaAvg` and `implicitAlpha` use
//! transform-local reference lines (docs/spec/av2/1.0.0/07-decoding-process.md).
//!
//! Feature tracking: `INFRA-DECODE-PARALLEL-STAGES`,
//! `DECODE-GENERAL-INTRA-CFL-NON-420`,
//! `DECODE-MHCCP-FRAME-EDGE-REFERENCE-EXTENT`.

use std::simd::{Simd, ToBytes, num::SimdUint, simd_swizzle};

use splot_recon::math::{approx_divide, resolve_division, round2_signed_i32};
use splot_recon::mhccp::{MhccpPredictor, MhccpRefs, derive_mhccp_params};
use splot_recon::{
    BitDepth, CurrentFramePlane, CurrentFrameWorkspace, IntraDcEdges, IntraPredictionScratchBuffer,
    IntraRectBlockSize, PixelFormat, PlaneId, ReconSample, predict_intra_dc_rect_value,
    predict_intra_dc_subsampled_rect_value,
};

use crate::bitstream::tile_payload::{
    CflMultiDirection, CflParams, CoeffBlock, GeneralIntraResidualError,
};
use crate::pipeline::reconstruct::commit_intra_prediction;
use crate::tile::block_context::NeighbourAvailability;

const MI_SIZE: usize = 4;
const CFL_FILTERS_420: [[[i32; 3]; 3]; 3] = [
    [[0, 0, 0], [0, 2, 2], [0, 2, 2]],
    [[0, 0, 0], [1, 2, 1], [1, 2, 1]],
    [[0, 1, 0], [1, 4, 1], [0, 1, 0]],
];
const CFL_FILTERS_422: [[i32; 3]; 3] = [[0, 4, 4], [2, 4, 2], [0, 8, 0]];
const CFL_ALPHA_SHIFT: u32 = 11;
const CFL_ALPHA_SCALE: i32 = 32;
const CFL_DERIVED_ALPHA_SHIFT: u8 = 8;
const NUM_REF_SAM_CFL: usize = 8;

#[allow(clippy::too_many_arguments)]
pub(crate) fn reconstruct_general_intra_chroma_cfl_block_into<T: ReconSample>(
    scratch: &mut crate::pipeline::general_intra::GeneralIntraReconScratch<T>,
    workspace: &mut CurrentFrameWorkspace<T>,
    block: CoeffBlock<'_>,
    plane_id: PlaneId,
    x: usize,
    y: usize,
    log2_width: u32,
    log2_height: u32,
    qindex: u32,
    cfl_params: CflParams,
    cfl_ds_filter_index: u8,
    sb_mib: usize,
    neighbours: NeighbourAvailability,
    bit_depth: BitDepth,
) -> core::result::Result<(), GeneralIntraResidualError> {
    let crate::pipeline::general_intra::GeneralIntraReconScratch {
        cfl_luma_ac,
        cfl_prediction,
        mhccp_refs,
        ..
    } = scratch;
    let block_size = IntraRectBlockSize::new(
        u8::try_from(log2_width).unwrap_or(u8::MAX),
        u8::try_from(log2_height).unwrap_or(u8::MAX),
    )?;
    let width = 1usize << log2_width;
    let height = 1usize << log2_height;
    let luma_ac = if matches!(cfl_params, CflParams::Multi { .. }) {
        None
    } else {
        prepare_cfl_luma_ac_into(
            workspace,
            x,
            y,
            width,
            height,
            cfl_ds_filter_index,
            sb_mib,
            neighbours,
            bit_depth,
            cfl_luma_ac,
        )?;
        Some(cfl_luma_ac.as_slice())
    };
    chroma_cfl_prediction_into(
        workspace,
        cfl_prediction,
        mhccp_refs,
        false,
        plane_id,
        x,
        y,
        log2_width,
        log2_height,
        cfl_params,
        cfl_ds_filter_index,
        sb_mib,
        neighbours,
        bit_depth,
        luma_ac,
    )?;
    commit_intra_prediction(
        workspace,
        block,
        cfl_prediction,
        plane_id,
        x,
        y,
        block_size,
        qindex,
        false,
        None,
        None,
        bit_depth,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn reconstruct_general_intra_chroma_cfl_pair_into<T: ReconSample>(
    scratch: &mut crate::pipeline::general_intra::GeneralIntraReconScratch<T>,
    workspace: &mut CurrentFrameWorkspace<T>,
    u_block: CoeffBlock<'_>,
    v_block: CoeffBlock<'_>,
    x: usize,
    y: usize,
    log2_width: u32,
    log2_height: u32,
    qindex: u32,
    cfl_params: CflParams,
    cfl_ds_filter_index: u8,
    sb_mib: usize,
    u_neighbours: NeighbourAvailability,
    v_neighbours: NeighbourAvailability,
    bit_depth: BitDepth,
) -> core::result::Result<(), GeneralIntraResidualError> {
    let crate::pipeline::general_intra::GeneralIntraReconScratch {
        cfl_luma_ac,
        cfl_prediction,
        mhccp_refs,
        ..
    } = scratch;
    let width = 1usize << log2_width;
    let height = 1usize << log2_height;
    let luma_ac = if matches!(cfl_params, CflParams::Multi { .. }) {
        None
    } else {
        prepare_cfl_luma_ac_into(
            workspace,
            x,
            y,
            width,
            height,
            cfl_ds_filter_index,
            sb_mib,
            u_neighbours,
            bit_depth,
            cfl_luma_ac,
        )?;
        Some(cfl_luma_ac.as_slice())
    };
    let block_size = IntraRectBlockSize::new(
        u8::try_from(log2_width).unwrap_or(u8::MAX),
        u8::try_from(log2_height).unwrap_or(u8::MAX),
    )?;
    let mut u_prediction = workspace.take_intra_prediction_buffer(
        IntraPredictionScratchBuffer::Primary,
        PlaneId::U,
        0,
        T::default(),
    )?;
    let mut predict = |plane_id, neighbours, luma_ready, prediction: &mut Vec<T>| {
        chroma_cfl_prediction_into(
            workspace,
            prediction,
            mhccp_refs,
            luma_ready,
            plane_id,
            x,
            y,
            log2_width,
            log2_height,
            cfl_params,
            cfl_ds_filter_index,
            sb_mib,
            neighbours,
            bit_depth,
            luma_ac,
        )
    };
    // U and V capture the same MHCCP luma references when their neighbours match.
    let result = predict(PlaneId::U, u_neighbours, false, &mut u_prediction)
        .and_then(|()| {
            predict(
                PlaneId::V,
                v_neighbours,
                u_neighbours == v_neighbours,
                cfl_prediction,
            )
        })
        .and_then(|()| {
            commit_intra_prediction(
                workspace,
                u_block,
                &u_prediction,
                PlaneId::U,
                x,
                y,
                block_size,
                qindex,
                false,
                None,
                None,
                bit_depth,
            )
        })
        .and_then(|()| {
            commit_intra_prediction(
                workspace,
                v_block,
                cfl_prediction,
                PlaneId::V,
                x,
                y,
                block_size,
                qindex,
                false,
                None,
                None,
                bit_depth,
            )
        });
    workspace.recycle_intra_prediction_buffer(IntraPredictionScratchBuffer::Primary, u_prediction);
    result
}

/// Builds one chroma block's § 7.13.5 CfL or § 7.13.6 MHCCP prediction into
/// `prediction`, sizing it to the block.
#[allow(clippy::too_many_arguments)]
fn chroma_cfl_prediction_into<T: ReconSample>(
    workspace: &CurrentFrameWorkspace<T>,
    prediction: &mut Vec<T>,
    reference_scratch: &mut [Vec<u16>; 2],
    luma_ready: bool,
    plane_id: PlaneId,
    x: usize,
    y: usize,
    log2_width: u32,
    log2_height: u32,
    cfl_params: CflParams,
    cfl_ds_filter_index: u8,
    sb_mib: usize,
    neighbours: NeighbourAvailability,
    bit_depth: BitDepth,
    luma_ac: Option<&[i32]>,
) -> core::result::Result<(), GeneralIntraResidualError> {
    if let CflParams::Multi { direction } = cfl_params {
        mhccp_prediction_into(
            workspace,
            plane_id,
            x,
            y,
            1usize << log2_width,
            1usize << log2_height,
            direction,
            cfl_ds_filter_index,
            sb_mib,
            neighbours,
            bit_depth,
            prediction,
            reference_scratch,
            luma_ready,
        )
    } else {
        cfl_prediction_into(
            workspace,
            plane_id,
            x,
            y,
            log2_width,
            log2_height,
            cfl_params,
            cfl_ds_filter_index,
            sb_mib,
            neighbours,
            bit_depth,
            luma_ac,
            prediction,
        )
    }
}

#[allow(clippy::too_many_arguments)]
fn cfl_prediction_into<T: ReconSample>(
    workspace: &CurrentFrameWorkspace<T>,
    plane_id: PlaneId,
    x: usize,
    y: usize,
    log2_width: u32,
    log2_height: u32,
    cfl_params: CflParams,
    cfl_ds_filter_index: u8,
    sb_mib: usize,
    neighbours: NeighbourAvailability,
    bit_depth: BitDepth,
    luma_ac: Option<&[i32]>,
    prediction: &mut Vec<T>,
) -> core::result::Result<(), GeneralIntraResidualError> {
    let width = 1usize << log2_width;
    let height = 1usize << log2_height;
    let block_size = IntraRectBlockSize::new(
        u8::try_from(log2_width).unwrap_or(u8::MAX),
        u8::try_from(log2_height).unwrap_or(u8::MAX),
    )?;
    let edges = workspace.intra_dc_edges_for_rect(plane_id, x, y, block_size)?;
    let dc_edges = IntraDcEdges::new(
        neighbours
            .has_left()
            .then(|| edges.left_samples())
            .flatten(),
        neighbours
            .has_above()
            .then(|| edges.above_samples())
            .flatten(),
    );
    let dc = if width > 32 || height > 32 {
        predict_intra_dc_subsampled_rect_value(bit_depth, block_size, dc_edges)?
    } else {
        predict_intra_dc_rect_value(bit_depth, block_size, dc_edges)?
    };
    let Some(luma_ac) = luma_ac else {
        return Err(GeneralIntraResidualError::InvalidReconstructionState {
            context: "CfL luma AC",
        });
    };
    apply_cfl_prediction(
        workspace,
        plane_id,
        x,
        y,
        width,
        height,
        cfl_params,
        cfl_ds_filter_index,
        sb_mib,
        neighbours,
        bit_depth,
        dc,
        luma_ac,
        prediction,
    )?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn apply_cfl_prediction<T: ReconSample>(
    workspace: &CurrentFrameWorkspace<T>,
    plane_id: PlaneId,
    x: usize,
    y: usize,
    width: usize,
    height: usize,
    cfl_params: CflParams,
    cfl_ds_filter_index: u8,
    sb_mib: usize,
    neighbours: NeighbourAvailability,
    bit_depth: BitDepth,
    dc: T,
    luma_ac: &[i32],
    prediction: &mut Vec<T>,
) -> core::result::Result<(), GeneralIntraResidualError> {
    let alpha_q3 = cfl_alpha_q3(
        workspace,
        plane_id,
        x,
        y,
        width,
        height,
        cfl_params,
        cfl_ds_filter_index,
        sb_mib,
        neighbours,
    )?;
    let max = i32::from(bit_depth.max_sample());
    let dc = i32::from(dc.to_u16());
    let Some(luma_ac) = luma_ac.get(..width.saturating_mul(height)) else {
        return Err(GeneralIntraResidualError::InvalidReconstructionState {
            context: "CfL luma AC length",
        });
    };
    let predict = |ac: i32| (dc + round2_signed_i32(alpha_q3 * ac, CFL_ALPHA_SHIFT)).clamp(0, max);
    prediction.clear();
    prediction.resize(luma_ac.len(), T::default());
    if let Some(out) = T::u16_slice_mut(prediction) {
        for (slot, &ac) in out.iter_mut().zip(luma_ac) {
            *slot = predict(ac) as u16;
        }
    } else if max <= i32::from(u8::MAX)
        && let Some(out) = T::u8_slice_mut(prediction)
    {
        for (slot, &ac) in out.iter_mut().zip(luma_ac) {
            *slot = predict(ac) as u8;
        }
    } else {
        for (slot, &ac) in prediction.iter_mut().zip(luma_ac) {
            *slot = T::try_from_u16(predict(ac) as u16)?;
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn prepare_cfl_luma_ac_into<T: ReconSample>(
    workspace: &CurrentFrameWorkspace<T>,
    x: usize,
    y: usize,
    width: usize,
    height: usize,
    cfl_ds_filter_index: u8,
    sb_mib: usize,
    neighbours: NeighbourAvailability,
    bit_depth: BitDepth,
    samples_q3: &mut Vec<i32>,
) -> core::result::Result<(), GeneralIntraResidualError> {
    if cfl_filter_index(cfl_ds_filter_index).is_none() {
        return Err(GeneralIntraResidualError::InvalidReconstructionState {
            context: "CfL filter",
        });
    }
    let pixel_format = workspace.info().pixel_format();
    let sub_x = usize::from(pixel_format.subsampling_x());
    let sub_y = usize::from(pixel_format.subsampling_y());
    let luma = CflLuma::new(workspace, cfl_ds_filter_index)?;
    let average_q3 =
        cfl_luma_average_q3(&luma, x, y, width, height, sb_mib, neighbours, bit_depth)?;
    let luma_plane = workspace.plane(PlaneId::Y)?;
    let luma_size = luma_plane.storage_size();
    let max_luma_x = luma_size.width().saturating_sub(1);
    let max_luma_y = luma_size.height().saturating_sub(1);
    let fill_420 = match cfl_filter_index(cfl_ds_filter_index) {
        Some(0) => fill_cfl_luma_ac_420_u16::<0>,
        Some(1) => fill_cfl_luma_ac_420_u16::<1>,
        _ => fill_cfl_luma_ac_420_u16::<2>,
    };
    if pixel_format == PixelFormat::Yuv420
        && let Some(luma) = T::u16_slice(luma_plane.samples())
        && fill_420(
            luma,
            luma_plane.origin_y(),
            luma_plane.stride_samples(),
            luma_plane.storage_size().width(),
            luma_plane.storage_size().height(),
            x,
            y,
            width,
            height,
            average_q3,
            samples_q3,
        )
    {
        return Ok(());
    }
    samples_q3.clear();
    samples_q3.reserve(width.saturating_mul(height));
    for row in 0..height {
        let chroma_y = y.saturating_add(row);
        let luma_y = clamped_cfl_luma_coordinate(chroma_y, sub_y, max_luma_y);
        let luma_row = luma.row(chroma_y, row == 0 || luma_y.is_multiple_of(64), None);
        for col in 0..width {
            let chroma_x = x.saturating_add(col);
            let luma_x = clamped_cfl_luma_coordinate(chroma_x, sub_x, max_luma_x);
            let clamp_x = col == 0 || luma_x.is_multiple_of(64);
            samples_q3.push(luma_row.q3(chroma_x, clamp_x)? - average_q3);
        }
    }
    Ok(())
}

/// The § 7.13.5 4:2:0 luma AC of filter `F`, or `false` when the plane
/// geometry does not hold the block.
#[allow(clippy::too_many_arguments)]
fn fill_cfl_luma_ac_420_u16<const F: usize>(
    luma: &[u16],
    origin_y: usize,
    stride: usize,
    plane_width: usize,
    plane_height: usize,
    x: usize,
    y: usize,
    width: usize,
    height: usize,
    average_q3: i32,
    output: &mut Vec<i32>,
) -> bool {
    let Some(sample_count) = width.checked_mul(height) else {
        return false;
    };
    if plane_width == 0 || plane_height == 0 || stride < plane_width {
        return false;
    }
    output.clear();
    output.resize(sample_count, 0);
    let max_x = plane_width - 1;
    let max_y = plane_height - 1;
    let average = Simd::<i32, 8>::splat(average_q3);
    let weights = CFL_FILTERS_420[F];
    for row in 0..height {
        let Some(chroma_y) = y.checked_add(row) else {
            return false;
        };
        let luma_y = clamped_cfl_luma_coordinate(chroma_y, 1, max_y);
        let clamp_y = row == 0 || luma_y.is_multiple_of(64);
        let tap_rows = [
            if clamp_y {
                luma_y
            } else {
                luma_y.saturating_sub(1)
            },
            luma_y,
            luma_y.saturating_add(1).min(max_y),
        ];
        let mut rows = [&[][..]; 3];
        for (slot, tap_row) in rows.iter_mut().zip(tap_rows) {
            let Some(start) = tap_row
                .checked_sub(origin_y)
                .and_then(|row| row.checked_mul(stride))
            else {
                return false;
            };
            let Some(samples) = luma.get(start..start + plane_width) else {
                return false;
            };
            *slot = samples;
        }
        let out = &mut output[row * width..(row + 1) * width];
        let mut col = 0usize;
        while col < width {
            let Some(chroma_x) = x.checked_add(col) else {
                return false;
            };
            let luma_x = clamped_cfl_luma_coordinate(chroma_x, 1, max_x);
            let offset_in_64 = luma_x & 63;
            if col + 8 <= width
                && col != 0
                && luma_x > 0
                && luma_x + 15 < plane_width
                && offset_in_64 != 0
                && offset_in_64 + 14 < 64
            {
                let samples = cfl_420_chunk::<F>(rows, luma_x) - average;
                out[col..col + 8].copy_from_slice(&samples.to_array()); // splot-copy-ok: publish eight CfL luma-AC samples
                col += 8;
                continue;
            }
            let clamp_x = col == 0 || luma_x.is_multiple_of(64);
            let columns = [
                if clamp_x {
                    luma_x
                } else {
                    luma_x.saturating_sub(1)
                },
                luma_x,
                luma_x.saturating_add(1).min(max_x),
            ];
            let mut total = 0;
            for (row, weights) in rows.iter().zip(weights) {
                for (&column, weight) in columns.iter().zip(weights) {
                    total += weight * i32::from(row[column]);
                }
            }
            out[col] = total - average_q3;
            col += 1;
        }
    }
    true
}

#[allow(clippy::too_many_arguments)]
fn mhccp_prediction_into<T: ReconSample>(
    workspace: &CurrentFrameWorkspace<T>,
    plane_id: PlaneId,
    x: usize,
    y: usize,
    width: usize,
    height: usize,
    direction: CflMultiDirection,
    cfl_ds_filter_index: u8,
    sb_mib: usize,
    neighbours: NeighbourAvailability,
    bit_depth: BitDepth,
    prediction: &mut Vec<T>,
    reference_scratch: &mut [Vec<u16>; 2],
    luma_ready: bool,
) -> core::result::Result<(), GeneralIntraResidualError> {
    if cfl_filter_index(cfl_ds_filter_index).is_none() {
        return Err(GeneralIntraResidualError::InvalidReconstructionState {
            context: "MHCCP filter",
        });
    }
    let mh_dir = direction.value();
    let refs = mhccp_references(
        workspace,
        plane_id,
        x,
        y,
        width,
        height,
        cfl_ds_filter_index,
        sb_mib,
        neighbours,
        mh_dir,
        reference_scratch,
        luma_ready,
    )?;
    let result = (|| {
        let predictor =
            MhccpPredictor::new(derive_mhccp_params(&refs, mh_dir, bit_depth), bit_depth);
        prediction.clear();
        prediction.resize(width.saturating_mul(height), T::default());
        let mut row_buf = [0u16; 128];
        for (row, out) in prediction.chunks_exact_mut(width).enumerate() {
            let center_start = (refs.above + row) * refs.width + refs.left;
            let center = &refs.luma[center_start..center_start + width];
            let samples = match T::u16_slice_mut(out) {
                Some(out) => out,
                None => &mut row_buf[..width],
            };
            match mh_dir {
                0 => predictor.row(center, center, samples),
                1 => {
                    let top = (refs.above + row).saturating_sub(1) * refs.width + refs.left;
                    predictor.row(&refs.luma[top..top + width], center, samples);
                }
                _ if refs.left == 0 => {
                    samples[0] = predictor.sample(center[0], center[0]);
                    predictor.row(&center[..width - 1], &center[1..], &mut samples[1..]);
                }
                _ => {
                    let left = center_start - 1;
                    predictor.row(&refs.luma[left..left + width], center, samples);
                }
            }
            if T::u16_slice(out).is_none() {
                for (slot, &sample) in out.iter_mut().zip(&row_buf[..width]) {
                    *slot = T::try_from_u16(sample)?;
                }
            }
        }
        Ok(())
    })();
    let MhccpRefs { luma, chroma, .. } = refs;
    reference_scratch[0] = luma;
    reference_scratch[1] = chroma;
    result
}

#[allow(clippy::too_many_arguments)]
fn cfl_alpha_q3<T: ReconSample>(
    workspace: &CurrentFrameWorkspace<T>,
    plane_id: PlaneId,
    x: usize,
    y: usize,
    width: usize,
    height: usize,
    cfl_params: CflParams,
    cfl_ds_filter_index: u8,
    sb_mib: usize,
    neighbours: NeighbourAvailability,
) -> core::result::Result<i32, GeneralIntraResidualError> {
    let alpha_q3 = match cfl_params {
        CflParams::Explicit { alpha_u, alpha_v } => {
            let alpha = match plane_id {
                PlaneId::U => alpha_u,
                PlaneId::V => alpha_v,
                PlaneId::Y => 0,
            };
            i32::from(alpha) * CFL_ALPHA_SCALE
        }
        CflParams::DerivedAlpha => derive_cfl_alpha_q3(
            workspace,
            plane_id,
            x,
            y,
            width,
            height,
            cfl_ds_filter_index,
            sb_mib,
            neighbours,
        )?,
        CflParams::Multi { .. } => 0,
    };
    Ok(alpha_q3)
}

#[allow(clippy::too_many_arguments)]
fn derive_cfl_alpha_q3<T: ReconSample>(
    workspace: &CurrentFrameWorkspace<T>,
    plane_id: PlaneId,
    x: usize,
    y: usize,
    width: usize,
    height: usize,
    cfl_ds_filter_index: u8,
    sb_mib: usize,
    neighbours: NeighbourAvailability,
) -> core::result::Result<i32, GeneralIntraResidualError> {
    let pixel_format = workspace.info().pixel_format();
    let have_above = neighbours.has_above();
    let have_left = neighbours.has_left();
    let (mut num_above, mut num_left) = if have_above && have_left {
        if width > height.saturating_mul(2) {
            (NUM_REF_SAM_CFL, 0)
        } else if height > width.saturating_mul(2) {
            (0, NUM_REF_SAM_CFL)
        } else {
            (NUM_REF_SAM_CFL >> 1, NUM_REF_SAM_CFL >> 1)
        }
    } else {
        (
            if have_above { NUM_REF_SAM_CFL } else { 0 },
            if have_left { NUM_REF_SAM_CFL } else { 0 },
        )
    };
    num_above = num_above.min(width);
    num_left = num_left.min(height);
    if num_above == 0 && num_left == 0 {
        return Ok(0);
    }
    let luma = CflLuma::new(workspace, cfl_ds_filter_index)?;
    let chroma_plane = PlaneSamples::new(workspace, plane_id)?;

    let mut count = 0i32;
    let mut sum_x = 0i32;
    let mut sum_y = 0i32;
    let mut sum_xy = 0i32;
    let mut sum_xx = 0i32;
    if num_above > 0 {
        let min_luma_ref_y = cfl_above_min_luma_ref_y(y, sb_mib, pixel_format);
        let above_row = luma.row(y - 1, false, min_luma_ref_y);
        let step = width.checked_div(num_above).unwrap_or(0).max(1);
        let start = if step == 1 { 0 } else { step >> 1 };
        for col in (start..width).step_by(step) {
            let chroma_x = x.saturating_add(col);
            let luma = above_row.q3(chroma_x, col == 0)? >> 3;
            let chroma = i32::from(chroma_plane.clamped(chroma_x, y - 1)?);
            sum_x += luma;
            sum_y += chroma;
            sum_xy += luma * chroma;
            sum_xx += luma * luma;
            count += 1;
        }
    }
    if num_left > 0 {
        let step = height.checked_div(num_left).unwrap_or(0).max(1);
        let start = if step == 1 { 0 } else { step >> 1 };
        for row in (start..height).step_by(step) {
            let chroma_y = y.saturating_add(row);
            let luma = luma.q3(x - 1, chroma_y, false, row == 0, None)? >> 3;
            let chroma = i32::from(chroma_plane.clamped(x - 1, chroma_y)?);
            sum_x += luma;
            sum_y += chroma;
            sum_xy += luma * chroma;
            sum_xx += luma * luma;
            count += 1;
        }
    }
    if count == 0 {
        return Ok(0);
    }
    let der = sum_xx - (sum_x * sum_x) / count;
    let nor = sum_xy - (sum_x * sum_y) / count;
    if der == 0 || nor == 0 {
        return Ok(0);
    }
    Ok(i32::from(resolve_division(
        nor,
        der,
        CFL_DERIVED_ALPHA_SHIFT,
    )))
}

#[allow(clippy::too_many_arguments)]
fn cfl_luma_average_q3<T: ReconSample>(
    luma: &CflLuma<'_, T>,
    x: usize,
    y: usize,
    width: usize,
    height: usize,
    sb_mib: usize,
    neighbours: NeighbourAvailability,
    bit_depth: BitDepth,
) -> core::result::Result<i32, GeneralIntraResidualError> {
    let step_w = if width > 32 { 2 } else { 1 };
    let step_h = if height > 32 { 2 } else { 1 };
    let mut sum = 0u32;
    let mut count = 0u32;
    if let Some(above_y) = neighbours.has_above().then(|| y.checked_sub(1)).flatten() {
        let min_luma_ref_y = cfl_above_min_luma_ref_y(y, sb_mib, luma.pixel_format);
        let above_row = luma.row(above_y, false, min_luma_ref_y);
        for col in (0..width).step_by(step_w) {
            let chroma_x = x.saturating_add(col);
            sum = sum.saturating_add(above_row.q3(chroma_x, col == 0)? as u32);
            count = count.saturating_add(1);
        }
    }
    if let Some(left_x) = neighbours.has_left().then(|| x.checked_sub(1)).flatten() {
        for row in (0..height).step_by(step_h) {
            let chroma_y = y.saturating_add(row);
            sum = sum.saturating_add(luma.q3(left_x, chroma_y, false, row == 0, None)? as u32);
            count = count.saturating_add(1);
        }
    }
    if count == 0 {
        return Ok(i32::from(8u16 << (bit_depth.bits() - 1)));
    }
    let max = (8u16 << bit_depth.bits()).saturating_sub(1);
    Ok(i32::from(approx_divide(sum, count)?.min(max)))
}

fn cfl_above_min_luma_ref_y(
    chroma_y: usize,
    sb_mib: usize,
    pixel_format: PixelFormat,
) -> Option<isize> {
    let luma_y = chroma_y << pixel_format.subsampling_y();
    let luma_mi_row = luma_y / MI_SIZE;
    let sb_height_luma = sb_mib.saturating_mul(MI_SIZE);
    let sb_start_luma_y = if sb_mib == 0 {
        0
    } else {
        luma_mi_row
            .checked_div(sb_mib)
            .map_or(0, |sb_row| sb_row.saturating_mul(sb_height_luma))
    };
    isize::try_from(sb_start_luma_y)
        .ok()
        .and_then(|sb_y| sb_y.checked_sub(1))
}

#[allow(clippy::too_many_arguments)]
fn mhccp_references<T: ReconSample>(
    workspace: &CurrentFrameWorkspace<T>,
    plane_id: PlaneId,
    x: usize,
    y: usize,
    width: usize,
    height: usize,
    cfl_ds_filter_index: u8,
    sb_mib: usize,
    neighbours: NeighbourAvailability,
    mh_dir: u8,
    reference_scratch: &mut [Vec<u16>; 2],
    luma_ready: bool,
) -> core::result::Result<MhccpRefs, GeneralIntraResidualError> {
    let pixel_format = workspace.info().pixel_format();
    let sub_x = usize::from(pixel_format.subsampling_x());
    let sub_y = usize::from(pixel_format.subsampling_y());
    let have_above = neighbours.has_above();
    let have_left = neighbours.has_left();
    let (num4_above_right, num4_below_left) =
        (neighbours.num_above_right(), neighbours.num_below_left());
    let above = if have_above { y.min(2) } else { 0 };
    let left = if have_left { x.min(2) } else { 0 };
    let luma_mi_row = (y << sub_y) / MI_SIZE;
    let sb_height_luma = sb_mib.saturating_mul(MI_SIZE);
    let sb_start_luma_y = if sb_mib == 0 {
        0
    } else {
        luma_mi_row
            .checked_div(sb_mib)
            .map_or(0, |sb_row| sb_row.saturating_mul(sb_height_luma))
    };
    let sb_chroma_y = sb_start_luma_y >> sub_y;
    let min_chroma_ref_y = sb_chroma_y.saturating_sub(1);
    let min_luma_ref_y = isize::try_from(sb_start_luma_y)
        .ok()
        .and_then(|sb_y| sb_y.checked_sub(1));
    let luma_width = width << sub_x;
    let luma_height = height << sub_y;
    let extra_right = if have_above && luma_width > MI_SIZE {
        num4_above_right.saturating_mul(MI_SIZE).min(width)
    } else {
        0
    };
    let extra_bottom = if have_left && luma_height > MI_SIZE {
        num4_below_left.saturating_mul(MI_SIZE).min(height)
    } else {
        0
    };
    let chroma_size = workspace.plane(plane_id)?.storage_size();
    let frame_right = chroma_size.width().saturating_sub(x);
    let frame_bottom = chroma_size.height().saturating_sub(y);
    let reference_width = left
        .saturating_add(width)
        .saturating_add(extra_right)
        .min(128 >> sub_x)
        .min(left.saturating_add(frame_right));
    let reference_height = above
        .saturating_add(height)
        .saturating_add(extra_bottom)
        .min(128 >> sub_y)
        .min(above.saturating_add(frame_bottom));
    if frame_right == 0 || frame_bottom == 0 {
        return Err(GeneralIntraResidualError::InvalidReconstructionState {
            context: "MHCCP reference geometry",
        });
    }
    let ref_width = reference_width.max(left.saturating_add(width));
    let ref_height = reference_height.max(above.saturating_add(height));

    let luma_plane = CflLuma::new(workspace, cfl_ds_filter_index)?;
    let chroma_plane = PlaneSamples::new(workspace, plane_id)?;
    let sample_count = ref_width.saturating_mul(ref_height);
    let [luma, chroma] = reference_scratch;
    // `luma_ready`: the other chroma plane of this block captured the same
    // luma references into `luma` already.
    let luma_ready = luma_ready && luma.len() == sample_count;
    if !luma_ready {
        luma.clear();
        luma.resize(sample_count, 0);
    }
    chroma.clear();
    chroma.resize(sample_count, 0);
    // § 7.13.7 reads neither row 0 nor column 0 of the edge as a centre:
    // chroma never, and luma only as `v[0]` of direction 1 (row 0) or 2
    // (column 0), or as block samples when the edge is absent.
    let first_luma_row = usize::from(above > 0 && mh_dir != 1);
    let first_luma_col = usize::from(left > 0 && mh_dir != 2);
    let mut q3 = [0i32; 128];
    for row in 0..reference_height {
        let chroma_y = y + row - above;
        let ref_chroma_y = chroma_y.max(min_chroma_ref_y);
        let chroma_cols = if row == 0 {
            0
        } else if row < above {
            reference_width
        } else {
            left.min(reference_width)
        };
        for col in 1..chroma_cols {
            chroma[row * ref_width + col] = chroma_plane.clamped(x + col - left, ref_chroma_y)?;
        }
        if luma_ready || row < first_luma_row {
            continue;
        }
        let luma_cols = if row < above {
            reference_width
        } else if row < above + height {
            (left + width).min(reference_width)
        } else {
            left.min(reference_width)
        };
        if luma_cols <= first_luma_col {
            continue;
        }
        let luma_row = luma_plane.row(chroma_y, row == 0, min_luma_ref_y);
        let q3 = &mut q3[first_luma_col..luma_cols];
        luma_row.q3_run(x - left + first_luma_col, first_luma_col == 0, q3)?;
        let start = row * ref_width + first_luma_col;
        for (dst, &sample) in luma[start..].iter_mut().zip(q3.iter()) {
            *dst = (sample >> 3) as u16;
        }
    }
    Ok(MhccpRefs {
        width: ref_width,
        height: ref_height,
        reference_width,
        reference_height,
        above,
        left,
        luma: core::mem::take(luma),
        chroma: core::mem::take(chroma),
    })
}

/// One plane of the current frame, resolved once for many sample reads.
struct PlaneSamples<'a, T: ReconSample> {
    plane: &'a CurrentFramePlane<T>,
    samples: &'a [T],
    origin_y: usize,
    stored_rows: usize,
    width: usize,
    stride: usize,
    height: usize,
}

impl<'a, T: ReconSample> PlaneSamples<'a, T> {
    fn new(
        workspace: &'a CurrentFrameWorkspace<T>,
        plane_id: PlaneId,
    ) -> splot_recon::Result<Self> {
        let plane = workspace.plane(plane_id)?;
        let size = plane.storage_size();
        Ok(Self {
            plane,
            samples: plane.samples(),
            origin_y: plane.origin_y(),
            stored_rows: plane.samples().len() / plane.stride_samples().max(1),
            width: size.width(),
            stride: plane.stride_samples(),
            height: size.height(),
        })
    }

    /// The sample (or error) `CurrentFramePlane::reconstructed_sample` gives,
    /// without its per-call row lookup.
    #[inline]
    fn get(&self, x: usize, y: usize) -> splot_recon::Result<u16> {
        if x < self.width
            && y < self.height
            && let Some(row) = y.checked_sub(self.origin_y)
            && row < self.stored_rows
            && let Some(sample) = self.samples.get(row * self.stride + x)
        {
            return Ok(sample.to_u16());
        }
        Ok(self.plane.reconstructed_sample(x, y)?.to_u16())
    }

    fn clamped(&self, x: usize, y: usize) -> splot_recon::Result<u16> {
        self.get(
            x.min(self.width.saturating_sub(1)),
            y.min(self.height.saturating_sub(1)),
        )
    }

    /// Stored row `y` (the samples `get` reads), or `None` when it is not
    /// stored.
    fn row(&self, y: usize) -> Option<&'a [T]> {
        let local = y.checked_sub(self.origin_y)?;
        if y >= self.height || local >= self.stored_rows || self.width == 0 {
            return None;
        }
        let start = local * self.stride;
        self.samples.get(start..start + self.width)
    }
}

/// The luma plane and the § 7.13.5 downsampling taps of one CfL block.
struct CflLuma<'a, T: ReconSample> {
    plane: PlaneSamples<'a, T>,
    pixel_format: PixelFormat,
    sub_x: usize,
    sub_y: usize,
    taps: [(isize, isize, i32); 9],
    tap_count: usize,
    filter_index: Option<usize>,
}

impl<'a, T: ReconSample> CflLuma<'a, T> {
    /// Resolves the taps of `cfl_ds_filter_index`; an invalid index has no
    /// taps, so every sum is 0.
    fn new(
        workspace: &'a CurrentFrameWorkspace<T>,
        cfl_ds_filter_index: u8,
    ) -> splot_recon::Result<Self> {
        let pixel_format = workspace.info().pixel_format();
        let sub_x = isize::from(pixel_format.subsampling_x());
        let sub_y = isize::from(pixel_format.subsampling_y());
        let mut taps = [(0, 0, 0); 9];
        let mut tap_count = 0;
        let filter_index = cfl_filter_index(cfl_ds_filter_index);
        if let Some(filter_index) = filter_index {
            for dy in -sub_y..=sub_y {
                for dx in -sub_x..=sub_x {
                    let weight = if sub_x != 0 && sub_y != 0 {
                        CFL_FILTERS_420[filter_index][(dy + sub_y) as usize][(dx + sub_x) as usize]
                    } else if sub_x != 0 {
                        CFL_FILTERS_422[filter_index][(dx + sub_x) as usize]
                    } else {
                        8
                    };
                    if weight != 0 {
                        taps[tap_count] = (dx, dy, weight);
                        tap_count += 1;
                    }
                }
            }
        }
        Ok(Self {
            plane: PlaneSamples::new(workspace, PlaneId::Y)?,
            pixel_format,
            sub_x: sub_x as usize,
            sub_y: sub_y as usize,
            taps,
            tap_count,
            filter_index,
        })
    }

    /// The § 7.13.5 downsampled luma sample at a chroma position, in Q3.
    fn q3(
        &self,
        chroma_x: usize,
        chroma_y: usize,
        clamp_x: bool,
        clamp_y: bool,
        min_luma_ref_y: Option<isize>,
    ) -> splot_recon::Result<i32> {
        self.row(chroma_y, clamp_y, min_luma_ref_y)
            .q3(chroma_x, clamp_x)
    }

    /// Resolves the luma rows that the taps of chroma row `chroma_y` read, so
    /// each sample of the row reads them directly.
    fn row(
        &self,
        chroma_y: usize,
        clamp_y: bool,
        min_luma_ref_y: Option<isize>,
    ) -> CflLumaRow<'_, 'a, T> {
        let max_y = self.plane.height.saturating_sub(1) as isize;
        let luma_y = clamped_cfl_luma_coordinate(chroma_y, self.sub_y, max_y as usize) as isize;
        let tap_row = |dy: isize| {
            let mut sy = luma_y + if clamp_y { dy.max(0) } else { dy };
            if let Some(min_y) = min_luma_ref_y {
                sy = sy.max(min_y);
            }
            sy.clamp(0, max_y) as usize
        };
        let sub_y = self.sub_y as isize;
        let mut rows = Some([&[][..]; 3]);
        for dy in -sub_y..=sub_y {
            let row = self.plane.row(tap_row(dy));
            if let (Some(rows), Some(row)) = (rows.as_mut(), row) {
                rows[(dy + 1) as usize] = row;
            } else {
                rows = None;
            }
        }
        CflLumaRow {
            luma: self,
            rows,
            tap_rows: [tap_row(-1), tap_row(0), tap_row(1)],
        }
    }
}

/// One chroma row of a [`CflLuma`]: the luma rows its taps read (each tap row
/// index, and the stored rows when the plane stores all of them).
struct CflLumaRow<'r, 'a, T: ReconSample> {
    luma: &'r CflLuma<'a, T>,
    rows: Option<[&'a [T]; 3]>,
    tap_rows: [usize; 3],
}

impl<T: ReconSample> CflLumaRow<'_, '_, T> {
    /// [`CflLuma::q3`] at `chroma_x` on this row.
    fn q3(&self, chroma_x: usize, clamp_x: bool) -> splot_recon::Result<i32> {
        let luma = self.luma;
        let max_x = luma.plane.width.saturating_sub(1) as isize;
        let luma_x = clamped_cfl_luma_coordinate(chroma_x, luma.sub_x, max_x as usize) as isize;
        let mut total = 0i32;
        for &(dx, dy, weight) in &luma.taps[..luma.tap_count] {
            let sx = (luma_x + if clamp_x { dx.max(0) } else { dx }).clamp(0, max_x) as usize;
            let row = (dy + 1) as usize;
            let sample = match &self.rows {
                Some(rows) => rows[row][sx].to_u16(),
                None => luma.plane.get(sx, self.tap_rows[row])?,
            };
            total += weight * i32::from(sample);
        }
        Ok(total)
    }

    /// [`Self::q3`] at `chroma_x0 + i` for each `out[i]`; only the first
    /// sample clamps when `clamp_first`.
    fn q3_run(
        &self,
        chroma_x0: usize,
        clamp_first: bool,
        out: &mut [i32],
    ) -> splot_recon::Result<()> {
        const LANES: usize = 8;
        let luma = self.luma;
        let mut done = 0;
        if luma.sub_x == 1
            && let Some(rows) = &self.rows
            && let [Some(r0), Some(r1), Some(r2)] = (*rows).map(T::u16_slice)
        {
            let rows = [r0, r1, r2];
            let width = luma.plane.width;
            done = usize::from(clamp_first || chroma_x0 == 0).min(out.len());
            for (i, slot) in out[..done].iter_mut().enumerate() {
                *slot = self.q3(chroma_x0 + i, clamp_first)?;
            }
            while done + LANES <= out.len() && 2 * (chroma_x0 + done) + 2 * LANES < width {
                let luma_x = 2 * (chroma_x0 + done);
                let acc = match (luma.sub_y, luma.filter_index) {
                    (1, Some(0)) => cfl_420_chunk::<0>(rows, luma_x),
                    (1, Some(1)) => cfl_420_chunk::<1>(rows, luma_x),
                    (1, Some(2)) => cfl_420_chunk::<2>(rows, luma_x),
                    _ => {
                        let mut acc = Simd::<i32, LANES>::splat(0);
                        for &(dx, dy, weight) in &luma.taps[..luma.tap_count] {
                            let start = luma_x.wrapping_add_signed(dx);
                            let row = rows[(dy + 1) as usize];
                            let v = Simd::<u16, 16>::from_slice(&row[start..start + 16]);
                            let even = simd_swizzle!(v, [0, 2, 4, 6, 8, 10, 12, 14]).cast::<i32>();
                            acc += even * Simd::splat(weight);
                        }
                        acc
                    }
                };
                out[done..done + LANES].copy_from_slice(&acc.to_array()); // splot-copy-ok: publish eight CfL luma samples
                done += LANES;
            }
        }
        for (i, slot) in out.iter_mut().enumerate().skip(done) {
            *slot = self.q3(chroma_x0 + i, clamp_first && i == 0)?;
        }
        Ok(())
    }
}

/// Eight § 7.13.5 4:2:0 Q3 luma samples of filter `F` from luma column
/// `luma_x` (at least 1, with 17 more samples in each row). Each `u32` lane
/// of a row read from `luma_x - 1` holds the left and centre taps, and one
/// from `luma_x` holds the centre and right taps.
#[allow(clippy::inline_always, reason = "measured: unrolls the constant taps")]
#[inline(always)]
fn cfl_420_chunk<const F: usize>(rows: [&[u16]; 3], luma_x: usize) -> Simd<i32, 8> {
    let [top, middle, bottom] = CFL_FILTERS_420[F];
    cfl_420_row_taps(rows[0], luma_x, top)
        + cfl_420_row_taps(rows[1], luma_x, middle)
        + cfl_420_row_taps(rows[2], luma_x, bottom)
}

/// One luma row's left, centre and right taps of [`cfl_420_chunk`].
#[allow(clippy::inline_always, reason = "measured: folds the constant weights")]
#[inline(always)]
fn cfl_420_row_taps(row: &[u16], luma_x: usize, weights: [i32; 3]) -> Simd<i32, 8> {
    let pairs = |start: usize| {
        let v = Simd::<u16, 16>::from_slice(&row[start..start + 16]);
        let pairs = Simd::<u32, 8>::from_le_bytes(v.to_le_bytes());
        (
            (pairs & Simd::splat(0xffff)).cast::<i32>(),
            (pairs >> 16).cast::<i32>(),
        )
    };
    let mut acc = Simd::splat(0);
    if weights[1] != 0 || weights[2] != 0 {
        let (center, right) = pairs(luma_x);
        acc = center * Simd::splat(weights[1]) + right * Simd::splat(weights[2]);
    }
    if weights[0] != 0 {
        acc += pairs(luma_x - 1).0 * Simd::splat(weights[0]);
    }
    acc
}

fn clamped_cfl_luma_coordinate(
    chroma_coordinate: usize,
    subsampling: usize,
    max_luma_coordinate: usize,
) -> usize {
    chroma_coordinate.min(max_luma_coordinate >> subsampling) << subsampling
}

const fn cfl_filter_index(cfl_ds_filter_index: u8) -> Option<usize> {
    match cfl_ds_filter_index {
        0 | 3 => Some(0),
        1 => Some(1),
        2 => Some(2),
        _ => None,
    }
}

#[cfg(test)]
#[path = "cfl_tests.rs"]
mod tests;
