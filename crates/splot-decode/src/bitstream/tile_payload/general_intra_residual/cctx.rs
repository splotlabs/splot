// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Cross-chroma transform reconstruction helpers.

use splot_core::simd::SimdBackend;
use splot_recon::math::round2_signed_i32;
use splot_recon::{
    BitDepth, CurrentFrameSurface, IntraRectBlockSize, PlaneId, ReconSample,
    inverse_transform_2d_outer_with_scratch_and_backend, reconstruct_add_residual_with,
};

#[cfg(test)]
use super::reconstruct::dequantize_coeff_block;
use super::reconstruct::{
    ReconstructBlockSetup, dequantize_coeff_block_with, reconstruct_block_setup,
};
use super::{CoeffBlock, GeneralIntraResidualError, with_residual_scratch};

const CCTX_PREC_BITS: u32 = 8;
pub(crate) const CCTX_MTX: [[i32; 2]; 6] = [
    [181, 181],
    [222, 128],
    [128, 222],
    [181, -181],
    [222, -128],
    [128, -222],
];

#[cfg(test)]
#[expect(clippy::too_many_arguments)]
pub(crate) fn reconstruct_general_intra_chroma_cctx_pair_with_predictions<T: ReconSample>(
    u_block: CoeffBlock<'_>,
    u_prediction: &[T],
    v_block: CoeffBlock<'_>,
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
    crate::test_support::with_auto_backend!(backend => {
        reconstruct_general_intra_chroma_cctx_pair_into_with(
            backend,
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
        )
    })?;
    Ok((u_out, v_out))
}

#[expect(clippy::too_many_arguments)]
pub(crate) fn reconstruct_general_intra_chroma_cctx_pair_into_with<
    B: SimdBackend,
    T: ReconSample,
>(
    backend: B,
    u_block: CoeffBlock<'_>,
    u_prediction: &[T],
    v_block: CoeffBlock<'_>,
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
    let backend = backend.narrow();
    let u_setup = reconstruct_block_setup(
        u_prediction.len(),
        qindex,
        PlaneId::U,
        log2_width,
        log2_height,
        u_block.plane_tx_type(),
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
        u_block.plane_tx_type(),
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
        let [u_dequant, v_dequant] = dequantize_cctx_pair(
            backend,
            [u_block, v_block],
            [&u_setup, &v_setup],
            [&mut scratch.dequant, &mut scratch.dequant_pair],
            cctx_type,
            bit_depth,
        )?;
        let pairs = [
            (&u_setup, u_dequant, u_prediction, &mut *u_out),
            (&v_setup, v_dequant, v_prediction, &mut *v_out),
        ];
        for (setup, dequant, prediction, out) in pairs {
            if let Some(fused) = setup.fused()
                && fused.add_onto_with(
                    backend,
                    prediction,
                    dequant,
                    (None, bit_depth),
                    out,
                    1 << log2_width,
                    &mut scratch.transform,
                )
            {
                continue;
            }
            let residual = &mut scratch.residual[..setup.samples];
            inverse_transform_2d_outer_with_scratch_and_backend(
                backend,
                &setup.transform()?,
                dequant,
                residual,
                &mut scratch.transform[0],
            )?;
            reconstruct_add_residual_with(backend, prediction, residual, bit_depth, out)?;
        }
        Ok::<(), GeneralIntraResidualError>(())
    })?;
    Ok(())
}

#[expect(clippy::too_many_arguments)]
pub(crate) fn add_cctx_pair_residual_in_place_with<B: SimdBackend, T: ReconSample>(
    backend: B,
    surface: &mut CurrentFrameSurface<'_, '_, T>,
    scratch: &mut super::ResidualScratch,
    [u_block, v_block]: [CoeffBlock<'_>; 2],
    (x, y, size): (usize, usize, IntraRectBlockSize),
    qindex: u32,
    cctx_type: usize,
    use_ddt: bool,
    bit_depth: BitDepth,
) -> Result<(), GeneralIntraResidualError> {
    let backend = backend.narrow();
    let (log2_width, log2_height) = (size.log2_width().into(), size.log2_height().into());
    let setup = |plane, block: CoeffBlock<'_>| {
        reconstruct_block_setup(
            size.sample_count(),
            qindex,
            plane,
            log2_width,
            log2_height,
            u_block.plane_tx_type(),
            block.use_tcq,
            use_ddt,
            block.lossless,
            None,
            bit_depth,
        )
    };
    let (u_setup, v_setup) = (setup(PlaneId::U, u_block)?, setup(PlaneId::V, v_block)?);
    if u_setup.adjusted != v_setup.adjusted || u_setup.samples != v_setup.samples {
        return Err(GeneralIntraResidualError::InvalidReconstructionState {
            context: "CCTX paired transform geometry",
        });
    }
    let [u_dequant, v_dequant] = dequantize_cctx_pair(
        backend,
        [u_block, v_block],
        [&u_setup, &v_setup],
        [&mut scratch.dequant, &mut scratch.dequant_pair],
        cctx_type,
        bit_depth,
    )?;
    let pairs = [
        (PlaneId::U, &u_setup, u_dequant),
        (PlaneId::V, &v_setup, v_dequant),
    ];
    for (plane, setup, dequant) in pairs {
        #[cfg(feature = "block-evidence")]
        let capture = super::reconstruct::capture_residual(plane, x, y, size);
        #[cfg(not(feature = "block-evidence"))]
        let capture = false;
        if !capture
            && let Some(fused) = setup.fused()
            && let Some((dst, stride)) = surface.residual_rect_mut(plane, x, y, size)?
            && fused.add_with(
                backend,
                dequant,
                (None, bit_depth),
                dst,
                stride,
                &mut scratch.transform,
            )
        {
            continue;
        }
        let residual = &mut scratch.residual[..setup.samples];
        inverse_transform_2d_outer_with_scratch_and_backend(
            backend,
            &setup.transform()?,
            dequant,
            residual,
            &mut scratch.transform[0],
        )?;
        #[cfg(feature = "block-evidence")]
        if capture {
            super::reconstruct::record_residual(plane, x, y, size, residual, false);
        }
        surface.add_residual_rect_block_with(backend, plane, x, y, size, residual)?;
    }
    Ok(())
}

/// Dequantizes and rotates the common whole-row prefix. Missing coefficients
/// in the shorter plane are zero, as are both planes' omitted rows.
fn dequantize_cctx_pair<'a, B: SimdBackend>(
    backend: B,
    [u, v]: [CoeffBlock<'_>; 2],
    [u_setup, v_setup]: [&ReconstructBlockSetup; 2],
    [u_out, v_out]: [&'a mut [i32]; 2],
    cctx_type: usize,
    bit_depth: BitDepth,
) -> Result<[&'a [i32]; 2], GeneralIntraResidualError> {
    let u_written =
        dequantize_coeff_block_with(backend, u, &u_setup.params, &mut u_out[..u_setup.adjusted])?;
    let v_written =
        dequantize_coeff_block_with(backend, v, &v_setup.params, &mut v_out[..v_setup.adjusted])?;
    let coded = u_written.max(v_written);
    let (u, v) = (&mut u_out[..coded], &mut v_out[..coded]);
    u[u_written..].fill(0);
    v[v_written..].fill(0);
    apply_cross_chroma_transform(cctx_type, bit_depth, u, v)?;
    Ok([u, v])
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
    let Some(&[cos, sin]) = cctx_type
        .checked_sub(1)
        .and_then(|index| CCTX_MTX.get(index))
    else {
        return Err(GeneralIntraResidualError::InvalidReconstructionState {
            context: "CCTX type",
        });
    };
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

#[cfg(test)]
mod tests {
    use super::super::{DCT_DCT, LumaCoeffBlock};
    use super::*;

    #[test]
    fn cctx_rotation_writes_only_the_common_coded_prefix() {
        let quant = [3, -7, 5, 2, -2, 5, 11, 9, 1];
        for depth in [BitDepth::Eight, BitDepth::Ten] {
            let [u_setup, v_setup] = [PlaneId::U, PlaneId::V].map(|plane| {
                reconstruct_block_setup(
                    64, 99, plane, 3, 3, DCT_DCT, false, false, false, None, depth,
                )
                .unwrap()
            });
            for (u_len, v_len) in [(0, 0), (0, 9), (9, 0), (9, 4), (4, 9), (9, 9)] {
                let [u_block, v_block] = [u_len, v_len].map(|len| LumaCoeffBlock {
                    eob: u16::from(len != 0),
                    scan_order: false,
                    quant_start: 0,
                    quant_len: len as u16,
                    zero_tail: (64 - len) as u16,
                    intra_ist: None,
                    cctx_type: None,
                    plane_tx_type: DCT_DCT as u8,
                    use_tcq: false,
                    lossless: false,
                });
                let u = u_block.view(&quant[..u_len]);
                let v = v_block.view(&quant[..v_len]);
                let (mut want_u, mut want_v) = ([0; 64], [0; 64]);
                dequantize_coeff_block(u, &u_setup.params, &mut want_u).unwrap();
                dequantize_coeff_block(v, &v_setup.params, &mut want_v).unwrap();
                apply_cross_chroma_transform(5, depth, &mut want_u, &mut want_v).unwrap();

                let (mut actual_u, mut actual_v) = ([i32::MAX; 64], [i32::MIN; 64]);
                let got = crate::test_support::with_auto_backend!(backend => {
                    dequantize_cctx_pair(
                        backend,
                        [u, v],
                        [&u_setup, &v_setup],
                        [&mut actual_u, &mut actual_v],
                        5,
                        depth,
                    )
                })
                .unwrap();
                let coded = u_len.max(v_len).next_multiple_of(8);
                assert_eq!(got, [&want_u[..coded], &want_v[..coded]]);
                assert!(actual_u[coded..].iter().all(|&v| v == i32::MAX));
                assert!(actual_v[coded..].iter().all(|&v| v == i32::MIN));
            }
        }
    }
}
