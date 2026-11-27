// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Cross-chroma transform reconstruction helpers.

use splot_core::simd::SimdBackend;
use splot_recon::{
    BitDepth, CCTX_MTX, CurrentFrameSurface, IntraRectBlockSize, PlaneId, ReconSample,
    dequantize_cctx_pair_with, inverse_transform_2d_outer_with_scratch_and_backend,
    reconstruct_add_residual_with,
};

use super::reconstruct::{ReconstructBlockSetup, check_dense_len, reconstruct_block_setup};
use super::{CoeffBlock, GeneralIntraResidualError, with_residual_scratch};

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
    )?;
    let pairs = [
        (PlaneId::U, &u_setup, u_dequant),
        (PlaneId::V, &v_setup, v_dequant),
    ];
    for (plane, setup, dequant) in pairs {
        #[cfg(feature = "block-evidence")]
        let capture = super::reconstruct::capture_residual(plane, x, y, size);
        if let Some(fused) =
            residual_capture_path!(setup.fused(), setup.fused().filter(|_| !capture))
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

/// The § 7.14.6 dequantized pair: the whole rows that the longer block
/// reaches.
fn dequantize_cctx_pair<'a, B: SimdBackend>(
    backend: B,
    [u, v]: [CoeffBlock<'_>; 2],
    [u_setup, v_setup]: [&ReconstructBlockSetup; 2],
    [u_out, v_out]: [&'a mut [i32]; 2],
    cctx_type: usize,
) -> Result<[&'a [i32]; 2], GeneralIntraResidualError> {
    let Some(&rotation) = cctx_type
        .checked_sub(1)
        .and_then(|angle| CCTX_MTX.get(angle))
    else {
        return Err(GeneralIntraResidualError::InvalidReconstructionState {
            context: "CCTX type",
        });
    };
    let quant = [coded_quant(u, u_setup)?, coded_quant(v, v_setup)?];
    let (u_out, v_out) = (
        &mut u_out[..u_setup.adjusted],
        &mut v_out[..v_setup.adjusted],
    );
    let coded = dequantize_cctx_pair_with(
        backend,
        [&u_setup.params, &v_setup.params],
        quant,
        rotation,
        [&mut *u_out, &mut *v_out],
    )?;
    Ok([&u_out[..coded], &v_out[..coded]])
}

/// The coefficients that `block` codes: none without an end of block.
fn coded_quant<'a>(
    block: CoeffBlock<'a>,
    setup: &ReconstructBlockSetup,
) -> Result<&'a [i32], GeneralIntraResidualError> {
    if block.eob == 0 {
        return Ok(&[]);
    }
    check_dense_len(block, setup)?;
    Ok(block.quant)
}

#[cfg(test)]
mod tests {
    use super::super::{DCT_DCT, LumaCoeffBlock};
    use super::*;

    fn setups(depth: BitDepth) -> [ReconstructBlockSetup; 2] {
        [PlaneId::U, PlaneId::V].map(|plane| {
            reconstruct_block_setup(
                64, 99, plane, 3, 3, DCT_DCT, false, false, false, None, depth,
            )
            .unwrap()
        })
    }

    fn block(eob: u16, len: usize) -> LumaCoeffBlock {
        LumaCoeffBlock {
            eob,
            scan_order: false,
            #[cfg(feature = "block-evidence")]
            coding: crate::bitstream::tile_payload::CoefficientCoding::Ordinary,
            quant_start: 0,
            quant_len: len as u16,
            zero_tail: (64 - len) as u16,
            intra_ist: None,
            cctx_type: None,
            plane_tx_type: DCT_DCT as u8,
            use_tcq: false,
            lossless: false,
        }
    }

    fn pair<'a>(
        blocks: [CoeffBlock<'_>; 2],
        [u_setup, v_setup]: &[ReconstructBlockSetup; 2],
        out: [&'a mut [i32]; 2],
        cctx_type: usize,
    ) -> Result<[&'a [i32]; 2], GeneralIntraResidualError> {
        crate::test_support::with_auto_backend!(backend => {
            dequantize_cctx_pair(backend, blocks, [u_setup, v_setup], out, cctx_type)
        })
    }

    /// Which coefficients reach the kernel: a block without an end of block
    /// codes none, whatever its span holds. The arithmetic has its oracle
    /// test in `splot-recon`.
    #[test]
    fn cctx_pair_rotates_the_coded_rows_of_both_blocks() {
        let quant = [3, -7, 5, 2, -2, 5, 11, 9, 1];
        let setups = setups(BitDepth::Eight);
        let params = [&setups[0].params, &setups[1].params];
        for (u_len, v_len) in [(0, 0), (0, 9), (9, 0), (9, 4), (4, 9), (9, 9)] {
            for (u_eob, v_eob) in [(1, 1), (0, 1), (1, 0)] {
                let [u_block, v_block] = [block(u_eob, u_len), block(v_eob, v_len)];
                let coded = [(u_eob, u_len), (v_eob, v_len)]
                    .map(|(eob, len)| &quant[..if eob == 0 { 0 } else { len }]);
                let (mut want_u, mut want_v) = ([i32::MAX; 64], [i32::MIN; 64]);
                let written = crate::test_support::with_auto_backend!(backend => {
                    dequantize_cctx_pair_with(
                        backend,
                        params,
                        coded,
                        CCTX_MTX[4],
                        [&mut want_u, &mut want_v],
                    )
                })
                .unwrap();
                assert_eq!(
                    written,
                    coded[0].len().max(coded[1].len()).next_multiple_of(8)
                );

                let (mut actual_u, mut actual_v) = ([i32::MAX; 64], [i32::MIN; 64]);
                let got = pair(
                    [u_block.view(&quant[..u_len]), v_block.view(&quant[..v_len])],
                    &setups,
                    [&mut actual_u, &mut actual_v],
                    5,
                )
                .unwrap();
                assert_eq!(got, [&want_u[..written], &want_v[..written]]);
                assert_eq!((actual_u, actual_v), (want_u, want_v));
            }
        }
    }

    #[test]
    fn invalid_cctx_pair_state_is_rejected_before_any_write() {
        let quant = [11, 12, 21, 22];
        let setups = setups(BitDepth::Eight);
        let (mut u, mut v) = ([7; 64], [9; 64]);
        for cctx_type in [0, 7] {
            let block = block(1, 4);
            let result = pair(
                [block.view(&quant); 2],
                &setups,
                [&mut u, &mut v],
                cctx_type,
            );
            assert!(matches!(
                result,
                Err(GeneralIntraResidualError::InvalidReconstructionState {
                    context: "CCTX type"
                })
            ));
        }
        let short = LumaCoeffBlock {
            zero_tail: 59,
            ..block(1, 4)
        };
        let result = pair(
            [block(1, 4).view(&quant), short.view(&quant)],
            &setups,
            [&mut u, &mut v],
            1,
        );
        assert!(matches!(
            result,
            Err(GeneralIntraResidualError::QuantLength {
                expected: 64,
                actual: 63
            })
        ));
        assert_eq!((u, v), ([7; 64], [9; 64]));
    }
}
