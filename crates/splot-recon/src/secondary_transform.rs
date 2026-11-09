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

use fearless_simd::{
    Bytes as _, Simd, SimdBase as _, SimdIntWiden as _, SimdWiden as _, i8x32, i16x16, i32x8, u32x8,
};

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
    pub kernel: &'static [i32],
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
        primary_scan_class,
        transpose,
        bit_depth,
    } = *params;
    if !is_valid_side(w) || !is_valid_side(h) {
        return Err(ReconError::SecondaryTransformInvalidShape { w, h });
    }
    let large = w >= 8 && h >= 8;
    let (set_size, kernel_height, width) = if large {
        (IST_SET_SIZE_8X8, IST_8X8_HEIGHT, IST_8X8_WIDTH)
    } else {
        (IST_SET_SIZE_4X4, IST_4X4_HEIGHT, IST_4X4_WIDTH)
    };
    let invalid = || ReconError::SecondaryTransformInvalidParams {
        n,
        kernel,
        sec_tx_type,
    };
    if n == 0 || n > kernel_height || kernel >= set_size || !(1..STX_TYPES).contains(&sec_tx_type) {
        return Err(invalid());
    }
    let stx = sec_tx_type - 1;
    let scan = crate::coefficient_scan_slice(w, h, primary_scan_class)?;
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
        bound: 1i32 << (u32::from(bit_depth.bits()) + 7),
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

/// [`secondary_inverse_transform`] of a block whose samples from `coded` on
/// are zero and may be unwritten: zeroes only the whole-row prefix that may
/// be non-zero afterwards, and returns its length.
pub(crate) fn secondary_inverse_transform_prefix(
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
    Ok(transform_into(&coefs, params, dequant, coded))
}

/// [`secondary_inverse_transform_prefix`] of the § 7.14.4 flat dequantization
/// of the raster `quant` prefix by `(dc_quant, ac_quant, dq_denom)`: only the
/// first `n` scan positions feed the transform and every other `Dequant`
/// entry ends zero, so only those positions are dequantized. Writes the
/// returned whole-row prefix of `dequant`.
pub(crate) fn secondary_inverse_transform_quant(
    quant: &[i32],
    quantizers: (u32, u32, u32),
    params: &SecondaryInverseTransform,
    dequant: &mut [i32],
) -> Result<usize> {
    let coefs = dequantized_inputs(quant, quantizers, params, dequant.len())?;
    Ok(transform_into(&coefs, params, dequant, 0))
}

/// [`secondary_inverse_transform_quant`] of a block at least 16 wide into the
/// first 8 columns of its rows, row `r` at `rows[8 * r..]`: the output rows
/// as the 8-wide transform writes them. Returns the written length.
pub(crate) fn secondary_inverse_transform_quant_rows8(
    quant: &[i32],
    quantizers: (u32, u32, u32),
    params: &SecondaryInverseTransform,
    rows: &mut [i32; 64],
) -> Result<usize> {
    if params.w < 16 {
        return Err(ReconError::SecondaryTransformInvalidShape {
            w: params.w,
            h: params.h,
        });
    }
    let coefs = dequantized_inputs(quant, quantizers, params, params.w * params.h)?;
    let narrow = SecondaryInverseTransform { w: 8, ..*params };
    Ok(transform_into(&coefs, &narrow, rows, 0))
}

/// The dequantized inputs of [`secondary_inverse_transform_quant`] into a
/// `len`-sample `Dequant`.
fn dequantized_inputs(
    quant: &[i32],
    (dc_quant, ac_quant, dq_denom): (u32, u32, u32),
    params: &SecondaryInverseTransform,
    len: usize,
) -> Result<[i32; IST_8X8_HEIGHT]> {
    let (scan, bound) = validated(params, len, quant.len())?;
    let mut coefs = [0i32; IST_8X8_HEIGHT];
    for (slot, &pos) in coefs[..params.n].iter_mut().zip(scan) {
        *slot = quant.get(usize::from(pos)).copied().unwrap_or(0);
    }
    if dq_denom.is_power_of_two() && scan.first() == Some(&0) {
        let shift = dq_denom.trailing_zeros();
        baseline_simd!(simd => {
            let used = params.n.div_ceil(8);
            for (k, chunk) in coefs.as_chunks_mut::<8>().0.iter_mut().take(used).enumerate() {
                let first = if k == 0 { dc_quant } else { ac_quant };
                let q2 = u32x8::load_array(simd, [first, ac_quant, ac_quant, ac_quant, ac_quant, ac_quant, ac_quant, ac_quant]);
                crate::dequant_process::dequant_lanes(simd, i32x8::from_slice(simd, chunk), q2, shift, bound)
                    .store_slice(chunk);
            }
        });
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
fn transform_into(
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
    if !(large && len == 64 && w == 8) {
        dequant[..len].fill(0);
    }
    let coefs = coefs.get(..n.div_ceil(2) * 2).unwrap_or(&coefs[..]);
    let stx = sec_tx_type - 1;
    let wide = bound > 1 << 15;
    if large {
        let pairs = IST_8X8_RASTER
            .get(usize::from(transpose))
            .and_then(|sets| sets.get(kernel))
            .and_then(|set| set.get(stx));
        baseline_simd!(simd => {
            let mut sums = [i32x8::splat(simd, 0); 8];
            if let Some(pairs) = pairs {
                accumulate_kernel_pairs(simd, pairs, coefs, wide, &mut sums);
            }
            for (r, &t) in sums.iter().enumerate() {
                if let Some(row) = dequant.get_mut(r * w..len).and_then(<[i32]>::first_chunk_mut::<8>) {
                    ((t + (1 << 6) + (t >> 31)) >> 7)
                        .max(i32x8::splat(simd, -bound))
                        .min(i32x8::splat(simd, bound - 1))
                        .store_array(row);
                }
            }
        });
        return len;
    }
    let clipped: [i32; IST_8X8_WIDTH] = baseline_simd!(simd => {
        let mut sums = [i32x8::splat(simd, 0); IST_8X8_WIDTH / 8];
        if let Some(pairs) = IST_4X4_PAIRS.get(kernel).and_then(|set| set.get(stx)) {
            accumulate_kernel_pairs(simd, pairs, coefs, wide, &mut sums);
        }
        let mut clipped = [0i32; IST_8X8_WIDTH];
        for (&t, out) in sums.iter().zip(clipped.as_chunks_mut::<8>().0) {
            ((t + (1 << 6) + (t >> 31)) >> 7)
                .max(i32x8::splat(simd, -bound))
                .min(i32x8::splat(simd, bound - 1))
                .store_slice(out);
        }
        clipped
    });
    let mut tile = [0i32; 64];
    for (&value, &pos) in clipped.iter().zip(&OUTPUTS_4X4[usize::from(transpose)]) {
        tile[usize::from(pos) & 63] = value;
    }
    for (row, tile) in dequant[..len]
        .chunks_mut(w)
        .zip(tile.as_chunks::<8>().0)
        .take(4)
    {
        if let (Some(row), Some(tile)) = (row.first_chunk_mut::<4>(), tile.first_chunk::<4>()) {
            *row = *tile;
        }
    }
    len
}

/// The output-block position, `row * 8 + column` with `[1]` transposed, of
/// each `Ist_8x8_Kernel` output of a kernel set and type:
/// `Stx_Scan_Order_8x8[Stx_Scan_Map[kernel][stx][i]]`.
static OUTPUTS_8X8: [[[[u8; IST_8X8_WIDTH]; STX_TYPES - 1]; IST_SET_SIZE_8X8]; 2] = {
    let mut out = [[[[0u8; IST_8X8_WIDTH]; STX_TYPES - 1]; IST_SET_SIZE_8X8]; 2];
    let mut index = 0;
    while index < IST_SET_SIZE_8X8 * (STX_TYPES - 1) * IST_8X8_WIDTH {
        let (set, rest) = (
            index / ((STX_TYPES - 1) * IST_8X8_WIDTH),
            index % ((STX_TYPES - 1) * IST_8X8_WIDTH),
        );
        let (kind, i) = (rest / IST_8X8_WIDTH, rest % IST_8X8_WIDTH);
        let mapped = STX_SCAN_MAP[set][kind][i];
        assert!(mapped >= 0 && (mapped as usize) < STX_SCAN_ORDER_8X8.len());
        let pos = STX_SCAN_ORDER_8X8[mapped as usize];
        out[0][set][kind][i] = pos as u8;
        out[1][set][kind][i] = ((pos % 8) * 8 + pos / 8) as u8;
        index += 1;
    }
    out
};

/// The § 9.7 `Ist_8x8_Kernel` as interleaved byte pairs with each output
/// moved to its [`OUTPUTS_8X8`] position of the 8x8 block (zero weights
/// elsewhere), so the sums are the block's rows:
/// `[transpose][set][kind][q][2 * pos + r]` is the weight of the output at
/// `pos` in kernel row `2 * q + r`.
static IST_8X8_RASTER: [[[[[i8; 128]; IST_8X8_HEIGHT / 2]; STX_TYPES - 1]; IST_SET_SIZE_8X8]; 2] = {
    let mut out = [[[[[0i8; 128]; IST_8X8_HEIGHT / 2]; STX_TYPES - 1]; IST_SET_SIZE_8X8]; 2];
    let mut index = 0;
    while index < 2 * IST_SET_SIZE_8X8 * (STX_TYPES - 1) * IST_8X8_HEIGHT {
        let (transpose, rest) = (
            index / (IST_SET_SIZE_8X8 * (STX_TYPES - 1) * IST_8X8_HEIGHT),
            index % (IST_SET_SIZE_8X8 * (STX_TYPES - 1) * IST_8X8_HEIGHT),
        );
        let (set, rest) = (
            rest / ((STX_TYPES - 1) * IST_8X8_HEIGHT),
            rest % ((STX_TYPES - 1) * IST_8X8_HEIGHT),
        );
        let (kind, row) = (rest / IST_8X8_HEIGHT, rest % IST_8X8_HEIGHT);
        let mut i = 0;
        while i < IST_8X8_WIDTH {
            let pos = OUTPUTS_8X8[transpose][set][kind][i] as usize;
            let weight = IST_8X8_KERNEL[set][kind][row][i];
            assert!(weight == weight as i8 as i32);
            out[transpose][set][kind][row / 2][2 * pos + row % 2] = weight as i8;
            i += 1;
        }
        index += 1;
    }
    out
};

/// [`OUTPUTS_8X8`] of `Ist_4x4_Kernel`, from `Stx_Scan_Order_4x4[i]`
/// (`y * 4 + x`).
static OUTPUTS_4X4: [[u8; IST_4X4_WIDTH]; 2] = {
    let mut out = [[0u8; IST_4X4_WIDTH]; 2];
    let mut i = 0;
    while i < IST_4X4_WIDTH {
        let (y, x) = (STX_SCAN_ORDER_4X4[i] / 4, STX_SCAN_ORDER_4X4[i] % 4);
        out[0][i] = (y * 8 + x) as u8;
        out[1][i] = (x * 8 + y) as u8;
        i += 1;
    }
    out
};

/// The § 9.7 `Ist_4x4_Kernel` as interleaved byte pairs: entry
/// `[q][2 * i + r]` is the weight of output `i` in kernel row `2 * q + r`.
/// Every weight is below 128 in magnitude, and the quarter-size tables keep
/// the inverse transform's working set in cache.
static IST_4X4_PAIRS: [[[[i8; 2 * IST_4X4_WIDTH]; IST_4X4_HEIGHT / 2]; STX_TYPES - 1];
    IST_SET_SIZE_4X4] = pair_kernels(&IST_4X4_KERNEL);

const fn pair_kernels<
    const W: usize,
    const H: usize,
    const T: usize,
    const S: usize,
    const W2: usize,
    const H2: usize,
>(
    kernels: &[[[[i32; W]; H]; T]; S],
) -> [[[[i8; W2]; H2]; T]; S] {
    assert!(W2 == 2 * W && 2 * H2 == H);
    let mut out = [[[[0i8; W2]; H2]; T]; S];
    let mut index = 0;
    while index < S * T * H * W {
        let (set, rest) = (index / (T * H * W), index % (T * H * W));
        let (kind, rest) = (rest / (H * W), rest % (H * W));
        let (row, col) = (rest / W, rest % W);
        let weight = kernels[set][kind][row][col];
        assert!(weight == weight as i8 as i32);
        out[set][kind][row / 2][2 * col + row % 2] = weight as i8;
        index += 1;
    }
    out
}

/// Adds `coef * row` over the kernel rows of the nonzero input coefficients
/// (an even count, zero padded) as `pmaddwd` pairs of adjacent rows, keeping
/// the up to 48 sums in vector registers.
///
/// A coefficient inside `i16` (every 8-bit one) is one multiply-add operand;
/// a `wide` coefficient `c` splits into `a = c >> 8` and `b = c & 255`, both
/// inside `i16`, and the row sum takes `256 * sum(k * a) + sum(k * b)`, the
/// same `i32` value.
#[inline]
fn accumulate_kernel_pairs<S: Simd, const W2: usize, const N: usize>(
    simd: S,
    pairs: &[[i8; W2]],
    coefs: &[i32],
    wide: bool,
    sums: &mut [i32x8<S>; N],
) {
    let operand =
        |a: i32, b: i32| i32x8::splat(simd, (a & 0xffff) | (b << 16)).bitcast::<i16x16<S>>();
    for (pair, &[a, b]) in pairs.iter().zip(coefs.as_chunks::<2>().0) {
        if a | b == 0 {
            continue;
        }
        let (high, low) = if wide {
            (operand(a >> 8, b >> 8), operand(a & 0xff, b & 0xff))
        } else {
            (operand(a, b), operand(0, 0))
        };
        for (chunk, sums) in pair
            .as_chunks::<32>()
            .0
            .iter()
            .zip(sums.as_chunks_mut::<2>().0)
        {
            let (lo, hi) = i8x32::from_slice(simd, chunk).widen();
            for (sum, weights) in sums.iter_mut().zip([lo, hi]) {
                let mut term = high.pairwise_dot_product(weights);
                if wide {
                    term = (term << 8) + low.pairwise_dot_product(weights);
                }
                *sum += term;
            }
        }
    }
}

/// Whether `side` is a valid § 7.15.3 operating side: a power of two in `4..=32`.
const fn is_valid_side(side: usize) -> bool {
    matches!(side, 4 | 8 | 16 | 32)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::coefficient_scan_order;

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

    #[allow(clippy::many_single_char_names)]
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
            for (kernel, sec) in [(0, 1), (5, 2), (10, 3)] {
                let base: [i32; 64] = core::array::from_fn(|_| next() >> (range + (next() & 3)));
                for n in [IST_8X8_HEIGHT, 20, 7] {
                    let mut p = params(8, 8, n, kernel, sec, false);
                    p.bit_depth = bit_depth;
                    assert_matches_reference(base, &p);
                }
                let small: [i32; 16] = core::array::from_fn(|i| base[i]);
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
}
