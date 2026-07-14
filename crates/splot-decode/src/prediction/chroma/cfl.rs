// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 § 7.13.5/§ 7.13.6 chroma-from-luma reconstruction helpers.
//!
//! Feature tracking: `INFRA-DECODE-PARALLEL-STAGES`.

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
    reconstruct_general_intra_coeff_block_rect_with_prediction_into,
};

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

std::thread_local! {
    static CFL_LUMA_AC_RECYCLER: std::cell::RefCell<Vec<i32>> = const { std::cell::RefCell::new(Vec::new()) };
    static CFL_PRED_RECYCLER: std::cell::RefCell<Option<Box<dyn std::any::Any>>> = const { std::cell::RefCell::new(None) };
}

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
    let block_size = IntraRectBlockSize::new(
        u8::try_from(log2_width).unwrap_or(u8::MAX),
        u8::try_from(log2_height).unwrap_or(u8::MAX),
    )?;
    let mut out = workspace.take_intra_prediction_buffer(
        IntraPredictionScratchBuffer::Primary,
        plane_id,
        block_size.sample_count(),
        T::default(),
    )?;
    let result = reconstruct_general_intra_chroma_cfl_block(
        workspace,
        block,
        &mut out,
        plane_id,
        x,
        y,
        log2_width,
        log2_height,
        qindex,
        cfl_params,
        cfl_ds_filter_index,
        sb_mib,
        num4_above_right,
        num4_below_left,
        bit_depth,
        None,
    )
    .and_then(|()| {
        workspace
            .write_rect_block(plane_id, x, y, block_size, &out)
            .map_err(Into::into)
    });
    workspace.recycle_intra_prediction_buffer(IntraPredictionScratchBuffer::Primary, out);
    result
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn reconstruct_general_intra_chroma_cfl_pair_into<T: ReconSample>(
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
    let width = 1usize << log2_width;
    let height = 1usize << log2_height;
    let mut owned_luma_ac = CFL_LUMA_AC_RECYCLER.with(std::cell::RefCell::take);
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
            &mut owned_luma_ac,
        )?;
        Some(&*owned_luma_ac)
    };
    let block_size = IntraRectBlockSize::new(
        u8::try_from(log2_width).unwrap_or(u8::MAX),
        u8::try_from(log2_height).unwrap_or(u8::MAX),
    )?;
    let mut u_out = workspace.take_intra_prediction_buffer(
        IntraPredictionScratchBuffer::Primary,
        PlaneId::U,
        block_size.sample_count(),
        T::default(),
    )?;
    let mut v_out = match workspace.take_intra_prediction_buffer(
        IntraPredictionScratchBuffer::Secondary,
        PlaneId::V,
        block_size.sample_count(),
        T::default(),
    ) {
        Ok(out) => out,
        Err(source) => {
            workspace.recycle_intra_prediction_buffer(IntraPredictionScratchBuffer::Primary, u_out);
            return Err(source.into());
        }
    };
    let run = |plane_id, block, (num4_above_right, num4_below_left), out: &mut Vec<T>| {
        reconstruct_general_intra_chroma_cfl_block(
            workspace,
            block,
            out,
            plane_id,
            x,
            y,
            log2_width,
            log2_height,
            qindex,
            cfl_params,
            cfl_ds_filter_index,
            sb_mib,
            num4_above_right,
            num4_below_left,
            bit_depth,
            luma_ac,
        )
    };
    let result = run(PlaneId::U, u_block, u_neighbours, &mut u_out)
        .and_then(|()| run(PlaneId::V, v_block, v_neighbours, &mut v_out))
        .and_then(|()| {
            workspace
                .write_rect_block(PlaneId::U, x, y, block_size, &u_out)
                .map_err(Into::into)
        })
        .and_then(|()| {
            workspace
                .write_rect_block(PlaneId::V, x, y, block_size, &v_out)
                .map_err(Into::into)
        });
    workspace.recycle_intra_prediction_buffer(IntraPredictionScratchBuffer::Primary, u_out);
    workspace.recycle_intra_prediction_buffer(IntraPredictionScratchBuffer::Secondary, v_out);
    CFL_LUMA_AC_RECYCLER.with(|cell| {
        let mut recycler = cell.borrow_mut();
        if recycler.capacity() < owned_luma_ac.capacity() {
            *recycler = owned_luma_ac;
        }
    });
    result
}

#[allow(clippy::too_many_arguments)]
fn reconstruct_general_intra_chroma_cfl_block<T: ReconSample>(
    workspace: &CurrentFrameWorkspace<T>,
    block: &LumaCoeffBlock,
    out: &mut Vec<T>,
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
    luma_ac: Option<&[i32]>,
) -> core::result::Result<(), GeneralIntraResidualError> {
    let width = 1usize << log2_width;
    let height = 1usize << log2_height;

    let mut prediction: Vec<T> = CFL_PRED_RECYCLER.with(|cell| {
        let mut slot = cell.borrow_mut();
        if let Some(any) = slot.take() {
            match any.downcast::<Vec<T>>() {
                Ok(vec) => *vec,
                Err(_) => Vec::new(),
            }
        } else {
            Vec::new()
        }
    });

    let res = (|| {
        if cfl_params.index == CflIndex::Multi {
            mhccp_prediction_into(
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
                &mut prediction,
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
                &mut prediction,
            )
        }?;

        if block.all_zero {
            out.copy_from_slice(&prediction);
            Ok(())
        } else {
            reconstruct_general_intra_coeff_block_rect_with_prediction_into(
                block,
                &prediction,
                out,
                qindex,
                plane_id,
                log2_width,
                log2_height,
                false,
                None,
                None,
                bit_depth,
            )
        }
    })();

    CFL_PRED_RECYCLER.with(|cell| {
        *cell.borrow_mut() = Some(Box::new(prediction));
    });

    res
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
    let mut owned_luma_ac = CFL_LUMA_AC_RECYCLER.with(std::cell::RefCell::take);
    let luma_ac = if let Some(luma_ac) = luma_ac {
        luma_ac
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
            &mut owned_luma_ac,
        )?;
        &owned_luma_ac
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
        bit_depth,
        luma_ac,
        prediction,
    )?;
    CFL_LUMA_AC_RECYCLER.with(|cell| {
        let mut recycler = cell.borrow_mut();
        if recycler.capacity() < owned_luma_ac.capacity() {
            *recycler = owned_luma_ac;
        }
    });
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
    Ok(())
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
    )?;
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
                predicted =
                    predicted.saturating_add(mul_fixed32_adapt(params[k], vector[k], MHCCP_BITS));
            }
            prediction.push(T::try_from_u16(predicted.clamp(0, max) as u16)?);
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

    let mut luma = vec![0u16; ref_width.saturating_mul(ref_height)];
    let mut chroma = vec![0u16; ref_width.saturating_mul(ref_height)];
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
) -> core::result::Result<i32, GeneralIntraResidualError> {
    let Some(filter_index) = cfl_filter_index(cfl_ds_filter_index) else {
        return Ok(0);
    };
    let pixel_format = workspace.info().pixel_format();
    let sub_x = isize::from(pixel_format.subsampling_x());
    let sub_y = isize::from(pixel_format.subsampling_y());
    let luma_x = (chroma_x << sub_x) as isize;
    let luma_y = (chroma_y << sub_y) as isize;
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
            total += weight * i32::from(clamped_luma_sample(workspace, sx, sy)?.to_u16());
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

#[cfg(test)]
#[path = "cfl_tests.rs"]
mod tests;
