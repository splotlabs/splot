// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 § 7.13.5/§ 7.13.6 chroma-from-luma reconstruction helpers.
//!
//! Feature tracking: `INFRA-DECODE-PARALLEL-STAGES`.

use std::simd::{Simd, num::SimdUint, simd_swizzle};

use splot_recon::math::{approx_divide, resolve_division, round2_i32, round2_signed_i32};
use splot_recon::mhccp::{
    MHCCP_BITS, MHCCP_PARAM_COUNT, MhccpRefs, derive_mhccp_params, mul_fixed32_adapt,
};
use splot_recon::{
    BitDepth, CurrentFrameWorkspace, IntraPredictionScratchBuffer, IntraRectBlockSize, PixelFormat,
    PlaneId, ReconSample, predict_intra_dc_rect_value, predict_intra_dc_subsampled_rect_value,
};

use crate::bitstream::tile_payload::{
    CflIndex, CflParams, GeneralIntraResidualError, LumaCoeffBlock,
};
use crate::pipeline::reconstruct::commit_intra_prediction;

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
    block: &LumaCoeffBlock,
    plane_id: PlaneId,
    x: usize,
    y: usize,
    log2_width: u32,
    log2_height: u32,
    qindex: u32,
    cfl_params: CflParams,
    cfl_ds_filter_index: u8,
    sb_mib: usize,
    num4_above_right: usize,
    num4_below_left: usize,
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
    let luma_ac = if cfl_params.index == CflIndex::Multi {
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
            bit_depth,
            cfl_luma_ac,
        )?;
        Some(cfl_luma_ac.as_slice())
    };
    chroma_cfl_prediction_into(
        workspace,
        cfl_prediction,
        mhccp_refs,
        plane_id,
        x,
        y,
        log2_width,
        log2_height,
        cfl_params,
        cfl_ds_filter_index,
        sb_mib,
        num4_above_right,
        num4_below_left,
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
    u_block: &LumaCoeffBlock,
    v_block: &LumaCoeffBlock,
    x: usize,
    y: usize,
    log2_width: u32,
    log2_height: u32,
    qindex: u32,
    cfl_params: CflParams,
    cfl_ds_filter_index: u8,
    sb_mib: usize,
    u_neighbours: (usize, usize),
    v_neighbours: (usize, usize),
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
    let luma_ac = if cfl_params.index == CflIndex::Multi {
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
    let mut predict = |plane_id, (num4_above_right, num4_below_left), prediction: &mut Vec<T>| {
        chroma_cfl_prediction_into(
            workspace,
            prediction,
            mhccp_refs,
            plane_id,
            x,
            y,
            log2_width,
            log2_height,
            cfl_params,
            cfl_ds_filter_index,
            sb_mib,
            num4_above_right,
            num4_below_left,
            bit_depth,
            luma_ac,
        )
    };
    let result = predict(PlaneId::U, u_neighbours, &mut u_prediction)
        .and_then(|()| predict(PlaneId::V, v_neighbours, cfl_prediction))
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
    plane_id: PlaneId,
    x: usize,
    y: usize,
    log2_width: u32,
    log2_height: u32,
    cfl_params: CflParams,
    cfl_ds_filter_index: u8,
    sb_mib: usize,
    num4_above_right: usize,
    num4_below_left: usize,
    bit_depth: BitDepth,
    luma_ac: Option<&[i32]>,
) -> core::result::Result<(), GeneralIntraResidualError> {
    if cfl_params.index == CflIndex::Multi {
        mhccp_prediction_into(
            workspace,
            plane_id,
            x,
            y,
            1usize << log2_width,
            1usize << log2_height,
            cfl_params,
            cfl_ds_filter_index,
            sb_mib,
            num4_above_right,
            num4_below_left,
            bit_depth,
            prediction,
            reference_scratch,
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
    bit_depth: BitDepth,
    luma_ac: Option<&[i32]>,
    prediction: &mut Vec<T>,
) -> core::result::Result<(), GeneralIntraResidualError> {
    if cfl_filter_index(cfl_ds_filter_index).is_none() {
        return Err(
            GeneralIntraResidualError::UnsupportedTransformToolResidual {
                reason: "general_intra_cfl_filter",
            },
        );
    }
    let width = 1usize << log2_width;
    let height = 1usize << log2_height;
    let block_size = IntraRectBlockSize::new(
        u8::try_from(log2_width).unwrap_or(u8::MAX),
        u8::try_from(log2_height).unwrap_or(u8::MAX),
    )?;
    let edges = workspace.intra_dc_edges_for_rect(plane_id, x, y, block_size)?;
    let dc = if width > 32 || height > 32 {
        predict_intra_dc_subsampled_rect_value(bit_depth, block_size, edges.as_dc_edges())?
    } else {
        predict_intra_dc_rect_value(bit_depth, block_size, edges.as_dc_edges())?
    };
    prediction.clear();
    prediction.resize(width.saturating_mul(height), dc);
    let luma_ac = luma_ac.ok_or(GeneralIntraResidualError::UnexpectedBranch)?;
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
        bit_depth,
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
    bit_depth: BitDepth,
    luma_ac: &[i32],
    prediction: &mut [T],
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
    )?;
    let max = i32::from(bit_depth.max_sample());
    for row in 0..height {
        for col in 0..width {
            let index = row * width + col;
            let scaled_luma = round2_signed_i32(alpha_q3 * luma_ac[index], CFL_ALPHA_SHIFT);
            let dc = i32::from(prediction[index].to_u16());
            let clipped = (dc + scaled_luma).clamp(0, max) as u16;
            prediction[index] = T::try_from_u16(clipped)?;
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
    bit_depth: BitDepth,
    samples_q3: &mut Vec<i32>,
) -> core::result::Result<(), GeneralIntraResidualError> {
    let timer = crate::timing::start();
    if cfl_filter_index(cfl_ds_filter_index).is_none() {
        return Err(
            GeneralIntraResidualError::UnsupportedTransformToolResidual {
                reason: "general_intra_cfl_filter",
            },
        );
    }
    let pixel_format = workspace.info().pixel_format();
    let sub_x = usize::from(pixel_format.subsampling_x());
    let sub_y = usize::from(pixel_format.subsampling_y());
    let average_q3 = cfl_luma_average_q3(
        workspace,
        x,
        y,
        width,
        height,
        cfl_ds_filter_index,
        sb_mib,
        bit_depth,
    )?;
    let luma_plane = workspace.plane(PlaneId::Y)?;
    if pixel_format == PixelFormat::Yuv420
        && cfl_ds_filter_index == 1
        && let Some(luma) = T::u16_slice(luma_plane.samples())
        && fill_cfl_luma_ac_420_filter1_u16(
            luma,
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
        crate::timing::accumulate(crate::timing::Phase::CflLumaAc, timer);
        return Ok(());
    }
    samples_q3.clear();
    samples_q3.reserve(width.saturating_mul(height));
    for row in 0..height {
        let chroma_y = y.saturating_add(row);
        let luma_y = chroma_y << sub_y;
        let clamp_y = row == 0 || luma_y % 64 == 0;
        for col in 0..width {
            let chroma_x = x.saturating_add(col);
            let luma_x = chroma_x << sub_x;
            let clamp_x = col == 0 || luma_x % 64 == 0;
            samples_q3.push(
                cfl_luma_q3(
                    workspace,
                    chroma_x,
                    chroma_y,
                    clamp_x,
                    clamp_y,
                    cfl_ds_filter_index,
                )? - average_q3,
            );
        }
    }
    crate::timing::accumulate(crate::timing::Phase::CflLumaAc, timer);
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn fill_cfl_luma_ac_420_filter1_u16(
    luma: &[u16],
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
    let Some(required) = stride.checked_mul(plane_height) else {
        return false;
    };
    if plane_width == 0 || plane_height == 0 || stride < plane_width || luma.len() < required {
        return false;
    }
    output.clear();
    output.resize(sample_count, 0);
    let max_x = plane_width - 1;
    let max_y = plane_height - 1;
    let average = Simd::<i32, 8>::splat(average_q3);
    for row in 0..height {
        let Some(chroma_y) = y.checked_add(row) else {
            return false;
        };
        let Some(luma_y) = chroma_y.checked_mul(2) else {
            return false;
        };
        let row0 = luma_y.min(max_y);
        let row1 = luma_y.saturating_add(1).min(max_y);
        let Some(row0_start) = row0.checked_mul(stride) else {
            return false;
        };
        let Some(row1_start) = row1.checked_mul(stride) else {
            return false;
        };
        let Some(row0) = luma.get(row0_start..row0_start + plane_width) else {
            return false;
        };
        let Some(row1) = luma.get(row1_start..row1_start + plane_width) else {
            return false;
        };
        let out = &mut output[row * width..(row + 1) * width];
        let mut col = 0usize;
        while col < width {
            let Some(chroma_x) = x.checked_add(col) else {
                return false;
            };
            let Some(luma_x) = chroma_x.checked_mul(2) else {
                return false;
            };
            let offset_in_64 = luma_x & 63;
            if col + 8 <= width
                && col != 0
                && luma_x > 0
                && luma_x + 15 < plane_width
                && offset_in_64 != 0
                && offset_in_64 + 14 < 64
            {
                let a0 = Simd::<u16, 16>::from_slice(&row0[luma_x - 1..]);
                let b0 = Simd::<u16, 16>::from_slice(&row0[luma_x..]);
                let a1 = Simd::<u16, 16>::from_slice(&row1[luma_x - 1..]);
                let b1 = Simd::<u16, 16>::from_slice(&row1[luma_x..]);
                let left = simd_swizzle!(a0 + a1, [0, 2, 4, 6, 8, 10, 12, 14]).cast::<i32>();
                let rows = b0 + b1;
                let center = simd_swizzle!(rows, [0, 2, 4, 6, 8, 10, 12, 14]).cast::<i32>();
                let right = simd_swizzle!(rows, [1, 3, 5, 7, 9, 11, 13, 15]).cast::<i32>();
                out[col..col + 8].copy_from_slice(
                    &(left + center * Simd::splat(2) + right - average).to_array(),
                ); // splot-copy-ok: publish eight CfL luma-AC samples
                col += 8;
                continue;
            }
            let center_x = luma_x.min(max_x);
            let clamp_x = col == 0 || luma_x.is_multiple_of(64);
            let left_x = if clamp_x {
                center_x
            } else {
                luma_x.saturating_sub(1).min(max_x)
            };
            let right_x = luma_x.saturating_add(1).min(max_x);
            out[col] = i32::from(row0[left_x])
                + 2 * i32::from(row0[center_x])
                + i32::from(row0[right_x])
                + i32::from(row1[left_x])
                + 2 * i32::from(row1[center_x])
                + i32::from(row1[right_x])
                - average_q3;
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
    cfl_params: CflParams,
    cfl_ds_filter_index: u8,
    sb_mib: usize,
    num4_above_right: usize,
    num4_below_left: usize,
    bit_depth: BitDepth,
    prediction: &mut Vec<T>,
    reference_scratch: &mut [Vec<u16>; 2],
) -> core::result::Result<(), GeneralIntraResidualError> {
    let Some(mh_dir) = cfl_params.mh_dir else {
        return Err(
            GeneralIntraResidualError::UnsupportedTransformToolResidual {
                reason: "general_intra_cfl_multi_missing_mh_dir",
            },
        );
    };
    if mh_dir > 2 || cfl_filter_index(cfl_ds_filter_index).is_none() {
        return Err(
            GeneralIntraResidualError::UnsupportedTransformToolResidual {
                reason: "general_intra_cfl_multi_filter",
            },
        );
    }
    let refs = mhccp_references(
        workspace,
        plane_id,
        x,
        y,
        width,
        height,
        cfl_ds_filter_index,
        sb_mib,
        num4_above_right,
        num4_below_left,
        reference_scratch,
    )?;
    let result = (|| {
        let params = derive_mhccp_params(&refs, mh_dir, bit_depth);
        let max = i32::from(bit_depth.max_sample());
        let mid = 1i32 << (u32::from(bit_depth.bits()) - 1);
        prediction.clear();
        prediction.reserve(width.saturating_mul(height));
        for row in 0..height {
            for col in 0..width {
                let center_index = (refs.above + row) * refs.width + refs.left + col;
                let center = i32::from(refs.luma[center_index]);
                let linear = match mh_dir {
                    0 => center,
                    1 => {
                        let top_row = refs.above.saturating_add(row).saturating_sub(1);
                        i32::from(refs.luma[top_row * refs.width + refs.left + col])
                    }
                    _ => {
                        let left_col = refs.left.saturating_add(col).saturating_sub(1);
                        i32::from(refs.luma[(refs.above + row) * refs.width + left_col])
                    }
                };
                let vector = [
                    linear,
                    round2_i32(center.saturating_mul(center), u32::from(bit_depth.bits())),
                    mid,
                ];
                let mut predicted = 0i32;
                for k in 0..MHCCP_PARAM_COUNT {
                    predicted = predicted
                        .saturating_add(mul_fixed32_adapt(params[k], vector[k], MHCCP_BITS));
                }
                prediction.push(T::try_from_u16(predicted.clamp(0, max) as u16)?);
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
) -> core::result::Result<i32, GeneralIntraResidualError> {
    let alpha_q3 = match cfl_params.index {
        CflIndex::Explicit => {
            let alpha = match plane_id {
                PlaneId::U => cfl_params.alpha_u,
                PlaneId::V => cfl_params.alpha_v,
                PlaneId::Y => 0,
            };
            i32::from(alpha) * CFL_ALPHA_SCALE
        }
        CflIndex::DerivedAlpha => derive_cfl_alpha_q3(
            workspace,
            plane_id,
            x,
            y,
            width,
            height,
            cfl_ds_filter_index,
            sb_mib,
        )?,
        CflIndex::Multi => 0,
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
) -> core::result::Result<i32, GeneralIntraResidualError> {
    let pixel_format = workspace.info().pixel_format();
    let sub_x = usize::from(pixel_format.subsampling_x());
    let sub_y = usize::from(pixel_format.subsampling_y());
    let have_above = y > 0;
    let have_left = x > 0;
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

    let mut count = 0i32;
    let mut sum_x = 0i32;
    let mut sum_y = 0i32;
    let mut sum_xy = 0i32;
    let mut sum_xx = 0i32;
    if num_above > 0 {
        let min_luma_ref_y = cfl_above_min_luma_ref_y(y, sb_mib, pixel_format);
        let step = width.checked_div(num_above).unwrap_or(0).max(1);
        let start = if step == 1 { 0 } else { step >> 1 };
        for col in (start..width).step_by(step) {
            let chroma_x = x.saturating_add(col);
            let luma_x = chroma_x << sub_x;
            let clamp_x = col == 0 || luma_x % 64 == 0;
            let luma = cfl_luma_q3_with_min_y(
                workspace,
                chroma_x,
                y - 1,
                clamp_x,
                false,
                min_luma_ref_y,
                cfl_ds_filter_index,
            )? >> 3;
            let chroma =
                i32::from(clamped_chroma_sample(workspace, plane_id, chroma_x, y - 1)?.to_u16());
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
            let luma_y = chroma_y << sub_y;
            let clamp_y = row == 0 || luma_y % 64 == 0;
            let luma = cfl_luma_q3(
                workspace,
                x - 1,
                chroma_y,
                false,
                clamp_y,
                cfl_ds_filter_index,
            )? >> 3;
            let chroma =
                i32::from(clamped_chroma_sample(workspace, plane_id, x - 1, chroma_y)?.to_u16());
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
    workspace: &CurrentFrameWorkspace<T>,
    x: usize,
    y: usize,
    width: usize,
    height: usize,
    cfl_ds_filter_index: u8,
    sb_mib: usize,
    bit_depth: BitDepth,
) -> core::result::Result<i32, GeneralIntraResidualError> {
    let pixel_format = workspace.info().pixel_format();
    let sub_x = usize::from(pixel_format.subsampling_x());
    let sub_y = usize::from(pixel_format.subsampling_y());
    let step_w = if width > 32 { 2 } else { 1 };
    let step_h = if height > 32 { 2 } else { 1 };
    let mut sum = 0u32;
    let mut count = 0u32;
    if let Some(above_y) = y.checked_sub(1) {
        let min_luma_ref_y = cfl_above_min_luma_ref_y(y, sb_mib, pixel_format);
        for col in (0..width).step_by(step_w) {
            let chroma_x = x.saturating_add(col);
            let luma_x = chroma_x << sub_x;
            let clamp_x = col == 0 || luma_x % 64 == 0;
            sum = sum.saturating_add(cfl_luma_q3_with_min_y(
                workspace,
                chroma_x,
                above_y,
                clamp_x,
                false,
                min_luma_ref_y,
                cfl_ds_filter_index,
            )? as u32);
            count = count.saturating_add(1);
        }
    }
    if let Some(left_x) = x.checked_sub(1) {
        for row in (0..height).step_by(step_h) {
            let chroma_y = y.saturating_add(row);
            let luma_y = chroma_y << sub_y;
            let clamp_y = row == 0 || luma_y % 64 == 0;
            sum = sum.saturating_add(cfl_luma_q3(
                workspace,
                left_x,
                chroma_y,
                false,
                clamp_y,
                cfl_ds_filter_index,
            )? as u32);
            count = count.saturating_add(1);
        }
    }
    if count == 0 {
        return Ok(i32::from(8u16 << (bit_depth.bits() - 1)));
    }
    let max = (8u16 << bit_depth.bits()).saturating_sub(1);
    Ok(i32::from(approx_divide(sum, count)?.min(max)))
}

fn cfl_luma_q3<T: ReconSample>(
    workspace: &CurrentFrameWorkspace<T>,
    chroma_x: usize,
    chroma_y: usize,
    clamp_x: bool,
    clamp_y: bool,
    cfl_ds_filter_index: u8,
) -> core::result::Result<i32, GeneralIntraResidualError> {
    cfl_luma_q3_with_min_y(
        workspace,
        chroma_x,
        chroma_y,
        clamp_x,
        clamp_y,
        None,
        cfl_ds_filter_index,
    )
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
    num4_above_right: usize,
    num4_below_left: usize,
    reference_scratch: &mut [Vec<u16>; 2],
) -> core::result::Result<MhccpRefs, GeneralIntraResidualError> {
    let pixel_format = workspace.info().pixel_format();
    let sub_x = usize::from(pixel_format.subsampling_x());
    let sub_y = usize::from(pixel_format.subsampling_y());
    let have_above = y > 0;
    let have_left = x > 0;
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
    let ref_width = left
        .saturating_add(width)
        .saturating_add(extra_right)
        .min(128 >> sub_x)
        .min(left.saturating_add(frame_right));
    let ref_height = above
        .saturating_add(height)
        .saturating_add(extra_bottom)
        .min(128 >> sub_y)
        .min(above.saturating_add(frame_bottom));
    if ref_width < left.saturating_add(width) || ref_height < above.saturating_add(height) {
        return Err(
            GeneralIntraResidualError::UnsupportedTransformToolResidual {
                reason: "general_intra_mhccp_reference_geometry",
            },
        );
    }

    let sample_count = ref_width.saturating_mul(ref_height);
    let [luma, chroma] = reference_scratch;
    luma.clear();
    luma.resize(sample_count, 0);
    chroma.clear();
    chroma.resize(sample_count, 0);
    for row in 0..ref_height {
        for col in 0..ref_width {
            let chroma_x = x + col - left;
            let chroma_y = y + row - above;
            if row < above || col < left {
                let ref_chroma_y = chroma_y.max(min_chroma_ref_y);
                chroma[row * ref_width + col] =
                    clamped_chroma_sample(workspace, plane_id, chroma_x, ref_chroma_y)?.to_u16();
            }
            if mhccp_luma_ref_available(row, col, above, left, width, height) {
                let clamp_x = col == 0;
                let clamp_y = row == 0;
                luma[row * ref_width + col] = (cfl_luma_q3_with_min_y(
                    workspace,
                    chroma_x,
                    chroma_y,
                    clamp_x,
                    clamp_y,
                    min_luma_ref_y,
                    cfl_ds_filter_index,
                )? >> 3) as u16;
            }
        }
    }
    Ok(MhccpRefs {
        width: ref_width,
        height: ref_height,
        above,
        left,
        luma: core::mem::take(luma),
        chroma: core::mem::take(chroma),
    })
}

fn cfl_luma_q3_with_min_y<T: ReconSample>(
    workspace: &CurrentFrameWorkspace<T>,
    chroma_x: usize,
    chroma_y: usize,
    clamp_x: bool,
    clamp_y: bool,
    min_luma_ref_y: Option<isize>,
    cfl_ds_filter_index: u8,
) -> core::result::Result<i32, GeneralIntraResidualError> {
    let Some(filter_index) = cfl_filter_index(cfl_ds_filter_index) else {
        return Ok(0);
    };
    let pixel_format = workspace.info().pixel_format();
    let sub_x = isize::from(pixel_format.subsampling_x());
    let sub_y = isize::from(pixel_format.subsampling_y());
    let luma_x = (chroma_x << sub_x) as isize;
    let luma_y = (chroma_y << sub_y) as isize;
    let y_plane = workspace.plane(PlaneId::Y)?;
    let size = y_plane.storage_size();
    let max_x = size.width().saturating_sub(1) as isize;
    let max_y = size.height().saturating_sub(1) as isize;
    let mut total = 0i32;
    for dy in -sub_y..=sub_y {
        for dx in -sub_x..=sub_x {
            let weight = if sub_x != 0 && sub_y != 0 {
                CFL_FILTERS_420[filter_index][(dy + sub_y) as usize][(dx + sub_x) as usize]
            } else if sub_x != 0 {
                CFL_FILTERS_422[filter_index][(dx + sub_x) as usize]
            } else {
                8
            };
            if weight == 0 {
                continue;
            }
            let sx = luma_x + if clamp_x { dx.max(0) } else { dx };
            let mut sy = luma_y + if clamp_y { dy.max(0) } else { dy };
            if let Some(min_y) = min_luma_ref_y {
                sy = sy.max(min_y);
            }
            total += weight
                * i32::from(
                    y_plane
                        .reconstructed_sample(
                            sx.clamp(0, max_x) as usize,
                            sy.clamp(0, max_y) as usize,
                        )?
                        .to_u16(),
                );
        }
    }
    Ok(total)
}

fn clamped_chroma_sample<T: ReconSample>(
    workspace: &CurrentFrameWorkspace<T>,
    plane_id: PlaneId,
    x: usize,
    y: usize,
) -> splot_recon::Result<T> {
    let size = workspace.plane(plane_id)?.storage_size();
    workspace.reconstructed_sample(
        plane_id,
        x.min(size.width().saturating_sub(1)),
        y.min(size.height().saturating_sub(1)),
    )
}

const fn cfl_filter_index(cfl_ds_filter_index: u8) -> Option<usize> {
    match cfl_ds_filter_index {
        0 | 3 => Some(0),
        1 => Some(1),
        2 => Some(2),
        _ => None,
    }
}

fn mhccp_luma_ref_available(
    row: usize,
    col: usize,
    above: usize,
    left: usize,
    width: usize,
    height: usize,
) -> bool {
    (row < above || col < left.saturating_add(width))
        && (row < above.saturating_add(height) || col < left)
}

#[cfg(test)]
#[path = "cfl_tests.rs"]
mod tests;
