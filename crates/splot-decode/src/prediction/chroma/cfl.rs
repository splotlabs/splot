// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 § 7.13.5/§ 7.13.6 chroma-from-luma reconstruction helpers.

use splot_recon::math::{approx_divide, clip3, resolve_division, round2, round2_signed};
use splot_recon::{
    BitDepth, CurrentFrameWorkspace, IntraRectBlockSize, PlaneId, ReconSample,
    predict_intra_dc_rect_value, predict_intra_dc_subsampled_rect_value,
};

use crate::bitstream::tile_payload::{
    CflIndex, CflParams, GeneralIntraResidualError, LumaCoeffBlock,
    reconstruct_general_intra_block_rect_with_prediction,
};

use super::mhccp::{
    MHCCP_BITS, MHCCP_PARAM_COUNT, MhccpRefs, derive_mhccp_params, mul_fixed32_adapt,
};

const MI_SIZE: usize = 4;
const CFL_FILTERS_420: [[[i64; 3]; 3]; 3] = [
    [[0, 0, 0], [0, 2, 2], [0, 2, 2]],
    [[0, 0, 0], [1, 2, 1], [1, 2, 1]],
    [[0, 1, 0], [1, 4, 1], [0, 1, 0]],
];
const CFL_ALPHA_SHIFT: u32 = 11;
const CFL_ALPHA_SCALE: i64 = 32;
const CFL_DERIVED_ALPHA_SHIFT: u8 = 8;
const NUM_REF_SAM_CFL: usize = 8;

#[allow(clippy::too_many_arguments)]
pub(crate) fn reconstruct_general_intra_chroma_cfl_block_into<T: ReconSample>(
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
    let width = 1usize << log2_width;
    let height = 1usize << log2_height;
    let prediction = if cfl_params.index == CflIndex::Multi {
        mhccp_prediction(
            workspace,
            plane_id,
            x,
            y,
            width,
            height,
            cfl_params,
            cfl_ds_filter_index,
            sb_mib,
            num4_above_right,
            num4_below_left,
            bit_depth,
        )?
    } else {
        cfl_prediction(
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
        )?
    };
    let out = if block.all_zero {
        prediction
    } else {
        reconstruct_general_intra_block_rect_with_prediction(
            &block.quant,
            &prediction,
            qindex,
            plane_id,
            log2_width,
            log2_height,
            block.plane_tx_type,
            false,
            bit_depth,
        )?
    };
    let block_size = IntraRectBlockSize::new(
        u8::try_from(log2_width).unwrap_or(u8::MAX),
        u8::try_from(log2_height).unwrap_or(u8::MAX),
    )?;
    workspace.write_rect_block(plane_id, x, y, block_size, &out)?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn cfl_prediction<T: ReconSample>(
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
) -> core::result::Result<Vec<T>, GeneralIntraResidualError> {
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
    let mut prediction = vec![dc; width.saturating_mul(height)];
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
        &mut prediction,
    )?;
    Ok(prediction)
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
    let luma_avg = cfl_luma_average_q3(
        workspace,
        x,
        y,
        width,
        height,
        cfl_ds_filter_index,
        sb_mib,
        bit_depth,
    )?;
    let max = i64::from(bit_depth.max_sample());
    for row in 0..height {
        let chroma_y = y.saturating_add(row);
        let luma_y = chroma_y.saturating_mul(2);
        let clamp_y = row == 0 || luma_y % 64 == 0;
        for col in 0..width {
            let chroma_x = x.saturating_add(col);
            let luma_x = chroma_x.saturating_mul(2);
            let clamp_x = col == 0 || luma_x % 64 == 0;
            let luma = cfl_luma_q3(
                workspace,
                chroma_x,
                chroma_y,
                clamp_x,
                clamp_y,
                cfl_ds_filter_index,
            )?;
            let scaled_luma = round2_signed(alpha_q3 * (luma - luma_avg), CFL_ALPHA_SHIFT);
            let index = row * width + col;
            let dc = i64::from(prediction[index].to_u16());
            let clipped = clip3(0, max, dc + scaled_luma) as u16;
            prediction[index] = T::try_from_u16(clipped)?;
        }
    }
    Ok(())
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
) -> core::result::Result<i64, GeneralIntraResidualError> {
    let alpha_q3 = match cfl_params.index {
        CflIndex::Explicit => {
            let alpha = match plane_id {
                PlaneId::U => cfl_params.alpha_u,
                PlaneId::V => cfl_params.alpha_v,
                PlaneId::Y => 0,
            };
            i64::from(alpha) * CFL_ALPHA_SCALE
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
) -> core::result::Result<i64, GeneralIntraResidualError> {
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

    let mut count = 0i64;
    let mut sum_x = 0i64;
    let mut sum_y = 0i64;
    let mut sum_xy = 0i64;
    let mut sum_xx = 0i64;
    if num_above > 0 {
        let min_luma_ref_y = cfl_above_min_luma_ref_y(y, sb_mib);
        let step = width.checked_div(num_above).unwrap_or(0).max(1);
        let start = if step == 1 { 0 } else { step >> 1 };
        for col in (start..width).step_by(step) {
            let chroma_x = x.saturating_add(col);
            let luma_x = chroma_x.saturating_mul(2);
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
                i64::from(clamped_chroma_sample(workspace, plane_id, chroma_x, y - 1)?.to_u16());
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
            let luma_y = chroma_y.saturating_mul(2);
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
                i64::from(clamped_chroma_sample(workspace, plane_id, x - 1, chroma_y)?.to_u16());
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
    Ok(i64::from(resolve_division(
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
) -> core::result::Result<i64, GeneralIntraResidualError> {
    let step_w = if width > 32 { 2 } else { 1 };
    let step_h = if height > 32 { 2 } else { 1 };
    let mut sum = 0u64;
    let mut count = 0u64;
    if let Some(above_y) = y.checked_sub(1) {
        let min_luma_ref_y = cfl_above_min_luma_ref_y(y, sb_mib);
        for col in (0..width).step_by(step_w) {
            let chroma_x = x.saturating_add(col);
            let luma_x = chroma_x.saturating_mul(2);
            let clamp_x = col == 0 || luma_x % 64 == 0;
            sum = sum.saturating_add(cfl_luma_q3_with_min_y(
                workspace,
                chroma_x,
                above_y,
                clamp_x,
                false,
                min_luma_ref_y,
                cfl_ds_filter_index,
            )? as u64);
            count = count.saturating_add(1);
        }
    }
    if let Some(left_x) = x.checked_sub(1) {
        for row in (0..height).step_by(step_h) {
            let chroma_y = y.saturating_add(row);
            let luma_y = chroma_y.saturating_mul(2);
            let clamp_y = row == 0 || luma_y % 64 == 0;
            sum = sum.saturating_add(cfl_luma_q3(
                workspace,
                left_x,
                chroma_y,
                false,
                clamp_y,
                cfl_ds_filter_index,
            )? as u64);
            count = count.saturating_add(1);
        }
    }
    if count == 0 {
        return Ok(i64::from(8u16 << (bit_depth.bits() - 1)));
    }
    let max = (8u16 << bit_depth.bits()).saturating_sub(1);
    Ok(i64::from(approx_divide(sum, count)?.min(max)))
}

fn cfl_luma_q3<T: ReconSample>(
    workspace: &CurrentFrameWorkspace<T>,
    chroma_x: usize,
    chroma_y: usize,
    clamp_x: bool,
    clamp_y: bool,
    cfl_ds_filter_index: u8,
) -> core::result::Result<i64, GeneralIntraResidualError> {
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

fn cfl_above_min_luma_ref_y(chroma_y: usize, sb_mib: usize) -> Option<isize> {
    let luma_mi_row = chroma_y / 2;
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
fn mhccp_prediction<T: ReconSample>(
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
) -> core::result::Result<Vec<T>, GeneralIntraResidualError> {
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
    )?;
    let params = derive_mhccp_params(&refs, mh_dir, bit_depth);
    let max = i64::from(bit_depth.max_sample());
    let mid = 1i64 << (u32::from(bit_depth.bits()) - 1);
    let mut prediction = Vec::with_capacity(width.saturating_mul(height));
    for row in 0..height {
        for col in 0..width {
            let center_index = (refs.above + row) * refs.width + refs.left + col;
            let center = refs.luma[center_index];
            let linear = match mh_dir {
                0 => center,
                1 => {
                    let top_row = refs.above.saturating_add(row).saturating_sub(1);
                    refs.luma[top_row * refs.width + refs.left + col]
                }
                _ => {
                    let left_col = refs.left.saturating_add(col).saturating_sub(1);
                    refs.luma[(refs.above + row) * refs.width + left_col]
                }
            };
            let vector = [
                linear,
                round2(center.saturating_mul(center), u32::from(bit_depth.bits())),
                mid,
            ];
            let mut predicted = 0i64;
            for k in 0..MHCCP_PARAM_COUNT {
                predicted =
                    predicted.saturating_add(mul_fixed32_adapt(params[k], vector[k], MHCCP_BITS));
            }
            prediction.push(T::try_from_u16(clip3(0, max, predicted) as u16)?);
        }
    }
    Ok(prediction)
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
) -> core::result::Result<MhccpRefs, GeneralIntraResidualError> {
    let have_above = y > 0;
    let have_left = x > 0;
    let above = if have_above { y.min(2) } else { 0 };
    let left = if have_left { x.min(2) } else { 0 };
    let luma_mi_row = y / 2;
    let sb_height_luma = sb_mib.saturating_mul(MI_SIZE);
    let sb_start_luma_y = if sb_mib == 0 {
        0
    } else {
        luma_mi_row
            .checked_div(sb_mib)
            .map_or(0, |sb_row| sb_row.saturating_mul(sb_height_luma))
    };
    let sb_chroma_y = sb_start_luma_y / 2;
    let min_chroma_ref_y = sb_chroma_y.saturating_sub(1);
    let min_luma_ref_y = isize::try_from(sb_start_luma_y)
        .ok()
        .and_then(|sb_y| sb_y.checked_sub(1));
    let extra_right = if have_above && width > 4 {
        num4_above_right.saturating_mul(MI_SIZE).min(width)
    } else {
        0
    };
    let extra_bottom = if have_left && height > 4 {
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
        .min(64)
        .min(left.saturating_add(frame_right));
    let ref_height = above
        .saturating_add(height)
        .saturating_add(extra_bottom)
        .min(64)
        .min(above.saturating_add(frame_bottom));
    if ref_width < left.saturating_add(width) || ref_height < above.saturating_add(height) {
        return Err(
            GeneralIntraResidualError::UnsupportedTransformToolResidual {
                reason: "general_intra_mhccp_reference_geometry",
            },
        );
    }

    let mut luma = vec![0i64; ref_width.saturating_mul(ref_height)];
    let mut chroma = vec![0i64; ref_width.saturating_mul(ref_height)];
    for row in 0..ref_height {
        for col in 0..ref_width {
            let chroma_x = x + col - left;
            let chroma_y = y + row - above;
            if row < above || col < left {
                let ref_chroma_y = chroma_y.max(min_chroma_ref_y);
                chroma[row * ref_width + col] = i64::from(
                    clamped_chroma_sample(workspace, plane_id, chroma_x, ref_chroma_y)?.to_u16(),
                );
            }
            if mhccp_luma_ref_available(row, col, above, left, width, height) {
                let clamp_x = col == 0;
                let clamp_y = row == 0;
                luma[row * ref_width + col] = cfl_luma_q3_with_min_y(
                    workspace,
                    chroma_x,
                    chroma_y,
                    clamp_x,
                    clamp_y,
                    min_luma_ref_y,
                    cfl_ds_filter_index,
                )? >> 3;
            }
        }
    }
    Ok(MhccpRefs {
        width: ref_width,
        height: ref_height,
        above,
        left,
        luma,
        chroma,
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
) -> core::result::Result<i64, GeneralIntraResidualError> {
    let Some(filter_index) = cfl_filter_index(cfl_ds_filter_index) else {
        return Ok(0);
    };
    let luma_x = (chroma_x.saturating_mul(2)) as isize;
    let luma_y = (chroma_y.saturating_mul(2)) as isize;
    let mut total = 0i64;
    for (dy_index, dy) in [-1isize, 0, 1].into_iter().enumerate() {
        for (dx_index, dx) in [-1isize, 0, 1].into_iter().enumerate() {
            let weight = CFL_FILTERS_420[filter_index][dy_index][dx_index];
            if weight == 0 {
                continue;
            }
            let sx = luma_x + if clamp_x { dx.max(0) } else { dx };
            let mut sy = luma_y + if clamp_y { dy.max(0) } else { dy };
            if let Some(min_y) = min_luma_ref_y {
                sy = sy.max(min_y);
            }
            total += weight * i64::from(clamped_luma_sample(workspace, sx, sy)?.to_u16());
        }
    }
    Ok(total)
}

fn clamped_luma_sample<T: ReconSample>(
    workspace: &CurrentFrameWorkspace<T>,
    x: isize,
    y: isize,
) -> splot_recon::Result<T> {
    let size = workspace.plane(PlaneId::Y)?.storage_size();
    let max_x = size.width().saturating_sub(1) as isize;
    let max_y = size.height().saturating_sub(1) as isize;
    workspace.reconstructed_sample(
        PlaneId::Y,
        x.clamp(0, max_x) as usize,
        y.clamp(0, max_y) as usize,
    )
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
