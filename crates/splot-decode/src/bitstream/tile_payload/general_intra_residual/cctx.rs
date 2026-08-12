// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Cross-chroma transform reconstruction helpers.

use splot_recon::math::round2_signed_i32;
use splot_recon::{
    BitDepth, PlaneId, ReconSample, inverse_transform_2d_outer, reconstruct_add_residual,
};

use super::reconstruct::{dequantize_coeff_block, reconstruct_block_setup};
use super::{GeneralIntraResidualError, LumaCoeffBlock, with_residual_scratch};

const CCTX_PREC_BITS: u32 = 8;
const CCTX_MTX: [[i32; 2]; 6] = [
    [181, 181],
    [222, 128],
    [128, 222],
    [181, -181],
    [222, -128],
    [128, -222],
];

#[cfg(test)]
#[allow(clippy::too_many_arguments)]
pub(crate) fn reconstruct_general_intra_chroma_cctx_pair_with_predictions<T: ReconSample>(
    u_block: &LumaCoeffBlock,
    u_prediction: &[T],
    v_block: &LumaCoeffBlock,
    v_prediction: &[T],
    qindex: u32,
    log2_width: u32,
    log2_height: u32,
    cctx_type: usize,
    use_ddt: bool,
    bit_depth: BitDepth,
) -> Result<(Vec<T>, Vec<T>), GeneralIntraResidualError> {
    let mut u_out = Vec::new();
    let mut v_out = Vec::new();
    reconstruct_general_intra_chroma_cctx_pair_into(
        u_block,
        u_prediction,
        v_block,
        v_prediction,
        qindex,
        log2_width,
        log2_height,
        cctx_type,
        use_ddt,
        bit_depth,
        &mut u_out,
        &mut v_out,
    )?;
    Ok((u_out, v_out))
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn reconstruct_general_intra_chroma_cctx_pair_into<T: ReconSample>(
    u_block: &LumaCoeffBlock,
    u_prediction: &[T],
    v_block: &LumaCoeffBlock,
    v_prediction: &[T],
    qindex: u32,
    log2_width: u32,
    log2_height: u32,
    cctx_type: usize,
    use_ddt: bool,
    bit_depth: BitDepth,
    u_out: &mut Vec<T>,
    v_out: &mut Vec<T>,
) -> Result<(), GeneralIntraResidualError> {
    let u_setup = reconstruct_block_setup(
        u_prediction.len(),
        qindex,
        PlaneId::U,
        log2_width,
        log2_height,
        u_block.plane_tx_type,
        u_block.use_tcq,
        use_ddt,
        u_block.lossless,
        None,
        bit_depth,
    )?;
    let v_setup = reconstruct_block_setup(
        v_prediction.len(),
        qindex,
        PlaneId::V,
        log2_width,
        log2_height,
        u_block.plane_tx_type,
        v_block.use_tcq,
        use_ddt,
        v_block.lossless,
        None,
        bit_depth,
    )?;
    if u_setup.adjusted != v_setup.adjusted || u_setup.samples != v_setup.samples {
        return Err(GeneralIntraResidualError::InvalidReconstructionState {
            context: "CCTX paired transform geometry",
        });
    }

    u_out.resize(u_setup.samples, T::default());
    v_out.resize(v_setup.samples, T::default());
    with_residual_scratch(|scratch| {
        let u_dequant = &mut scratch.dequant[..u_setup.adjusted];
        let v_dequant = &mut scratch.dequant_pair[..v_setup.adjusted];
        dequantize_coeff_block(u_block, &u_setup.params, u_dequant)?;
        dequantize_coeff_block(v_block, &v_setup.params, v_dequant)?;
        apply_cross_chroma_transform(cctx_type, bit_depth, u_dequant, v_dequant)?;

        let residual = &mut scratch.residual[..u_setup.samples];
        inverse_transform_2d_outer(&u_setup.transform, u_dequant, residual)?;
        reconstruct_add_residual(u_prediction, residual, bit_depth, u_out)?;
        inverse_transform_2d_outer(&v_setup.transform, v_dequant, residual)?;
        reconstruct_add_residual(v_prediction, residual, bit_depth, v_out)?;
        Ok::<(), GeneralIntraResidualError>(())
    })?;
    Ok(())
}

pub(super) fn apply_cross_chroma_transform(
    cctx_type: usize,
    bit_depth: BitDepth,
    u_dequant: &mut [i32],
    v_dequant: &mut [i32],
) -> Result<(), GeneralIntraResidualError> {
    if u_dequant.len() != v_dequant.len() {
        return Err(GeneralIntraResidualError::InvalidReconstructionState {
            context: "CCTX coefficient lengths",
        });
    }
    let [cos, sin] = *CCTX_MTX
        .get(cctx_type.checked_sub(1).ok_or(
            GeneralIntraResidualError::InvalidReconstructionState {
                context: "CCTX type",
            },
        )?)
        .ok_or(GeneralIntraResidualError::InvalidReconstructionState {
            context: "CCTX type",
        })?;
    let bound = 1i32 << (u32::from(bit_depth.bits()) + 7);
    for (u, v) in u_dequant.iter_mut().zip(v_dequant.iter_mut()) {
        let saved_u = (*u).clamp(-bound, bound - 1);
        let saved_v = (*v).clamp(-bound, bound - 1);
        let next_u = round2_signed_i32(saved_u * cos - saved_v * sin, CCTX_PREC_BITS)
            .clamp(-bound, bound - 1);
        let next_v = round2_signed_i32(saved_u * sin + saved_v * cos, CCTX_PREC_BITS)
            .clamp(-bound, bound - 1);
        *u = next_u;
        *v = next_v;
    }
    Ok(())
}
