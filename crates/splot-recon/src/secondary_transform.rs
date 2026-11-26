// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 § 7.15.3 secondary (inverse) transform process.
//!
//! This module implements the scheduler-free matrix-based secondary transform
//! ([`07-decoding-process.md`](../../../docs/spec/av2/1.0.0/07-decoding-process.md)
//! `#s-7-15-3`) that runs over the dequantized coefficients before the § 7.15.4
//! 2D inverse transform when `sec_tx_type != 0`. It gathers the first `n`
//! coefficients in the caller-resolved primary scan order, multiplies them by
//! the § 9.7 IST kernel, and scatters the `Round2Signed` / `Clip3` results back
//! into the top-left scan sub-block of `Dequant`.
//!
//! Feature tracking: `RECON-SECONDARY-INVERSE-TRANSFORM`.

use fearless_simd::{Bytes as _, Simd, SimdBase as _, SimdIntWiden as _, i16x16, i32x8, u32x8};
#[cfg(target_arch = "aarch64")]
use fearless_simd::{SimdCombine as _, i16x8, i32x4};

use splot_core::{simd::SimdBackend, with_simd};
use splot_tables::tables::secondary_transform::{IST_4X4_KERNEL, IST_8X8_KERNEL, STX_SCAN_MAP};

use crate::coefficient_scan::TransformClass;
use crate::{BitDepth, ReconError, Result};

/// AV2 § 3 `IST_4X4_HEIGHT`: rows in the 4x4 secondary-transform matrix.
const IST_4X4_HEIGHT: usize = 8;
/// AV2 § 3 `IST_4X4_WIDTH`: columns in the 4x4 secondary-transform matrix.
const IST_4X4_WIDTH: usize = 16;
/// AV2 § 3 `IST_8X8_HEIGHT`: rows in the 8x8 secondary-transform matrix.
const IST_8X8_HEIGHT: usize = 32;
/// AV2 § 3 `IST_8X8_WIDTH`: columns in the 8x8 secondary-transform matrix.
const IST_8X8_WIDTH: usize = 48;
/// AV2 § 3 `IST_SET_SIZE_4X4`: number of 4x4 secondary-transform kernel sets.
const IST_SET_SIZE_4X4: usize = 14;
/// AV2 § 3 `IST_SET_SIZE_8X8`: number of 8x8 secondary-transform kernel sets.
const IST_SET_SIZE_8X8: usize = 11;
/// AV2 § 3 `STX_TYPES`: number of secondary transform types (`sec_tx_type`
/// `0..=3`; `0` is "no secondary transform", and types `1..=3` index the kernel
/// at `sec_tx_type - 1`).
const STX_TYPES: usize = 4;

/// AV2 § 7.15.3 `Stx_Scan_Order_4x4[IST_4X4_WIDTH]`, transcribed verbatim from the
/// spec process body (`07-decoding-process.md#s-7-15-3`). It is a § 7.15.3
/// process-body constant absent from the generated `all_tables.h`, so it is a
/// hand-written, spec-cited constant.
const STX_SCAN_ORDER_4X4: [usize; IST_4X4_WIDTH] =
    [0, 1, 4, 8, 5, 2, 3, 6, 9, 12, 13, 10, 7, 11, 14, 15];

/// AV2 § 7.15.3 `Stx_Scan_Order_8x8[64]`, transcribed verbatim from the spec
/// process body (`07-decoding-process.md#s-7-15-3`). Like `Stx_Scan_Order_4x4`,
/// it is a hand-written, spec-cited process-body constant.
const STX_SCAN_ORDER_8X8: [usize; 64] = [
    0, 1, 8, 16, 9, 2, 3, 10, 17, 24, 32, 25, 18, 11, 4, 5, 12, 19, 26, 33, 40, 48, 41, 34, 27, 20,
    13, 6, 7, 14, 21, 28, 35, 42, 49, 56, 57, 50, 43, 36, 29, 22, 15, 23, 30, 37, 44, 51, 58, 59,
    52, 45, 38, 31, 39, 46, 53, 60, 61, 54, 47, 55, 62, 63,
];

/// Caller-resolved parameters for the AV2 § 7.15.3 secondary transform.
///
/// `w` / `h` are the operating dimensions `Min(32, Tx_Width[txSz])` /
/// `Min(32, Tx_Height[txSz])` (each a power of two in `4..=32`); `large` (whether
/// `w >= 8 && h >= 8`), `bwl` (`Min(5, Tx_Width_Log2[txSz])`, i.e. `log2(w)`), and
/// the output width are derived internally. `n` is the § 7.15.3 input
/// coefficient count (`IST_4X4_HEIGHT`, `IST_8X8_HEIGHT`, or the reduced
/// `IST_8X8_HEIGHT_RED`), or a block's smaller end of block, past which the
/// § 7.15.3 inputs are zero; `kernel` and `sec_tx_type` (`1..=3`) select the § 9.7
/// IST kernel; `primary_scan_class` aligns the IST input support with the
/// primary transform scan; `transpose` chooses the § 7.15.3 output layout;
/// `bit_depth` bounds the `Clip3`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SecondaryInverseTransform {
    /// Operating width `Min(32, Tx_Width[txSz])` (a power of two in `4..=32`).
    pub w: usize,
    /// Operating height `Min(32, Tx_Height[txSz])` (a power of two in `4..=32`).
    pub h: usize,
    /// § 7.15.3 input coefficient count (`n`), at most the kernel height.
    pub n: usize,
    /// § 9.7 IST kernel-set index (`kernel`).
    pub kernel: usize,
    /// § 7.15.3 secondary transform type (`sec_tx_type`, `1..=3`).
    pub sec_tx_type: usize,
    /// AVM-compatible input scan class for the primary transform.
    pub primary_scan_class: TransformClass,
    /// § 7.15.3 output transpose flag.
    pub transpose: bool,
    /// Active decoded bit depth (bounds the `Clip3`).
    pub bit_depth: BitDepth,
}

/// The § 7.15.3 geometry of one secondary transform, shared by the inverse
/// and an encoder's forward transform: the row-major block positions of the
/// `n` inputs (the first `n` of the primary scan), the block position of each
/// of the `width` outputs, and the `n` kernel rows.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SecondaryTransformLayout {
    /// § 7.15.3 `n`.
    pub n: usize,
    /// Block positions of the `n` inputs.
    pub inputs: [u16; IST_8X8_HEIGHT],
    /// Kernel width: `IST_4X4_WIDTH` or `IST_8X8_WIDTH`.
    pub width: usize,
    /// Block position of each of the `width` outputs.
    pub outputs: [u16; IST_8X8_WIDTH],
    /// The `n` kernel rows of `width` weights each, row-major.
    pub kernel: &'static [i8],
    /// The `Clip3` bound `1 << (BitDepth + 7)`.
    pub bound: i32,
}

/// The § 7.15.3 layout of `params`.
///
/// # Errors
/// Returns [`ReconError::SecondaryTransformInvalidShape`] if `w` / `h` are not
/// powers of two in `4..=32`, and [`ReconError::SecondaryTransformInvalidParams`]
/// if `n`, `kernel`, or `sec_tx_type` are out of range for the kernel set.
pub fn secondary_transform_layout(
    params: &SecondaryInverseTransform,
) -> Result<SecondaryTransformLayout> {
    let SecondaryInverseTransform {
        w,
        h,
        n,
        kernel,
        sec_tx_type,
        transpose,
        ..
    } = *params;
    let (scan, bound) = validated(params, w.wrapping_mul(h), 0)?;
    let large = w >= 8 && h >= 8;
    let width = if large { IST_8X8_WIDTH } else { IST_4X4_WIDTH };
    let invalid = || ReconError::SecondaryTransformInvalidParams {
        n,
        kernel,
        sec_tx_type,
    };
    let stx = sec_tx_type - 1;
    let mut inputs = [0u16; IST_8X8_HEIGHT];
    for (slot, &pos) in inputs[..n].iter_mut().zip(scan.iter()) {
        *slot = pos;
    }
    let (scan_bwl, scan_w) = if large {
        (3u32, 8usize)
    } else {
        (2u32, 4usize)
    };
    let mut outputs = [0u16; IST_8X8_WIDTH];
    for (i, slot) in outputs[..width].iter_mut().enumerate() {
        let pos = if large {
            let mapped = usize::try_from(STX_SCAN_MAP[kernel][stx][i]).map_err(|_| invalid())?;
            STX_SCAN_ORDER_8X8[mapped]
        } else {
            STX_SCAN_ORDER_4X4[i]
        };
        let (x, y) = (pos & (scan_w - 1), pos >> scan_bwl);
        let index = if transpose { x * w + y } else { y * w + x };
        *slot = u16::try_from(index).map_err(|_| invalid())?;
    }
    let kernel = if large {
        &IST_8X8_KERNEL[kernel][stx].as_flattened()[..n * width]
    } else {
        &IST_4X4_KERNEL[kernel][stx].as_flattened()[..n * width]
    };
    Ok(SecondaryTransformLayout {
        n,
        inputs,
        width,
        outputs,
        kernel,
        bound,
    })
}

/// Applies the AV2 § 7.15.3 secondary inverse transform to the dequantized block
/// `dequant` (a `w * h` row-major array), modifying it in place.
///
/// The process gathers the first `params.n` coefficients of `dequant` in
/// § 5.20.7.30 2D scan order, multiplies the gathered vector by the § 9.7 IST
/// kernel (`Ist_4x4_Kernel` or `Ist_8x8_Kernel` selected by
/// `large = w >= 8 && h >= 8`), clears the dequantized coefficient block as
/// AVM does before writeback, applies `Round2Signed(t, 7)` and the
/// `Clip3(±(1 << (BitDepth + 7)))` bound, and scatters the results into the
/// top-left scan sub-block ([`secondary_transform_layout`]).
///
/// The computation is total and panic-free for valid inputs: the accumulation
/// uses `i32` (at most 32 products of clipped coefficients and kernels below
/// 128, fitting below 2^30), and every table index is validated before use.
///
/// # Errors
/// Returns the [`secondary_transform_layout`] errors, and
/// [`ReconError::SecondaryTransformBufferMismatch`] if `dequant` is not
/// `w * h`. All inputs are validated before any coefficient is modified.
pub fn secondary_inverse_transform(
    dequant: &mut [i32],
    params: &SecondaryInverseTransform,
) -> Result<()> {
    let coded = dequant.len();
    secondary_inverse_transform_prefix(dequant, coded, params).map(|_| ())
}

/// Typed [`secondary_inverse_transform`] using caller-selected `backend`.
///
/// # Errors
/// Returns the same shape, parameter, and buffer errors as [`secondary_inverse_transform`].
pub fn secondary_inverse_transform_with<B: SimdBackend>(
    backend: B,
    dequant: &mut [i32],
    params: &SecondaryInverseTransform,
) -> Result<()> {
    let coded = dequant.len();
    secondary_inverse_transform_prefix_with(backend, dequant, coded, params).map(|_| ())
}

/// [`secondary_inverse_transform`] of a block whose samples from `coded` on
/// are zero and may be unwritten: zeroes only the whole-row prefix that may
/// be non-zero afterwards, and returns its length.
pub(crate) fn secondary_inverse_transform_prefix(
    dequant: &mut [i32],
    coded: usize,
    params: &SecondaryInverseTransform,
) -> Result<usize> {
    secondary_inverse_transform_prefix_with_runner(BaselineIst, dequant, coded, params)
}

/// Typed [`secondary_inverse_transform_prefix`] using caller-selected `backend`.
pub(crate) fn secondary_inverse_transform_prefix_with<B: SimdBackend>(
    backend: B,
    dequant: &mut [i32],
    coded: usize,
    params: &SecondaryInverseTransform,
) -> Result<usize> {
    secondary_inverse_transform_prefix_with_runner(
        TypedIst(backend.narrow()),
        dequant,
        coded,
        params,
    )
}

fn secondary_inverse_transform_prefix_with_runner<R: IstRunner>(
    runner: R,
    dequant: &mut [i32],
    coded: usize,
    params: &SecondaryInverseTransform,
) -> Result<usize> {
    let (scan, bound) = validated(params, dequant.len(), coded)?;
    let mut coefs = [0i32; IST_8X8_HEIGHT];
    for (slot, &pos) in coefs[..params.n].iter_mut().zip(scan) {
        if let Some(&coeff) = dequant[..coded].get(usize::from(pos)) {
            *slot = coeff.clamp(-bound, bound - 1);
        }
    }
    Ok(transform_into_with_runner(
        runner, &coefs, params, dequant, coded,
    ))
}

/// Quantized inputs of a § 7.15.3 secondary inverse transform.
/// Coefficients beyond the supplied prefix are implicitly zero.
#[derive(Clone, Copy, Debug)]
pub enum SecondaryQuant<'a> {
    /// Row-major coefficients through the last stored raster position.
    Raster(&'a [i32]),
    /// Up to `n` coefficients in the primary scan order of the transform.
    Scan(&'a [i32]),
}

/// Storage layout for the secondary transform's output coefficients.
pub(crate) enum SecondaryOutput<'a> {
    Full(&'a mut [i32]),
    /// The leading eight columns of a block at least 16 samples wide.
    Rows8(&'a mut [i32; 64]),
}

/// [`secondary_inverse_transform_prefix`] of the § 7.14.4 flat dequantization
/// of the `quant` prefix by `(dc_quant, ac_quant, dq_denom)`: only the
/// first `n` scan positions feed the transform and every other `Dequant`
/// entry ends zero, so only those positions are dequantized. Writes the
/// returned whole-row prefix of `dequant`.
pub(crate) fn secondary_inverse_transform_quant(
    quant: SecondaryQuant<'_>,
    quantizers: (u32, u32, u32),
    params: &SecondaryInverseTransform,
    dequant: SecondaryOutput<'_>,
) -> Result<usize> {
    secondary_inverse_transform_quant_with_runner(BaselineIst, quant, quantizers, params, dequant)
}

/// Typed [`secondary_inverse_transform_quant`] using caller-selected `backend`.
pub(crate) fn secondary_inverse_transform_quant_with<B: SimdBackend>(
    backend: B,
    quant: SecondaryQuant<'_>,
    quantizers: (u32, u32, u32),
    params: &SecondaryInverseTransform,
    dequant: SecondaryOutput<'_>,
) -> Result<usize> {
    secondary_inverse_transform_quant_with_runner(
        TypedIst(backend.narrow()),
        quant,
        quantizers,
        params,
        dequant,
    )
}

fn secondary_inverse_transform_quant_with_runner<R: IstRunner>(
    runner: R,
    quant: SecondaryQuant<'_>,
    quantizers: (u32, u32, u32),
    params: &SecondaryInverseTransform,
    dequant: SecondaryOutput<'_>,
) -> Result<usize> {
    let (len, output_params, dst) = match dequant {
        SecondaryOutput::Full(dst) => (dst.len(), *params, dst),
        SecondaryOutput::Rows8(rows) => {
            if params.w < 16 {
                return Err(ReconError::SecondaryTransformInvalidShape {
                    w: params.w,
                    h: params.h,
                });
            }
            (
                params.w.wrapping_mul(params.h),
                SecondaryInverseTransform { w: 8, ..*params },
                rows.as_mut_slice(),
            )
        }
    };
    let coefs = dequantized_inputs_with_runner(runner, quant, quantizers, params, len)?;
    Ok(transform_into_with_runner(
        runner,
        &coefs,
        &output_params,
        dst,
        0,
    ))
}

macro_rules! ist_runner_methods {
    ($runner:ident, $($mode:tt)+) => {
        simd_runner_methods! {
            $($mode)+;

            #[inline]
            (fn dequant_pow2(
                $runner,
                coefs: &mut [i32; IST_8X8_HEIGHT],
                n: usize,
                quantizers: (u32, u32, u32),
                bound: i32,
            )) simd =>
                dequant_ist_pow2(simd, coefs, n, quantizers, bound);

            #[inline]
            (fn ist_8x8(
                $runner,
                kernel: Option<&IstKernel>,
                coefs: &[i32],
                wide: bool,
                bound: i32,
                geom: (usize, usize),
                dequant: &mut [i32],
            )) simd =>
                ist_8x8_simd(simd, kernel, coefs, wide, bound, geom, dequant);

            #[inline]
            (fn ist_4x4(
                $runner,
                kernel: usize,
                stx: usize,
                coefs: &[i32],
                wide: bool,
                bound: i32,
            ) -> [i32; IST_4X4_WIDTH]) simd =>
                ist_4x4_simd(simd, kernel, stx, coefs, wide, bound);
        }
    };
}

trait IstRunner: Copy {
    ist_runner_methods!(self, declare);
}

#[derive(Clone, Copy)]
struct BaselineIst;

impl IstRunner for BaselineIst {
    ist_runner_methods!(self, baseline);
}

#[derive(Clone, Copy)]
struct TypedIst<B: SimdBackend>(B);

impl<B: SimdBackend> IstRunner for TypedIst<B> {
    ist_runner_methods!(self, typed(with_simd, self.0.narrow()));
}

#[inline]
fn dequant_ist_pow2<S: Simd>(
    simd: S,
    coefs: &mut [i32; IST_8X8_HEIGHT],
    n: usize,
    (dc_quant, ac_quant, shift): (u32, u32, u32),
    bound: i32,
) {
    let used = n.div_ceil(8);
    for (k, chunk) in coefs
        .as_chunks_mut::<8>()
        .0
        .iter_mut()
        .take(used)
        .enumerate()
    {
        let first = if k == 0 { dc_quant } else { ac_quant };
        let q2 = u32x8::load_array(
            simd,
            [
                first, ac_quant, ac_quant, ac_quant, ac_quant, ac_quant, ac_quant, ac_quant,
            ],
        );
        crate::dequant_process::dequant_lanes(
            simd,
            i32x8::from_slice(simd, chunk),
            q2,
            shift,
            bound,
        )
        .store_slice(chunk);
    }
}

#[inline]
fn ist_8x8_simd<S: Simd>(
    simd: S,
    kernel: Option<&IstKernel>,
    coefs: &[i32],
    wide: bool,
    bound: i32,
    (w, len): (usize, usize),
    dequant: &mut [i32],
) {
    let zero = i32x8::splat(simd, 0);
    let mut sums = [zero; 6];
    let rows = if let Some(kernel) = kernel {
        accumulate_kernel_pairs(simd, &kernel.pairs, coefs, wide, &mut sums);
        raster_rows(simd, kernel.layout, sums.map(|s| clip(simd, s, bound)))
    } else {
        [zero; 8]
    };
    if let (8, Some(block)) = (w, dequant.first_chunk_mut::<64>()) {
        for (row, &t) in block.as_chunks_mut::<8>().0.iter_mut().zip(&rows) {
            t.store_array(row);
        }
    } else {
        for (r, &t) in rows.iter().enumerate() {
            if let Some(row) = dequant
                .get_mut(r * w..len)
                .and_then(<[i32]>::first_chunk_mut::<8>)
            {
                t.store_array(row);
            }
        }
    }
}

#[inline]
fn ist_4x4_simd<S: Simd>(
    simd: S,
    kernel: usize,
    stx: usize,
    coefs: &[i32],
    wide: bool,
    bound: i32,
) -> [i32; IST_4X4_WIDTH] {
    let mut sums = [i32x8::splat(simd, 0); IST_4X4_WIDTH / 8];
    if let Some(pairs) = IST_4X4_PAIRS.get(kernel).and_then(|set| set.get(stx)) {
        accumulate_kernel_pairs(simd, pairs, coefs, wide, &mut sums);
    }
    let mut clipped = [0i32; IST_4X4_WIDTH];
    for (&t, out) in sums.iter().zip(clipped.as_chunks_mut::<8>().0) {
        clip(simd, t, bound).store_slice(out);
    }
    clipped
}

/// The dequantized inputs of [`secondary_inverse_transform_quant`] into a
/// `len`-sample `Dequant`.
fn dequantized_inputs_with_runner<R: IstRunner>(
    runner: R,
    quant: SecondaryQuant<'_>,
    (dc_quant, ac_quant, dq_denom): (u32, u32, u32),
    params: &SecondaryInverseTransform,
    len: usize,
) -> Result<[i32; IST_8X8_HEIGHT]> {
    let (SecondaryQuant::Raster(values) | SecondaryQuant::Scan(values)) = quant;
    let (scan, bound) = validated(params, len, values.len())?;
    let mut coefs = [0i32; IST_8X8_HEIGHT];
    match quant {
        SecondaryQuant::Raster(values) => {
            for (slot, &pos) in coefs[..params.n].iter_mut().zip(scan) {
                *slot = values.get(usize::from(pos)).copied().unwrap_or(0);
            }
        }
        SecondaryQuant::Scan(values) => {
            let dst = coefs[..params.n].get_mut(..values.len()).ok_or(
                ReconError::SecondaryTransformBufferMismatch {
                    expected: params.n,
                    actual: values.len(),
                },
            )?;
            dst.copy_from_slice(values); // splot-copy-ok: scan-ordered inputs go directly into the dequantization lanes.
        }
    }
    if dq_denom.is_power_of_two() && scan.first() == Some(&0) {
        let shift = dq_denom.trailing_zeros();
        runner.dequant_pow2(&mut coefs, params.n, (dc_quant, ac_quant, shift), bound);
    } else {
        for (slot, &pos) in coefs[..params.n].iter_mut().zip(scan) {
            let q2 = if pos == 0 { dc_quant } else { ac_quant };
            *slot = crate::dequant_coefficient(*slot, q2, dq_denom, params.bit_depth);
        }
    }
    Ok(coefs)
}

/// The checked shape, parameters and buffer of `params` with the primary
/// scan and the § 7.15.3 clamp bound.
fn validated(
    params: &SecondaryInverseTransform,
    len: usize,
    coded: usize,
) -> Result<(&'static [u16], i32)> {
    let SecondaryInverseTransform {
        w,
        h,
        n,
        kernel,
        sec_tx_type,
        primary_scan_class,
        bit_depth,
        ..
    } = *params;
    if !is_valid_side(w) || !is_valid_side(h) {
        return Err(ReconError::SecondaryTransformInvalidShape { w, h });
    }
    let (set_size, kernel_height) = if w >= 8 && h >= 8 {
        (IST_SET_SIZE_8X8, IST_8X8_HEIGHT)
    } else {
        (IST_SET_SIZE_4X4, IST_4X4_HEIGHT)
    };
    if n == 0 || n > kernel_height || kernel >= set_size || !(1..STX_TYPES).contains(&sec_tx_type) {
        return Err(ReconError::SecondaryTransformInvalidParams {
            n,
            kernel,
            sec_tx_type,
        });
    }
    let scan = crate::coefficient_scan_slice(w, h, primary_scan_class)?;
    if len != w * h || coded > len {
        return Err(ReconError::SecondaryTransformBufferMismatch {
            expected: w * h,
            actual: len,
        });
    }
    Ok((scan, 1i32 << (u32::from(bit_depth.bits()) + 7)))
}

/// Runs the kernel over the gathered `coefs` and writes `Dequant`: zero over
/// its first `coded` entries and the output rows, with the outputs scattered
/// into the top-left sub-block. Returns the written whole-row prefix.
/// Adding 32768 maps the coefficients that fit in `i16` to `0..=65535`.
fn transform_into_with_runner<R: IstRunner>(
    runner: R,
    coefs: &[i32; IST_8X8_HEIGHT],
    params: &SecondaryInverseTransform,
    dequant: &mut [i32],
    coded: usize,
) -> usize {
    let SecondaryInverseTransform {
        w,
        h,
        n,
        kernel,
        sec_tx_type,
        transpose,
        bit_depth,
        ..
    } = *params;
    let bound = 1i32 << (u32::from(bit_depth.bits()) + 7);
    let expected = w * h;
    let large = w >= 8 && h >= 8;
    let output_rows = if large { 8 } else { 4 };
    let len = coded
        .max((output_rows * w).min(expected))
        .min(dequant.len());
    let clear_from = if w == output_rows { w * output_rows } else { 0 };
    if clear_from < len {
        dequant[clear_from..len].fill(0);
    }
    let wide = bound > 1 << 15
        && coefs
            .iter()
            .fold(0u32, |bits, &coef| bits | coef.wrapping_add(1 << 15) as u32)
            > u32::from(u16::MAX);
    let coefs = coefs.get(..n.div_ceil(2) * 2).unwrap_or(&coefs[..]);
    let stx = sec_tx_type - 1;
    if large {
        let kernel = IST_8X8_PACKED
            .get(usize::from(transpose))
            .and_then(|sets| sets.get(kernel))
            .and_then(|set| set.get(stx));
        runner.ist_8x8(kernel, coefs, wide, bound, (w, len), dequant);
        return len;
    }
    let clipped = runner.ist_4x4(kernel, stx, coefs, wide, bound);
    let rows = clipped.as_chunks::<4>().0;
    for (y, (dst, &src)) in dequant[..len].chunks_mut(w).zip(rows).enumerate() {
        if let Some(dst) = dst.first_chunk_mut::<4>() {
            *dst = if transpose {
                [rows[0][y], rows[1][y], rows[2][y], rows[3][y]]
            } else {
                src
            };
        }
    }
    len
}

/// The § 7.15.3 `Round2Signed(t, 7)` output clamped to `bound`.
#[inline]
fn clip<S: Simd>(simd: S, t: i32x8<S>, bound: i32) -> i32x8<S> {
    ((t + (1 << 6) + (t >> 31)) >> 7)
        .max(i32x8::splat(simd, -bound))
        .min(i32x8::splat(simd, bound - 1))
}

#[derive(Clone, Copy)]
enum IstRaster {
    SixRows,
    SixColumns,
    TaperedRows,
}

#[derive(Clone, Copy)]
struct IstKernel {
    layout: IstRaster,
    pairs: [[i16; 2 * IST_8X8_WIDTH]; IST_8X8_HEIGHT / 2],
}

/// Paired § 9.7 `Ist_8x8_Kernel` weights for the 48 raster-ordered outputs.
static IST_8X8_PACKED: [[[IstKernel; STX_TYPES - 1]; IST_SET_SIZE_8X8]; 2] = {
    const EMPTY: IstKernel = IstKernel {
        layout: IstRaster::TaperedRows,
        pairs: [[0i16; 2 * IST_8X8_WIDTH]; IST_8X8_HEIGHT / 2],
    };
    let mut kernels = [[[EMPTY; STX_TYPES - 1]; IST_SET_SIZE_8X8]; 2];
    let mut i = 0;
    while i < 2 * IST_SET_SIZE_8X8 * (STX_TYPES - 1) {
        let count = IST_SET_SIZE_8X8 * (STX_TYPES - 1);
        let transpose = i / count;
        let set = i % count / (STX_TYPES - 1);
        let kind = i % (STX_TYPES - 1);
        let mut outputs = [0; IST_8X8_WIDTH];
        let mut mask = 0;
        let mut col = 0;
        while col < outputs.len() {
            let mapped = STX_SCAN_MAP[set][kind][col];
            assert!(mapped >= 0 && (mapped as usize) < STX_SCAN_ORDER_8X8.len());
            let pos = STX_SCAN_ORDER_8X8[mapped as usize];
            let pos = if transpose == 0 {
                pos
            } else {
                (pos % 8) * 8 + pos / 8
            };
            outputs[col] = pos;
            mask |= 1u64 << pos;
            col += 1;
        }
        let layout = if mask == 0x0000_ffff_ffff_ffff {
            IstRaster::SixRows
        } else if mask == 0x3f3f_3f3f_3f3f_3f3f {
            IstRaster::SixColumns
        } else {
            assert!(mask == 0x030f_3f3f_7f7f_ffff);
            IstRaster::TaperedRows
        };
        kernels[transpose][set][kind].layout = layout;
        col = 0;
        while col < outputs.len() {
            let rank = (mask & ((1u64 << outputs[col]) - 1)).count_ones() as usize;
            let mut row = 0;
            while row < IST_8X8_HEIGHT {
                let weight = IST_8X8_KERNEL[set][kind][row][col];
                kernels[transpose][set][kind].pairs[row / 2][2 * rank + row % 2] = weight as i16;
                row += 1;
            }
            col += 1;
        }
        i += 1;
    }
    kernels
};

/// Fills the unused raster positions of the § 7.15.3 output with zeros.
fn raster_rows<S: Simd>(
    simd: S,
    layout: IstRaster,
    [sum0, sum1, sum2, sum3, sum4, sum5]: [i32x8<S>; 6],
) -> [i32x8<S>; 8] {
    let zero = i32x8::splat(simd, 0);
    let mask6 = i32x8::load_array(simd, [-1, -1, -1, -1, -1, -1, 0, 0]);
    match layout {
        IstRaster::SixRows => [sum0, sum1, sum2, sum3, sum4, sum5, zero, zero],
        IstRaster::SixColumns => [
            sum0 & mask6,
            simd.slide_i32x8::<6>(sum0, sum1) & mask6,
            simd.slide_i32x8::<4>(sum1, sum2) & mask6,
            simd.slide_i32x8::<2>(sum2, sum3) & mask6,
            sum3 & mask6,
            simd.slide_i32x8::<6>(sum3, sum4) & mask6,
            simd.slide_i32x8::<4>(sum4, sum5) & mask6,
            simd.slide_i32x8::<2>(sum5, zero) & mask6,
        ],
        IstRaster::TaperedRows => {
            let mask7 = i32x8::load_array(simd, [-1, -1, -1, -1, -1, -1, -1, 0]);
            let mask4 = i32x8::load_array(simd, [-1, -1, -1, -1, 0, 0, 0, 0]);
            [
                sum0,
                sum1,
                sum2 & mask7,
                simd.slide_i32x8::<7>(sum2, sum3) & mask7,
                simd.slide_i32x8::<6>(sum3, sum4) & mask6,
                simd.slide_i32x8::<4>(sum4, sum5) & mask6,
                simd.slide_i32x8::<2>(sum5, zero) & mask4,
                simd.slide_i32x8::<6>(sum5, zero),
            ]
        }
    }
}

/// The § 9.7 4x4 kernel in raster output order, with adjacent input weights
/// paired for the SIMD multiply-add. The row pass can store four pixels at once.
static IST_4X4_PAIRS: [[[[i16; 2 * IST_4X4_WIDTH]; IST_4X4_HEIGHT / 2]; STX_TYPES - 1];
    IST_SET_SIZE_4X4] = pair_kernels(&IST_4X4_KERNEL, &STX_SCAN_ORDER_4X4);

const fn pair_kernels<
    const W: usize,
    const H: usize,
    const T: usize,
    const S: usize,
    const W2: usize,
    const H2: usize,
>(
    kernels: &[[[[i8; W]; H]; T]; S],
    raster_positions: &[usize; W],
) -> [[[[i16; W2]; H2]; T]; S] {
    assert!(W2 == 2 * W && 2 * H2 == H);
    let mut out = [[[[0i16; W2]; H2]; T]; S];
    let mut index = 0;
    while index < S * T * H * W {
        let (set, rest) = (index / (T * H * W), index % (T * H * W));
        let (kind, rest) = (rest / (H * W), rest % (H * W));
        let (row, col) = (rest / W, rest % W);
        let weight = kernels[set][kind][row][col];
        out[set][kind][row / 2][2 * raster_positions[col] + row % 2] = weight as i16;
        index += 1;
    }
    out
}

/// Adds `coef * row` over the kernel rows of the nonzero input coefficients
/// (an even count, zero padded) as `pmaddwd` pairs of adjacent rows, the sums
/// in registers; NEON keeps 32 unpaired `smlal2` sums per pass and pairs them.
///
/// A coefficient inside `i16` (every 8-bit one) is one multiply-add operand;
/// a `wide` coefficient `c` splits into `a = c >> 8` and `b = c & 255`, both
/// inside `i16`, and the row sum takes `256 * sum(k * a) + sum(k * b)`, the
/// same `i32` value.
#[inline]
fn accumulate_kernel_pairs<S: Simd, const W2: usize, const N: usize>(
    simd: S,
    pairs: &[[i16; W2]],
    coefs: &[i32],
    wide: bool,
    sums: &mut [i32x8<S>; N],
) {
    let operand =
        |a: i32, b: i32| i32x8::splat(simd, (a & 0xffff) | (b << 16)).bitcast::<i16x16<S>>();
    #[cfg(target_arch = "aarch64")]
    if !wide {
        for (pass, out) in sums.chunks_mut(4).enumerate() {
            let mut acc = [i32x4::splat(simd, 0); 16];
            for (pair, &[a, b]) in pairs.iter().zip(coefs.as_chunks::<2>().0) {
                if a | b == 0 {
                    continue;
                }
                let op = i16x8::splat(simd, a as i16).zip_low(i16x8::splat(simd, b as i16));
                let chunks = pair.get(64 * pass..).unwrap_or_default().as_chunks::<8>().0;
                for (chunk, acc) in chunks.iter().zip(acc.as_chunks_mut::<2>().0) {
                    let (low, high) = i16x8::from_slice(simd, chunk).widening_mul(op);
                    acc[0] += low;
                    acc[1] += high;
                }
            }
            for (sum, [a, b, c, d]) in out.iter_mut().zip(acc.as_chunks::<4>().0) {
                let pair_sums = |x: i32x4<S>, y| x.unzip_low(y) + x.unzip_high(y);
                *sum = pair_sums(*a, *b).combine(pair_sums(*c, *d));
            }
        }
        return;
    }
    for (pair, &[a, b]) in pairs.iter().zip(coefs.as_chunks::<2>().0) {
        if a | b == 0 {
            continue;
        }
        let (high, low) = if wide {
            (operand(a >> 8, b >> 8), operand(a & 0xff, b & 0xff))
        } else {
            (operand(a, b), operand(0, 0))
        };
        for (chunk, sum) in pair.as_chunks::<16>().0.iter().zip(sums.iter_mut()) {
            let weights = i16x16::from_slice(simd, chunk);
            let mut term = high.pairwise_dot_product(weights);
            if wide {
                term = (term << 8) + low.pairwise_dot_product(weights);
            }
            *sum += term;
        }
    }
}

/// Whether `side` is a valid § 7.15.3 operating side: a power of two in `4..=32`.
const fn is_valid_side(side: usize) -> bool {
    matches!(side, 4 | 8 | 16 | 32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coefficient_scan::coefficient_scan_order;

    fn round2_signed_reference(value: i64, shift: u32) -> i64 {
        let magnitude = value.unsigned_abs();
        let rounded = if shift == 0 {
            magnitude
        } else {
            (magnitude + (1u64 << (shift - 1))) >> shift
        };
        if value < 0 {
            -(rounded as i64)
        } else {
            rounded as i64
        }
    }

    fn params(
        w: usize,
        h: usize,
        n: usize,
        kernel: usize,
        sec: usize,
        transpose: bool,
    ) -> SecondaryInverseTransform {
        SecondaryInverseTransform {
            w,
            h,
            n,
            kernel,
            sec_tx_type: sec,
            primary_scan_class: TransformClass::TwoD,
            transpose,
            bit_depth: BitDepth::Eight,
        }
    }

    #[expect(clippy::many_single_char_names)]
    fn reference(dequant: &mut [i32], p: &SecondaryInverseTransform) {
        let large = p.w >= 8 && p.h >= 8;
        let (kernel_width, scan_bwl, scan_w) = if large {
            (IST_8X8_WIDTH, 3u32, 8usize)
        } else {
            (IST_4X4_WIDTH, 2u32, 4usize)
        };
        let mut scan = [0u16; 32 * 32];
        let scan = &mut scan[..p.w * p.h];
        coefficient_scan_order(p.w, p.h, p.primary_scan_class, scan).unwrap();
        let coefs: Vec<i64> = scan[..p.n]
            .iter()
            .map(|&pos| i64::from(dequant[pos as usize]))
            .collect();
        dequant.fill(0);
        let stx = p.sec_tx_type - 1;
        let bound = 1i64 << (u32::from(p.bit_depth.bits()) + 7);
        for i in 0..kernel_width {
            let mut t = 0i64;
            for (j, &c) in coefs.iter().enumerate() {
                let weight = if large {
                    IST_8X8_KERNEL[p.kernel][stx][j][i]
                } else {
                    IST_4X4_KERNEL[p.kernel][stx][j][i]
                };
                t += c * i64::from(weight);
            }
            let v = round2_signed_reference(t, 7).clamp(-bound, bound - 1) as i32;
            let pos = if large {
                STX_SCAN_ORDER_8X8[STX_SCAN_MAP[p.kernel][stx][i] as usize]
            } else {
                STX_SCAN_ORDER_4X4[i]
            };
            let x = pos & (scan_w - 1);
            let y = pos >> scan_bwl;
            dequant[if p.transpose {
                x * p.w + y
            } else {
                y * p.w + x
            }] = v;
        }
    }

    fn assert_matches_reference<const N: usize>(base: [i32; N], p: &SecondaryInverseTransform) {
        let mut produced = base;
        secondary_inverse_transform(&mut produced, p).unwrap();
        let mut expected = base;
        reference(&mut expected, p);
        assert_eq!(produced, expected);
    }

    #[test]
    fn small_4x4_matches_independent_reference() {
        let base: [i32; 16] = core::array::from_fn(|i| (i as i32 - 7) * 11);
        assert_matches_reference(base, &params(4, 4, IST_4X4_HEIGHT, 3, 2, false));
    }

    #[test]
    fn small_4x4_transpose_swaps_output_axes() {
        let base: [i32; 16] = core::array::from_fn(|i| (i as i32) * 5 - 30);
        let p_plain = params(4, 4, IST_4X4_HEIGHT, 1, 1, false);
        let p_transposed = params(4, 4, IST_4X4_HEIGHT, 1, 1, true);
        assert_matches_reference(base, &p_plain);
        assert_matches_reference(base, &p_transposed);

        let mut plain = base;
        secondary_inverse_transform(&mut plain, &p_plain).unwrap();
        let mut transposed = base;
        secondary_inverse_transform(&mut transposed, &p_transposed).unwrap();
        assert_ne!(plain, transposed);
    }

    #[test]
    fn input_scan_class_selects_gather_order() {
        let base: [i32; 16] = core::array::from_fn(|i| (i as i32 + 1) * 3);
        let two_d = params(4, 4, IST_4X4_HEIGHT, 2, 1, false);
        let mut horizontal = two_d;
        horizontal.primary_scan_class = TransformClass::Horizontal;

        let mut produced_two_d = base;
        secondary_inverse_transform(&mut produced_two_d, &two_d).unwrap();
        let mut produced_horizontal = base;
        secondary_inverse_transform(&mut produced_horizontal, &horizontal).unwrap();

        assert_ne!(produced_two_d, produced_horizontal);
        assert_matches_reference(base, &horizontal);
    }

    #[test]
    fn secondary_transform_clears_coefficients_outside_the_support_region() {
        let mut dequant = [0i32; 16];
        dequant[15] = 4096;
        secondary_inverse_transform(&mut dequant, &params(4, 4, IST_4X4_HEIGHT, 0, 1, false))
            .unwrap();
        assert_eq!(dequant[15], 0);
    }

    #[test]
    fn large_8x8_matches_independent_reference() {
        let base: [i32; 64] = core::array::from_fn(|i| (i as i32 % 9 - 4) * 13);
        assert_matches_reference(base, &params(8, 8, IST_8X8_HEIGHT, 7, 3, false));
    }

    #[test]
    fn paired_kernel_rows_match_the_reference_at_both_depths() {
        let mut state = 0x9e37_79b9_7f4a_7c15u64;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state >> 40) as i32 - (1 << 23)
        };
        for (bit_depth, range) in [(BitDepth::Eight, 8), (BitDepth::Ten, 6)] {
            for kernel in 0..IST_SET_SIZE_8X8 {
                for sec in 1..STX_TYPES {
                    for transpose in [false, true] {
                        for shift in [range, range + 3] {
                            let base: [i32; 64] = core::array::from_fn(|_| next() >> shift);
                            for n in [IST_8X8_HEIGHT, 20, 7] {
                                let mut p = params(8, 8, n, kernel, sec, transpose);
                                p.bit_depth = bit_depth;
                                assert_matches_reference(base, &p);
                            }
                        }
                    }
                }
            }
            for (kernel, sec) in [(0, 1), (5, 2), (10, 3)] {
                let small: [i32; 16] = core::array::from_fn(|_| next() >> (range + (next() & 3)));
                let mut p = params(4, 4, IST_4X4_HEIGHT, kernel, sec, true);
                p.bit_depth = bit_depth;
                assert_matches_reference(small, &p);
            }
        }
    }

    #[test]
    fn large_8x8_reduced_height_uses_only_n_inputs() {
        let base: [i32; 64] = core::array::from_fn(|i| (i as i32 % 5 - 2) * 7);
        assert_matches_reference(base, &params(8, 8, 20, 2, 1, false));
    }

    #[test]
    fn rejects_invalid_shape_buffer_and_params() {
        let mut nine = [0i32; 9];
        assert!(matches!(
            secondary_inverse_transform(&mut nine, &params(3, 3, 8, 0, 1, false)),
            Err(ReconError::SecondaryTransformInvalidShape { w: 3, h: 3 })
        ));
        let mut short = [0i32; 15];
        assert!(matches!(
            secondary_inverse_transform(&mut short, &params(4, 4, 8, 0, 1, false)),
            Err(ReconError::SecondaryTransformBufferMismatch {
                expected: 16,
                actual: 15
            })
        ));
        let mut block = [0i32; 16];
        assert!(matches!(
            secondary_inverse_transform(&mut block, &params(4, 4, 9, 0, 1, false)),
            Err(ReconError::SecondaryTransformInvalidParams { .. })
        ));
        assert!(matches!(
            secondary_inverse_transform(&mut block, &params(4, 4, 8, 0, 0, false)),
            Err(ReconError::SecondaryTransformInvalidParams { .. })
        ));
        assert!(matches!(
            secondary_inverse_transform(&mut block, &params(4, 4, 8, 14, 1, false)),
            Err(ReconError::SecondaryTransformInvalidParams { .. })
        ));
        assert_eq!(block, [0i32; 16]);
    }

    #[test]
    fn is_total_for_extreme_coefficients() {
        for &(w, h, n, kernel) in &[
            (4usize, 4usize, IST_4X4_HEIGHT, 13usize),
            (32, 32, IST_8X8_HEIGHT, 0),
        ] {
            let mut max_block = vec![i32::MAX; w * h];
            secondary_inverse_transform(&mut max_block, &params(w, h, n, kernel, 3, true)).unwrap();
            let mut min_block = vec![i32::MIN; w * h];
            secondary_inverse_transform(&mut min_block, &params(w, h, n, kernel, 3, false))
                .unwrap();
        }
    }

    #[test]
    fn small_4x4_dc_only_matches_hand_computed_kernel_values() {
        let mut dequant = [0i32; 16];
        dequant[0] = 128; // scan position 0 is the DC for a 4x4 2D scan
        secondary_inverse_transform(&mut dequant, &params(4, 4, IST_4X4_HEIGHT, 0, 1, false))
            .unwrap();
        assert_eq!(dequant[0], 102); // (128*102 + 64) >> 7 = 102
        assert_eq!(dequant[1], -45); // -((128*45 + 64) >> 7) = -45
        assert_eq!(dequant[4], -53); // -((128*53 + 64) >> 7) = -53
    }

    #[test]
    fn typed_backend_variants_match_compatibility_routes() {
        use splot_core::simd::{SimdPolicy, SimdScope, resolve_backend};

        struct Probe<'a> {
            params: &'a SecondaryInverseTransform,
            quant: &'a [i32],
            dequant_init: &'a [i32],
            coded: usize,
            quantizers: (u32, u32, u32),
        }
        impl
            SimdScope<(
                Vec<i32>,
                (usize, Vec<i32>),
                (usize, Vec<i32>),
                (usize, [i32; 64]),
            )> for Probe<'_>
        {
            fn run<B: SimdBackend>(
                self,
                backend: B,
            ) -> (
                Vec<i32>,
                (usize, Vec<i32>),
                (usize, Vec<i32>),
                (usize, [i32; 64]),
            ) {
                let mut full = Vec::from(self.dequant_init);
                secondary_inverse_transform_with(backend, &mut full, self.params).unwrap();

                let mut pref = Vec::from(self.dequant_init);
                let pref_len = secondary_inverse_transform_prefix_with(
                    backend,
                    &mut pref,
                    self.coded,
                    self.params,
                )
                .unwrap();

                let mut q_out = vec![0i32; self.dequant_init.len()];
                let q_len = secondary_inverse_transform_quant_with(
                    backend,
                    SecondaryQuant::Raster(self.quant),
                    self.quantizers,
                    self.params,
                    SecondaryOutput::Full(&mut q_out),
                )
                .unwrap();

                let mut rows = [0i32; 64];
                let r_len = if self.params.w >= 16 {
                    secondary_inverse_transform_quant_with(
                        backend,
                        SecondaryQuant::Raster(self.quant),
                        self.quantizers,
                        self.params,
                        SecondaryOutput::Rows8(&mut rows),
                    )
                    .unwrap()
                } else {
                    0
                };
                (full, (pref_len, pref), (q_len, q_out), (r_len, rows))
            }
        }

        let resolved = resolve_backend(SimdPolicy::Auto).expect("auto backend");
        for (w, h, n, kernel, transpose) in [
            (4usize, 4usize, 8usize, 0usize, false),
            (8, 8, 24, 3, true),
            (16, 8, 32, 5, false),
        ] {
            let p = params(w, h, n, kernel, 2, transpose);
            let dequant_init: Vec<i32> = (0..w * h).map(|i| (i as i32 * 53) - 310).collect();
            let quant: Vec<i32> = (0..(w * h).min(36))
                .map(|i| (i as i32 * 19) - 100)
                .collect();
            let coded = (w * h).min(w * 2);
            let quantizers = (96, 144, 2);

            let mut c_full = dequant_init.clone();
            secondary_inverse_transform(&mut c_full, &p).unwrap();
            let mut c_pref = dequant_init.clone();
            let c_pref_len = secondary_inverse_transform_prefix(&mut c_pref, coded, &p).unwrap();
            let mut c_q = vec![0i32; w * h];
            let c_q_len = secondary_inverse_transform_quant(
                SecondaryQuant::Raster(&quant),
                quantizers,
                &p,
                SecondaryOutput::Full(&mut c_q),
            )
            .unwrap();
            let mut c_rows = [0i32; 64];
            let c_r_len = if w >= 16 {
                secondary_inverse_transform_quant(
                    SecondaryQuant::Raster(&quant),
                    quantizers,
                    &p,
                    SecondaryOutput::Rows8(&mut c_rows),
                )
                .unwrap()
            } else {
                0
            };

            let (t_full, (t_pref_len, t_pref), (t_q_len, t_q), (t_r_len, t_rows)) =
                resolved.enter(Probe {
                    params: &p,
                    quant: &quant,
                    dequant_init: &dequant_init,
                    coded,
                    quantizers,
                });
            assert_eq!(t_full, c_full);
            assert_eq!((t_pref_len, t_pref), (c_pref_len, c_pref));
            assert_eq!((t_q_len, t_q), (c_q_len, c_q));
            assert_eq!((t_r_len, t_rows), (c_r_len, c_rows));
        }
    }
}
