// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Shared transform-block reconstruction setup.

use splot_recon::math::round2_i32;
use splot_recon::{
    BitDepth, CurrentFrameSurface, DequantBlockParams, DpcmDirection, IntraRectBlockSize,
    InverseTransform2dOuter, PlaneId, ReconSample, SecondaryInverseTransform, ac_quantizer,
    dc_quantizer, dequant_coefficient, dequantize_block, inverse_transform_2d_outer,
    secondary_inverse_transform, tx_class,
};

use super::super::coeff_loop::max_level::CoeffTransformClass;
use super::{
    ADST_ADST, D67_PRED, D157_PRED, DCT_DCT, GeneralIntraResidualError, H_PRED, IST_4X4_HEIGHT,
    IST_8X8_HEIGHT, IST_8X8_HEIGHT_RED, LumaCoeffBlock, LumaTransformTypeContext, SMOOTH_H_PRED,
    current_quantizer_deltas, intra_secondary_transform_kernel, intra_secondary_transform_mode,
    resolve_block_qm, unsupported_transform_tool_residual,
    unsupported_transform_tool_residual_error,
};

pub(super) struct ReconstructBlockSetup {
    pub(super) adjusted: usize,
    pub(super) samples: usize,
    pub(super) params: DequantBlockParams,
    pub(super) transform: InverseTransform2dOuter,
}

pub(super) fn resolve_secondary_inverse_transform(
    block: &LumaCoeffBlock,
    log2_width: u32,
    log2_height: u32,
    bit_depth: BitDepth,
    luma_context: Option<LumaTransformTypeContext>,
) -> Result<Option<SecondaryInverseTransform>, GeneralIntraResidualError> {
    let Some(ist) = block.intra_ist else {
        return Ok(None);
    };
    if ist.sec_tx_type == 0 {
        return Ok(None);
    }
    let tx_width = transform_dimension(log2_width)?;
    let tx_height = transform_dimension(log2_height)?;
    let w = tx_width.min(32);
    let h = tx_height.min(32);
    let large = w >= 8 && h >= 8;
    let n = if !large {
        IST_4X4_HEIGHT
    } else if (tx_width == 8 && tx_height == 8) || block.plane_tx_type == ADST_ADST {
        IST_8X8_HEIGHT_RED
    } else {
        IST_8X8_HEIGHT
    };
    let (kernel, transpose) = if let Some(luma_context) = luma_context {
        let most_probable_stx_set =
            ist.most_probable_stx_set
                .ok_or(unsupported_transform_tool_residual_error(
                    "unsupported_dctonly_residual_intra_ist_missing_most_probable_stx_set",
                ))?;
        let mode = intra_secondary_transform_mode(luma_context, tx_width, tx_height)?;
        (
            intra_secondary_transform_kernel(
                mode,
                block.plane_tx_type,
                most_probable_stx_set,
                tx_width,
                tx_height,
            )?,
            matches!(mode, H_PRED | D157_PRED | D67_PRED | SMOOTH_H_PRED),
        )
    } else {
        if block.plane_tx_type != DCT_DCT
            || tx_width < 16
            || tx_height < 16
            || ist.most_probable_stx_set.is_some()
        {
            return unsupported_transform_tool_residual(
                "unsupported_dctonly_residual_inter_ist_context",
            );
        }
        (0, false)
    };
    Ok(Some(SecondaryInverseTransform {
        w,
        h,
        n,
        kernel,
        sec_tx_type: ist.sec_tx_type,
        primary_scan_class: tx_class(block.plane_tx_type),
        transpose,
        bit_depth,
    }))
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn reconstruct_general_intra_coeff_block_rect_with_prediction_into<T: ReconSample>(
    block: &LumaCoeffBlock,
    prediction: &[T],
    out: &mut Vec<T>,
    qindex: u32,
    plane_id: PlaneId,
    log2_width: u32,
    log2_height: u32,
    use_tcq: bool,
    luma_context: Option<LumaTransformTypeContext>,
    dpcm: Option<DpcmDirection>,
    bit_depth: BitDepth,
) -> Result<(), GeneralIntraResidualError> {
    let (plane_id, dpcm, secondary) = if let Some(luma_context) = luma_context {
        (
            PlaneId::Y,
            luma_context.dpcm,
            resolve_secondary_inverse_transform(
                block,
                log2_width,
                log2_height,
                bit_depth,
                Some(luma_context),
            )?,
        )
    } else {
        (plane_id, dpcm, None)
    };
    super::reconstruct_general_intra_block_rect_with_prediction_core(
        &block.quant,
        prediction,
        out,
        qindex,
        plane_id,
        log2_width,
        log2_height,
        block.plane_tx_type,
        use_tcq && block.use_tcq,
        false,
        block.lossless,
        secondary.as_ref(),
        dpcm,
        bit_depth,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn reconstruct_inter_coeff_block_residual_rect_into<T: ReconSample>(
    sink: &mut CurrentFrameSurface<'_, '_, T>,
    block: &LumaCoeffBlock,
    plane_id: PlaneId,
    x: usize,
    y: usize,
    block_size: IntraRectBlockSize,
    qindex: u32,
    use_tcq: bool,
    use_ddt: bool,
    bit_depth: BitDepth,
) -> Result<(), GeneralIntraResidualError> {
    let log2_width = u32::from(block_size.log2_width());
    let log2_height = u32::from(block_size.log2_height());
    let secondary =
        resolve_secondary_inverse_transform(block, log2_width, log2_height, bit_depth, None)?;
    let setup = reconstruct_block_setup(
        block_size.sample_count(),
        qindex,
        plane_id,
        log2_width,
        log2_height,
        block.plane_tx_type,
        use_tcq && block.use_tcq,
        use_ddt,
        block.lossless,
        None,
        bit_depth,
    )?;
    if block.quant.len() != setup.adjusted {
        return Err(GeneralIntraResidualError::QuantLength {
            expected: setup.adjusted,
            actual: block.quant.len(),
        });
    }
    if block.eob == 1
        && block.plane_tx_type == DCT_DCT
        && !block.lossless
        && secondary.is_none()
        && setup.params.qm.is_none()
    {
        let residual = dct_dc_residual(block.quant[0], &setup);
        sink.add_constant_residual_rect_block(plane_id, x, y, block_size, residual)?;
        return Ok(());
    }
    super::with_residual_scratch(|scratch| {
        let dequant = &mut scratch.dequant[..setup.adjusted];
        dequantize_block(&setup.params, &block.quant, dequant)?;
        if let Some(secondary) = secondary.as_ref() {
            secondary_inverse_transform(dequant, secondary)?;
        }
        let residual = &mut scratch.residual[..setup.samples];
        inverse_transform_2d_outer(&setup.transform, dequant, residual)?;
        sink.add_residual_rect_block(plane_id, x, y, block_size, residual)?;
        Ok(())
    })
}

#[inline]
fn dct_dc_residual(quant: i32, setup: &ReconstructBlockSetup) -> i32 {
    let transform = setup.transform;
    let mut dc = dequant_coefficient(
        quant,
        setup.params.dc_quant,
        setup.params.dq_denom,
        setup.params.bit_depth,
    );
    if transform.log2_width.abs_diff(transform.log2_height) % 2 == 1 {
        dc = round2_i32(dc * 2896, 12);
    }

    let input_bound = 1i32 << (u32::from(transform.bit_depth.bits()) + 7);
    let row = round2_i32(
        dc.clamp(-input_bound, input_bound - 1) * 64,
        u32::from(transform.row_shift),
    )
    .clamp(-input_bound, input_bound - 1);
    let output_bound = 1i32 << u32::from(transform.bit_depth.bits());
    round2_i32(row * 64, u32::from(transform.col_shift)).clamp(-output_bound, output_bound - 1)
}

fn transform_dimension(log2_dim: u32) -> Result<usize, GeneralIntraResidualError> {
    if !(2..=6).contains(&log2_dim) {
        return unsupported_transform_tool_residual(
            "unsupported_dctonly_residual_intra_ist_invalid_transform_shape",
        );
    }
    Ok(1usize << log2_dim)
}

#[allow(clippy::too_many_arguments)]
pub(super) fn reconstruct_block_setup(
    prediction_len: usize,
    qindex: u32,
    plane_id: PlaneId,
    log2_width: u32,
    log2_height: u32,
    plane_tx_type: usize,
    use_tcq: bool,
    use_ddt: bool,
    lossless: bool,
    dpcm: Option<DpcmDirection>,
    bit_depth: BitDepth,
) -> Result<ReconstructBlockSetup, GeneralIntraResidualError> {
    let orig_w = 1usize << log2_width;
    let orig_h = 1usize << log2_height;

    let adj_w = 1usize << log2_width.min(5);
    let adj_h = 1usize << log2_height.min(5);
    let adjusted = adj_w * adj_h;
    let samples = orig_w * orig_h;
    if prediction_len != samples {
        return Err(GeneralIntraResidualError::PredictionLength {
            expected: samples,
            actual: prediction_len,
        });
    }
    let deltas = current_quantizer_deltas();
    let pels = (orig_w * orig_h) as u32;
    let tcq_two_d = use_tcq
        && CoeffTransformClass::from_plane_tx_type(plane_tx_type) == CoeffTransformClass::TwoD;
    let dq_shift = u32::from(pels > 256) + u32::from(pels > 1024) + u32::from(tcq_two_d);
    let dq_denom = 1u32 << dq_shift;
    let params = DequantBlockParams {
        dc_quant: dc_quantizer(plane_id, qindex, deltas, bit_depth),
        ac_quant: ac_quantizer(plane_id, qindex, deltas, bit_depth),
        tx_width: adj_w,
        tx_height: adj_h,
        dq_denom,
        bit_depth,
        qm: resolve_block_qm(
            plane_id,
            plane_tx_type,
            adj_w,
            adj_h,
            log2_width,
            log2_height,
        ),
    };
    let transform = InverseTransform2dOuter::resolve(
        plane_tx_type,
        log2_width,
        log2_height,
        use_ddt,
        lossless,
        bit_depth,
        dpcm,
    )
    .map_err(|source| GeneralIntraResidualError::Reconstruct { source })?;

    Ok(ReconstructBlockSetup {
        adjusted,
        samples,
        params,
        transform,
    })
}

pub(super) fn dequantize_coeff_block(
    block: &LumaCoeffBlock,
    params: &DequantBlockParams,
    out: &mut [i32],
) -> Result<(), GeneralIntraResidualError> {
    if block.all_zero {
        out.fill(0);
        return Ok(());
    }
    if block.quant.len() != out.len() {
        return Err(GeneralIntraResidualError::QuantLength {
            expected: out.len(),
            actual: block.quant.len(),
        });
    }
    dequantize_block(params, &block.quant, out)?;
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn dct_dc_residual_matches_full_transform_for_every_shape_and_bit_depth() {
        for bit_depth in [BitDepth::Eight, BitDepth::Ten] {
            for log2_width in 2..=6 {
                for log2_height in 2..=6 {
                    let samples = (1usize << log2_width) * (1usize << log2_height);
                    let setup = reconstruct_block_setup(
                        samples,
                        220,
                        PlaneId::Y,
                        log2_width,
                        log2_height,
                        DCT_DCT,
                        true,
                        true,
                        false,
                        None,
                        bit_depth,
                    )
                    .unwrap();
                    assert!(setup.params.qm.is_none());
                    for quant_dc in [i32::MIN, -257, -1, 1, 257, i32::MAX] {
                        let mut quant = vec![0; setup.adjusted];
                        quant[0] = quant_dc;
                        let mut dequant = vec![0; setup.adjusted];
                        let mut residual = vec![0; setup.samples];
                        dequantize_block(&setup.params, &quant, &mut dequant).unwrap();
                        inverse_transform_2d_outer(&setup.transform, &dequant, &mut residual)
                            .unwrap();
                        let dc = dct_dc_residual(quant_dc, &setup);
                        assert!(
                            residual.iter().all(|&value| value == dc),
                            "shape {log2_width}x{log2_height}, {bit_depth:?}, quant {quant_dc}"
                        );
                    }
                }
            }
        }
    }
}
