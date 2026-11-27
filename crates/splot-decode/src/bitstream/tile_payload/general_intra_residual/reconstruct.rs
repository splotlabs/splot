// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Shared transform-block reconstruction setup.

#[cfg(feature = "block-evidence")]
use crate::block_evidence::{DecodeStage, stages};

use splot_core::simd::SimdBackend;
use splot_recon::math::round2_i32;
use splot_recon::{
    BitDepth, CurrentFrameSurface, CurrentFrameWorkspace, DequantBlockParams, DpcmDirection,
    FlatDequant, IntraRectBlockSize, InverseTransform2dOuter, InverseTransformAdd, PlaneId,
    ReconSample, SecondaryInverseTransform, coefficient_scan_slice, dequant_coefficient,
    dequantize_block_with, dequantize_with_secondary_and_backend,
    inverse_transform_2d_outer_adjusted_with, inverse_transform_2d_outer_with_scratch_and_backend,
    secondary_inverse_transform_scan_with, tx_class,
};

use super::{
    ADST_ADST, CoeffBlock, D67_PRED, D157_PRED, DCT_DCT, GeneralIntraResidualError, H_PRED,
    IST_4X4_HEIGHT, IST_8X8_HEIGHT, IST_8X8_HEIGHT_RED, LumaTransformTypeContext, SMOOTH_H_PRED,
    block_qm_level, current_quantizers, intra_secondary_transform_kernel,
    intra_secondary_transform_mode, invalid_reconstruction_state_error, resolve_block_qm,
};
use splot_recon::TransformClass;

pub(super) struct ReconstructBlockSetup {
    pub(super) adjusted: usize,
    pub(super) samples: usize,
    pub(super) params: DequantBlockParams,
    /// The [`InverseTransform2dOuter::resolve`] inputs, resolved only off the
    /// fused path.
    resolve: (usize, u32, u32, bool, bool, Option<DpcmDirection>),
}

impl ReconstructBlockSetup {
    /// The block's § 7.15.4 transform parameters.
    pub(super) fn transform(&self) -> splot_recon::Result<InverseTransform2dOuter> {
        let (plane_tx_type, log2_width, log2_height, use_ddt, lossless, dpcm) = self.resolve;
        InverseTransform2dOuter::resolve(
            plane_tx_type,
            log2_width,
            log2_height,
            use_ddt,
            lossless,
            self.params.bit_depth,
            dpcm,
        )
    }

    /// The fused transform and add of the block, when it has one.
    pub(super) fn fused(&self) -> Option<InverseTransformAdd> {
        let (plane_tx_type, log2_width, log2_height, use_ddt, lossless, dpcm) = self.resolve;
        if lossless || dpcm.is_some() || self.params.qm.is_some() {
            return None;
        }
        InverseTransformAdd::new(plane_tx_type, log2_width, log2_height, use_ddt)
    }
}

pub(crate) fn resolve_secondary_inverse_transform(
    block: CoeffBlock<'_>,
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
    } else if (tx_width == 8 && tx_height == 8) || block.plane_tx_type() == ADST_ADST {
        IST_8X8_HEIGHT_RED
    } else {
        IST_8X8_HEIGHT
    };
    let (kernel, transpose) = if let Some(luma_context) = luma_context {
        let most_probable_stx_set = ist
            .most_probable_stx_set
            .ok_or_else(|| invalid_reconstruction_state_error("active intra IST set"))?;
        let mode = intra_secondary_transform_mode(luma_context, tx_width, tx_height)?;
        (
            intra_secondary_transform_kernel(
                mode,
                block.plane_tx_type(),
                most_probable_stx_set.into(),
                tx_width,
                tx_height,
            )?,
            matches!(mode, H_PRED | D157_PRED | D67_PRED | SMOOTH_H_PRED),
        )
    } else {
        if block.plane_tx_type() != DCT_DCT
            || tx_width < 16
            || tx_height < 16
            || ist.most_probable_stx_set.is_some()
        {
            return Err(invalid_reconstruction_state_error("inter IST context"));
        }
        (0, false)
    };
    Ok(Some(SecondaryInverseTransform {
        w,
        h,
        n: match block.eob() {
            0 => n,
            eob => n.min(eob),
        },
        kernel,
        sec_tx_type: ist.sec_tx_type.into(),
        primary_scan_class: tx_class(block.plane_tx_type()),
        transpose,
        bit_depth,
    }))
}

#[cfg(test)]
#[expect(clippy::too_many_arguments)]
pub(crate) fn reconstruct_general_intra_coeff_block_rect_with_prediction_into<T: ReconSample>(
    block: CoeffBlock<'_>,
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
    crate::test_support::with_auto_backend!(backend => {
        reconstruct_general_intra_coeff_block_rect_with_prediction_into_with(
            backend,
            block,
            prediction,
            out,
            qindex,
            plane_id,
            log2_width,
            log2_height,
            use_tcq,
            luma_context,
            dpcm,
            bit_depth,
        )
    })
}

#[expect(clippy::too_many_arguments)]
pub(crate) fn reconstruct_general_intra_coeff_block_rect_with_prediction_into_with<
    B: SimdBackend,
    T: ReconSample,
>(
    backend: B,
    block: CoeffBlock<'_>,
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
    #[cfg(feature = "block-evidence")] capture_origin: Option<(usize, usize)>,
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
        backend.narrow(),
        block,
        prediction,
        out,
        qindex,
        plane_id,
        log2_width,
        log2_height,
        block.plane_tx_type(),
        use_tcq && block.use_tcq,
        false,
        block.lossless,
        secondary.as_ref(),
        dpcm,
        bit_depth,
        #[cfg(feature = "block-evidence")]
        capture_origin,
    )
}

/// Reconstructs one general-intra transform block straight into the current
/// frame, fusing the AV2 § 7.14.3 residual addition with the block's frame write.
///
/// This is the write-through form of
/// `reconstruct_general_intra_coeff_block_rect_with_prediction_into_with` followed
/// by `write_rect_block_with`: it drops the reconstructed-block staging buffer, its
/// fill, its range scan, and the block-to-plane copy. Returns `Ok(false)`
/// without touching the frame when the block cannot take the write-through path
/// — a block overhanging the frame edge, whose in-frame clamp only the buffered
/// write reproduces, or a caller bit depth other than the frame's, whose
/// out-of-range samples only the buffered write can reject — leaving the caller
/// to reconstruct it through the staging buffer.
#[expect(clippy::too_many_arguments)]
pub(crate) fn reconstruct_general_intra_coeff_block_rect_into_frame_with<
    B: SimdBackend,
    T: ReconSample,
>(
    backend: B,
    workspace: &mut CurrentFrameWorkspace<T>,
    block: CoeffBlock<'_>,
    prediction: &[T],
    plane_id: PlaneId,
    x: usize,
    y: usize,
    block_size: IntraRectBlockSize,
    qindex: u32,
    use_tcq: bool,
    luma_context: Option<LumaTransformTypeContext>,
    dpcm: Option<DpcmDirection>,
    bit_depth: BitDepth,
) -> Result<bool, GeneralIntraResidualError> {
    let backend = backend.narrow();
    if bit_depth != workspace.info().bit_depth() {
        return Ok(false);
    }
    let adjusted = block_size.width().min(32) * block_size.height().min(32);
    if block.dense_len() != adjusted {
        return Err(GeneralIntraResidualError::QuantLength {
            expected: adjusted,
            actual: block.dense_len(),
        });
    }
    let capture = capture_residual(plane_id, x, y, block_size);
    let fused = fused_intra_setup(
        block,
        plane_id,
        block_size,
        qindex,
        use_tcq,
        luma_context,
        dpcm,
        bit_depth,
    );
    let general = || {
        intra_residual_setup(
            block,
            plane_id,
            block_size,
            qindex,
            use_tcq,
            luma_context,
            dpcm,
            bit_depth,
        )
    };
    let written = workspace.with_rect_block_rows_mut(plane_id, x, y, block_size, |rows| {
        super::with_residual_scratch(|scratch| {
            let (dst, stride) = rows.prediction_target();
            if !capture
                && let Some((fused, flat)) = fused
                && fused.add_onto_with(
                    backend,
                    prediction,
                    block.quant,
                    (Some(flat), bit_depth),
                    dst,
                    stride,
                    &mut scratch.transform,
                )
            {
                return Ok(());
            }
            if !capture
                && let Some(luma_context) = luma_context
                && has_secondary(block)
                && add_secondary_fused(
                    backend,
                    block,
                    (block_size, PlaneId::Y, bit_depth),
                    (qindex, use_tcq && block.use_tcq, false),
                    Some(luma_context),
                    (Some(prediction), dst, stride),
                    scratch,
                )?
            {
                return Ok(());
            }
            let (setup, secondary) = general()?;
            let residual = intra_residual(backend, block, &setup, secondary.as_ref(), scratch)
                .map_err(reconstruct_error)?;
            #[cfg(feature = "block-evidence")]
            if capture {
                record_residual(plane_id, x, y, block_size, residual, false);
            }
            rows.add_block_residual_with(backend, prediction, residual)
                .map_err(reconstruct_error)
        })
    })?;
    Ok(written.is_some())
}

const fn reconstruct_error(source: splot_recon::ReconError) -> GeneralIntraResidualError {
    GeneralIntraResidualError::Reconstruct { source }
}

/// The fused transform and add of an intra block (see
/// [`fused_residual_setup`]).
#[expect(clippy::too_many_arguments)]
fn fused_intra_setup(
    block: CoeffBlock<'_>,
    plane_id: PlaneId,
    block_size: IntraRectBlockSize,
    qindex: u32,
    use_tcq: bool,
    luma_context: Option<LumaTransformTypeContext>,
    dpcm: Option<DpcmDirection>,
    bit_depth: BitDepth,
) -> Option<(InverseTransformAdd, FlatDequant)> {
    let (plane_id, dpcm) = match luma_context {
        Some(luma_context) => (PlaneId::Y, luma_context.dpcm),
        None => (plane_id, dpcm),
    };
    fused_residual_setup(
        block,
        block_size,
        qindex,
        plane_id,
        use_tcq && block.use_tcq,
        false,
        dpcm,
        bit_depth,
    )
}

/// The fused transform and add of a block without a secondary transform,
/// with its flat dequantization; `None` sends the block down the general
/// path, which also reports a block of the wrong length.
#[expect(clippy::too_many_arguments)]
#[inline]
fn fused_residual_setup(
    block: CoeffBlock<'_>,
    block_size: IntraRectBlockSize,
    qindex: u32,
    plane_id: PlaneId,
    use_tcq: bool,
    use_ddt: bool,
    dpcm: Option<DpcmDirection>,
    bit_depth: BitDepth,
) -> Option<(InverseTransformAdd, FlatDequant)> {
    if block.lossless || dpcm.is_some() || has_secondary(block) {
        return None;
    }
    let fused = fused_transform(block, block_size, plane_id, use_ddt)?;
    let plane_tx_type = block.plane_tx_type();
    let (dc_quant, ac_quant) = current_quantizers(plane_id, qindex, bit_depth);
    let dq_denom = dq_denom(block_size.sample_count(), use_tcq, plane_tx_type);
    FlatDequant::new(dc_quant, ac_quant, dq_denom).map(|flat| (fused, flat))
}

/// Whether `block` codes a secondary transform.
#[inline]
fn has_secondary(block: CoeffBlock<'_>) -> bool {
    block.intra_ist.is_some_and(|ist| ist.sec_tx_type != 0)
}

/// Adds the residual of a block of `plane_id` with a secondary transform
/// onto the block at the start of `dst` through the fused transform, its
/// coefficients dequantized through the secondary transform first, after
/// writing `prediction` there when it is given; `luma_context` holds its
/// intra luma context, `None` for an inter block. `false` with `dst`
/// untouched sends the block down the general path.
#[inline(never)]
fn add_secondary_fused<B: SimdBackend, T: ReconSample>(
    backend: B,
    block: CoeffBlock<'_>,
    (block_size, plane_id, bit_depth): (IntraRectBlockSize, PlaneId, BitDepth),
    (qindex, use_tcq, use_ddt): (u32, bool, bool),
    luma_context: Option<LumaTransformTypeContext>,
    (prediction, dst, stride): (Option<&[T]>, &mut [T], usize),
    scratch: &mut super::ResidualScratch,
) -> Result<bool, GeneralIntraResidualError> {
    if block.lossless || luma_context.is_some_and(|context| context.dpcm.is_some()) {
        return Ok(false);
    }
    let Some(fused) = fused_transform(block, block_size, plane_id, use_ddt) else {
        return Ok(false);
    };
    let (log2_width, log2_height) = (
        u32::from(block_size.log2_width()),
        u32::from(block_size.log2_height()),
    );
    let Some(secondary) = resolve_secondary_inverse_transform(
        block,
        log2_width,
        log2_height,
        bit_depth,
        luma_context,
    )?
    else {
        return Ok(false);
    };
    let (dc_quant, ac_quant) = current_quantizers(plane_id, qindex, bit_depth);
    let samples = block_size.sample_count();
    let dq_denom = dq_denom(samples, use_tcq, block.plane_tx_type());
    if let Some(prediction) = prediction
        && !fused.put_prediction_with(backend, prediction, bit_depth, dst, stride)
    {
        return Ok(false);
    }
    let quant = if block.scan_order {
        splot_recon::SecondaryQuant::Scan(block.quant)
    } else {
        splot_recon::SecondaryQuant::Raster(block.quant)
    };
    fused
        .add_secondary_quant_with(
            backend,
            ((dc_quant, ac_quant, dq_denom), &secondary),
            quant,
            &mut scratch.dequant[..samples],
            (dst, stride),
            &mut scratch.transform,
        )
        .map_err(reconstruct_error)
}

/// The fused transform of a block of `plane_id`; `None` for a shape
/// it does not cover, a block of the wrong length or a block weighted by a
/// quantization matrix.
#[inline]
fn fused_transform(
    block: CoeffBlock<'_>,
    block_size: IntraRectBlockSize,
    plane_id: PlaneId,
    use_ddt: bool,
) -> Option<InverseTransformAdd> {
    let plane_tx_type = block.plane_tx_type();
    let (log2_w, log2_h) = (block_size.log2_width(), block_size.log2_height());
    let fused = InverseTransformAdd::new(plane_tx_type, log2_w.into(), log2_h.into(), use_ddt)?;
    let (w, h) = (block_size.width(), block_size.height());
    let flat = block_qm_level(plane_id, plane_tx_type, w, h).is_none();
    (block.dense_len() == w * h && flat).then_some(fused)
}

/// The § 7.14.4 `dq_denom` of a block of `pels` samples.
fn dq_denom(pels: usize, use_tcq: bool, plane_tx_type: usize) -> u32 {
    let tcq_two_d = use_tcq && splot_recon::tx_class(plane_tx_type) == TransformClass::TwoD;
    1 << (u32::from(pels > 256) + u32::from(pels > 1024) + u32::from(tcq_two_d))
}

/// Adds an intra block's residual onto the prediction already written into
/// the frame rows, the destination inter residuals are added into as well.
#[expect(clippy::too_many_arguments)]
pub(crate) fn add_general_intra_coeff_block_residual_in_place_with<
    B: SimdBackend,
    T: ReconSample,
>(
    backend: B,
    workspace: &mut CurrentFrameWorkspace<T>,
    block: CoeffBlock<'_>,
    plane_id: PlaneId,
    x: usize,
    y: usize,
    block_size: IntraRectBlockSize,
    qindex: u32,
    use_tcq: bool,
    luma_context: Option<LumaTransformTypeContext>,
    dpcm: Option<DpcmDirection>,
    bit_depth: BitDepth,
) -> Result<(), GeneralIntraResidualError> {
    let backend = backend.narrow();
    let capture = capture_residual(plane_id, x, y, block_size);
    let fused = fused_intra_setup(
        block,
        plane_id,
        block_size,
        qindex,
        use_tcq,
        luma_context,
        dpcm,
        bit_depth,
    );
    let general = || {
        intra_residual_setup(
            block,
            plane_id,
            block_size,
            qindex,
            use_tcq,
            luma_context,
            dpcm,
            bit_depth,
        )
    };
    super::with_residual_scratch(|scratch| {
        let mut surface = splot_recon::CurrentFrameSurface::Frame(&mut *workspace);
        if !capture
            && let Some((fused, flat)) = fused
            && let Some((dst, stride)) = surface
                .residual_rect_mut(plane_id, x, y, block_size)
                .map_err(reconstruct_error)?
            && fused.add_with(
                backend,
                block.quant,
                (Some(flat), bit_depth),
                dst,
                stride,
                &mut scratch.transform,
            )
        {
            return Ok(());
        }
        if !capture
            && let Some(luma_context) = luma_context
            && has_secondary(block)
            && let Some((dst, stride)) = surface
                .residual_rect_mut(plane_id, x, y, block_size)
                .map_err(reconstruct_error)?
            && add_secondary_fused::<_, T>(
                backend,
                block,
                (block_size, PlaneId::Y, bit_depth),
                (qindex, use_tcq && block.use_tcq, false),
                Some(luma_context),
                (None, dst, stride),
                scratch,
            )?
        {
            return Ok(());
        }
        let (setup, secondary) = general()?;
        let residual = intra_residual(backend, block, &setup, secondary.as_ref(), scratch)
            .map_err(reconstruct_error)?;
        #[cfg(feature = "block-evidence")]
        if capture {
            record_residual(plane_id, x, y, block_size, residual, false);
        }
        surface
            .add_residual_rect_block_with(backend, plane_id, x, y, block_size, residual)
            .map_err(reconstruct_error)
    })
}

#[expect(clippy::too_many_arguments)]
#[inline]
fn intra_residual_setup(
    block: CoeffBlock<'_>,
    plane_id: PlaneId,
    block_size: IntraRectBlockSize,
    qindex: u32,
    use_tcq: bool,
    luma_context: Option<LumaTransformTypeContext>,
    dpcm: Option<DpcmDirection>,
    bit_depth: BitDepth,
) -> Result<(ReconstructBlockSetup, Option<SecondaryInverseTransform>), GeneralIntraResidualError> {
    let log2_width = u32::from(block_size.log2_width());
    let log2_height = u32::from(block_size.log2_height());
    let (quant_plane, dpcm, secondary) = if let Some(luma_context) = luma_context {
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
    let setup = reconstruct_block_setup(
        block_size.sample_count(),
        qindex,
        quant_plane,
        log2_width,
        log2_height,
        block.plane_tx_type(),
        use_tcq && block.use_tcq,
        false,
        block.lossless,
        dpcm,
        bit_depth,
    )?;
    check_dense_len(block, &setup)?;
    Ok((setup, secondary))
}

/// Borrows raster coefficients or scatters a packed IST prefix into the
/// caller's reusable scratch for the general reconstruction path.
fn raster_quant<'a>(
    block: CoeffBlock<'a>,
    params: &DequantBlockParams,
    scratch: &'a mut [i32],
) -> splot_recon::Result<&'a [i32]> {
    if !block.scan_order {
        return Ok(block.quant);
    }
    let scan = coefficient_scan_slice(
        params.tx_width,
        params.tx_height,
        tx_class(block.plane_tx_type()),
    )?;
    let scan = scan.get(..block.quant.len()).ok_or(
        splot_recon::ReconError::SecondaryTransformBufferMismatch {
            expected: scan.len(),
            actual: block.quant.len(),
        },
    )?;
    let extent = scan.iter().max().map_or(0, |&pos| usize::from(pos) + 1);
    let actual = scratch.len();
    let raster = scratch.get_mut(..extent).ok_or(
        splot_recon::ReconError::SecondaryTransformBufferMismatch {
            expected: extent,
            actual,
        },
    )?;
    raster.fill(0);
    for (&pos, &value) in scan.iter().zip(block.quant) {
        raster[usize::from(pos)] = value;
    }
    Ok(raster)
}

fn dequantize_residual<B: SimdBackend>(
    backend: B,
    block: CoeffBlock<'_>,
    params: &DequantBlockParams,
    secondary: Option<&SecondaryInverseTransform>,
    raster: &mut [i32],
    out: &mut [i32],
) -> splot_recon::Result<usize> {
    if block.scan_order
        && params.qm.is_none()
        && let Some(secondary) = secondary
        && (params.tx_width, params.tx_height) == (secondary.w, secondary.h)
    {
        return secondary_inverse_transform_scan_with(
            backend,
            &block.quant[..block.quant.len().min(secondary.n)],
            (params.dc_quant, params.ac_quant, params.dq_denom),
            secondary,
            out,
        );
    }
    let quant = raster_quant(block, params, raster)?;
    dequantize_with_secondary_and_backend(backend, params, quant, secondary, out)
}

/// Dequantizes and inverse-transforms one intra block into `scratch`.
pub(super) fn intra_residual<'s, B: SimdBackend>(
    backend: B,
    block: CoeffBlock<'_>,
    setup: &ReconstructBlockSetup,
    secondary: Option<&SecondaryInverseTransform>,
    scratch: &'s mut super::ResidualScratch,
) -> splot_recon::Result<&'s [i32]> {
    let dequant = &mut scratch.dequant[..setup.adjusted];
    let coded = dequantize_residual(
        backend,
        block,
        &setup.params,
        secondary,
        &mut scratch.dequant_pair,
        dequant,
    )?;
    let residual = &mut scratch.residual[..setup.samples];
    inverse_transform_2d_outer_with_scratch_and_backend(
        backend,
        &setup.transform()?,
        &dequant[..coded],
        residual,
        &mut scratch.dequant_pair,
    )?;
    Ok(residual)
}

#[expect(clippy::too_many_arguments)]
pub(crate) fn reconstruct_inter_coeff_block_residual_rect_into_with<
    B: SimdBackend,
    T: ReconSample,
>(
    backend: B,
    sink: &mut CurrentFrameSurface<'_, '_, T>,
    scratch: &mut super::ResidualScratch,
    block: CoeffBlock<'_>,
    plane_id: PlaneId,
    x: usize,
    y: usize,
    block_size: IntraRectBlockSize,
    qindex: u32,
    use_tcq: bool,
    use_ddt: bool,
    bit_depth: BitDepth,
) -> Result<(), GeneralIntraResidualError> {
    let backend = backend.narrow();
    let capture = capture_residual(plane_id, x, y, block_size);
    let log2_width = u32::from(block_size.log2_width());
    let log2_height = u32::from(block_size.log2_height());
    let use_block_tcq = use_tcq && block.use_tcq;
    let fused = fused_residual_setup(
        block,
        block_size,
        qindex,
        plane_id,
        use_block_tcq,
        use_ddt,
        None,
        bit_depth,
    );
    if !capture && let Some((fused, flat)) = fused {
        if let Some(residual) = fused.flat_residual(block.quant, Some(flat), bit_depth) {
            sink.add_constant_residual_rect_block_with(
                backend, plane_id, x, y, block_size, residual,
            )?;
            return Ok(());
        }
        let added = sink
            .residual_rect_mut(plane_id, x, y, block_size)?
            .is_some_and(|(dst, stride)| {
                fused.add_with(
                    backend,
                    block.quant,
                    (Some(flat), bit_depth),
                    dst,
                    stride,
                    &mut scratch.transform,
                )
            });
        if added {
            return Ok(());
        }
    }
    if !capture
        && has_secondary(block)
        && let Some((dst, stride)) = sink.residual_rect_mut(plane_id, x, y, block_size)?
        && add_secondary_fused::<_, T>(
            backend,
            block,
            (block_size, plane_id, bit_depth),
            (qindex, use_block_tcq, use_ddt),
            None,
            (None, dst, stride),
            scratch,
        )?
    {
        return Ok(());
    }
    let secondary =
        resolve_secondary_inverse_transform(block, log2_width, log2_height, bit_depth, None)?;
    let setup = reconstruct_block_setup(
        block_size.sample_count(),
        qindex,
        plane_id,
        log2_width,
        log2_height,
        block.plane_tx_type(),
        use_block_tcq,
        use_ddt,
        block.lossless,
        None,
        bit_depth,
    )?;
    check_dense_len(block, &setup)?;
    if block.eob == 1
        && block.plane_tx_type() == DCT_DCT
        && !block.lossless
        && secondary.is_none()
        && setup.params.qm.is_none()
    {
        let transform = setup
            .transform()
            .map_err(|source| GeneralIntraResidualError::Reconstruct { source })?;
        let residual =
            dct_dc_residual(block.quant.first().copied().unwrap_or(0), &setup, transform);
        #[cfg(feature = "block-evidence")]
        if capture {
            stages::transform_samples(
                DecodeStage::Residual,
                plane_id,
                stages::block_rect(x, y, block_size),
                true,
                |_, _| Some(residual),
            );
        }
        sink.add_constant_residual_rect_block_with(backend, plane_id, x, y, block_size, residual)?;
        return Ok(());
    }
    let dequant = &mut scratch.dequant[..setup.adjusted];
    let coded = dequantize_residual(
        backend,
        block,
        &setup.params,
        secondary.as_ref(),
        &mut scratch.dequant_pair,
        dequant,
    )?;
    let residual = &mut scratch.residual[..setup.adjusted];
    inverse_transform_2d_outer_adjusted_with(
        backend,
        &setup.transform()?,
        &dequant[..coded],
        residual,
        &mut scratch.dequant_pair,
    )?;
    #[cfg(feature = "block-evidence")]
    if capture {
        record_residual(plane_id, x, y, block_size, residual, true);
    }
    sink.add_adjusted_residual_rect_block_with(backend, plane_id, x, y, block_size, residual)?;
    Ok(())
}

#[inline]
pub(super) fn capture_residual(
    plane: PlaneId,
    x: usize,
    y: usize,
    size: IntraRectBlockSize,
) -> bool {
    #[cfg(feature = "block-evidence")]
    {
        stages::captures_block(plane, stages::block_rect(x, y, size))
    }
    #[cfg(not(feature = "block-evidence"))]
    {
        let _ = (plane, x, y, size);
        false
    }
}

#[cfg(feature = "block-evidence")]
pub(super) fn record_residual(
    plane: PlaneId,
    x: usize,
    y: usize,
    size: IntraRectBlockSize,
    residual: &[i32],
    adjusted: bool,
) {
    let width = size.width();
    let shift_x = usize::from(adjusted && width == 64);
    let shift_y = usize::from(adjusted && size.height() == 64);
    stages::transform_samples(
        DecodeStage::Residual,
        plane,
        stages::block_rect(x, y, size),
        true,
        |x, y| {
            residual
                .get((y >> shift_y) * (width >> shift_x) + (x >> shift_x))
                .copied()
        },
    );
}

fn check_dense_len(
    block: CoeffBlock<'_>,
    setup: &ReconstructBlockSetup,
) -> Result<(), GeneralIntraResidualError> {
    if block.dense_len() != setup.adjusted {
        return Err(GeneralIntraResidualError::QuantLength {
            expected: setup.adjusted,
            actual: block.dense_len(),
        });
    }
    Ok(())
}

#[inline]
fn dct_dc_residual(
    quant: i32,
    setup: &ReconstructBlockSetup,
    transform: InverseTransform2dOuter,
) -> i32 {
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
        return Err(invalid_reconstruction_state_error(
            "intra IST transform shape",
        ));
    }
    Ok(1usize << log2_dim)
}

#[expect(clippy::too_many_arguments)]
#[inline]
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
    let dq_denom = dq_denom(samples, use_tcq, plane_tx_type);
    let (dc_quant, ac_quant) = current_quantizers(plane_id, qindex, bit_depth);
    let qm = resolve_block_qm(
        plane_id,
        plane_tx_type,
        adj_w,
        adj_h,
        log2_width,
        log2_height,
    );
    let params = DequantBlockParams {
        dc_quant,
        ac_quant,
        tx_width: adj_w,
        tx_height: adj_h,
        dq_denom,
        bit_depth,
        qm,
    };
    Ok(ReconstructBlockSetup {
        adjusted,
        samples,
        params,
        resolve: (
            plane_tx_type,
            log2_width,
            log2_height,
            use_ddt,
            lossless,
            dpcm,
        ),
    })
}

/// Dequantizes the whole-row prefix of `block`, returning the number of
/// samples it wrote and leaving the rest of `out` untouched.
pub(super) fn dequantize_coeff_block_with<B: SimdBackend>(
    backend: B,
    block: CoeffBlock<'_>,
    params: &DequantBlockParams,
    out: &mut [i32],
) -> Result<usize, GeneralIntraResidualError> {
    if block.eob == 0 {
        return Ok(0);
    }
    if block.dense_len() != out.len() {
        return Err(GeneralIntraResidualError::QuantLength {
            expected: out.len(),
            actual: block.dense_len(),
        });
    }
    dequantize_block_with(backend, params, block.quant, out).map_err(Into::into)
}

#[cfg(test)]
pub(super) fn dequantize_coeff_block(
    block: CoeffBlock<'_>,
    params: &DequantBlockParams,
    out: &mut [i32],
) -> Result<usize, GeneralIntraResidualError> {
    crate::test_support::with_auto_backend!(backend => {
        dequantize_coeff_block_with(backend, block, params, out)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use splot_recon::{dequantize_block, inverse_transform_2d_outer};

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
                        inverse_transform_2d_outer(
                            &setup.transform().unwrap(),
                            &dequant,
                            &mut residual,
                        )
                        .unwrap();
                        let dc = dct_dc_residual(quant_dc, &setup, setup.transform().unwrap());
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
