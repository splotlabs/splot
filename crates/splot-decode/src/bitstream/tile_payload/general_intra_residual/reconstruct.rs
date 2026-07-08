// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Shared transform-block reconstruction setup.

use splot_recon::{
    BitDepth, DequantBlockParams, DpcmDirection, InverseTransform2dOuter, PlaneId, ac_quantizer,
    dc_quantizer, dequantize_block,
};

use super::super::coeff_loop::max_level::CoeffTransformClass;
use super::{
    GeneralIntraResidualError, LumaCoeffBlock, current_quantizer_deltas, resolve_block_qm,
};

pub(super) struct ReconstructBlockSetup {
    pub(super) adjusted: usize,
    pub(super) samples: usize,
    pub(super) params: DequantBlockParams,
    pub(super) transform: InverseTransform2dOuter,
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
