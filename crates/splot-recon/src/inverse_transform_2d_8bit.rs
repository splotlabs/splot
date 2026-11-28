// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 § 7.15.4.1 kernel-by-kernel 2D matrix transform.
//!
//! Every § 7.15.2.1 input lies in the row clamp `±2^(BitDepth + 7)`, every
//! row output in the row `Clip3` bound of the same range (§ 7.15.2.1,
//! `docs/spec/av2/1.0.0/07-decoding-process.md#s-7-15-2-1`), and every kernel
//! entry below `2^7`, so at 10 bits `|sum| < 32 * 2^7 * 2^17` never wraps an
//! `i32` and the re-associated sums are the scalar sums.
//!
//! The lane form here is one `i32` multiply-add per input and output, four
//! outputs per vector: the row pass vectorizes over a row's outputs, the
//! column pass over the block's columns, and the `DCT` passes split into
//! even and odd halves. At 8 bits the AVX2 form in [`avx2`] runs on
//! `pmaddwd` pairs instead, and the NEON form in `neon` keeps `i16` lanes
//! between the passes.

#[cfg(target_arch = "x86_64")]
use fearless_simd::Level;
use fearless_simd::{
    Bytes as _, Simd, SimdBase as _, SimdInt, i16x8, i16x16, i32x4, i32x8, u8x16, u16x8, u16x16,
    u32x8, u64x2,
};
use splot_core::simd::SimdBackend;
use splot_tables::tables::transform_1d::{DCT_KERNEL8, DCT_KERNEL16, DCT_KERNEL32};

use crate::dequant_process::{dequant_coefficient, dequant_lanes};
use crate::inverse_transform::{InverseTransform1dType, Kernel1d, select_kernel};
use crate::inverse_transform_2d::InverseTransform2dDim;
use crate::secondary_transform::{
    SecondaryInverseTransform, SecondaryOutput, SecondaryQuant, secondary_inverse_transform_quant,
    secondary_inverse_transform_quant_with,
};
use crate::{BitDepth, ReconSample};

#[cfg(target_arch = "x86_64")]
mod avx2;
#[cfg(target_arch = "aarch64")]
mod neon;

/// One § 7.15.4.1 kernel-by-kernel transform.
#[derive(Clone, Copy)]
pub(crate) struct Transform8 {
    pub bit_depth: crate::BitDepth,
    pub row: InverseTransform2dDim,
    pub col: InverseTransform2dDim,
    /// Adjusted width and height.
    pub w: usize,
    pub h: usize,
    /// Whether the row inputs take the `Round2(x * 2896, 12)` rescale.
    pub rescale: bool,
    pub row_shift: u8,
    pub col_shift: u8,
    /// The flat dequantization of a quantized input, or `None` for an
    /// already dequantized whole-row prefix.
    pub dequant: Option<FlatDequant>,
}

/// The § 7.14.4 `useQm == 0` dequantization of a transform block with a
/// power-of-two `dq_denom`, applied as the row pass loads its input; its
/// clamp `±2^(BitDepth + 7)` is the row input clamp.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FlatDequant {
    dc: u32,
    ac: u32,
    denom_shift: u32,
}

impl FlatDequant {
    /// The flat dequantization by the `dc_quant` and `ac_quant` quantizers
    /// and `dq_denom`, or `None` when `dq_denom` is not a power of two.
    #[must_use]
    pub const fn new(dc_quant: u32, ac_quant: u32, dq_denom: u32) -> Option<Self> {
        if !dq_denom.is_power_of_two() {
            return None;
        }
        Some(Self {
            dc: dc_quant,
            ac: ac_quant,
            denom_shift: dq_denom.trailing_zeros(),
        })
    }
}

/// How the row pass loads its input: the optional rescale and the optional
/// dequantization.
#[derive(Clone, Copy)]
struct Load {
    rescale: bool,
    dequant: Option<FlatDequant>,
}

impl Load {
    /// Up to eight coefficients of `row` from `start`, zero past its end,
    /// dequantized to `±bound` when the input is quantized; `dc` marks lane
    /// 0 as the DC coefficient. No rescale or row clamp.
    #[inline]
    fn coeffs<S: Simd>(
        self,
        simd: S,
        row: &[i32],
        (start, bound): (usize, i32),
        dc: bool,
    ) -> i32x8<S> {
        let v = if let Some(chunk) = row.get(start..start + 8) {
            i32x8::from_slice(simd, chunk)
        } else {
            let tail = row.get(start..).unwrap_or_default();
            if tail.is_empty() {
                return i32x8::splat(simd, 0);
            }
            let padded: [i32; 8] = core::array::from_fn(|i| tail.get(i).copied().unwrap_or(0));
            i32x8::from_slice(simd, &padded)
        };
        self.dequantize(simd, v, dc, bound)
    }

    #[inline]
    fn dequantize<S: Simd>(self, simd: S, v: i32x8<S>, dc: bool, bound: i32) -> i32x8<S> {
        let Some(dq) = self.dequant else {
            return v;
        };
        let q2 = if dc {
            let ac = dq.ac;
            u32x8::load_array(simd, [dq.dc, ac, ac, ac, ac, ac, ac, ac])
        } else {
            u32x8::splat(simd, dq.ac)
        };
        dequant_lanes(simd, v, q2, dq.denom_shift, bound)
    }
}

/// Row `row` of a `w`-wide input, short when the input ends inside it.
#[inline]
fn input_row(input: &[i32], row: usize, w: usize) -> &[i32] {
    let row = input.get(row * w..).unwrap_or_default();
    &row[..row.len().min(w)]
}

/// Working memory of [`InverseTransformAdd`]: the row-pass outputs of a block
/// and its tile.
pub type InverseTransformAddScratch = [[i32; 32 * 32]; 2];

/// Writes chunk `index` of 8 row-pass outputs to `out`.
#[inline]
fn put<S: Simd>(out: &mut [i32], index: usize, v: i32x8<S>) {
    if let Some(out) = out.as_chunks_mut::<8>().0.get_mut(index) {
        v.store_slice(out);
    }
}

/// Writes group `index` of 8 column-pass outputs to `tile`. A tile holds the
/// `w * h` rounded column sums of a block in raster order as `Sat16`, two to
/// an `i32` word: the § 7.14.3 add reads them as they are, since
/// `Clip1(p + Clip3(-2^BitDepth, 2^BitDepth - 1, r)) == Clip1(p + Sat16(r))`
/// for a sample `p` below `2^15`.
#[inline]
fn put_tile<S: Simd>(simd: S, tile: &mut [i32], index: usize, v: i32x8<S>) {
    if let Some(out) = tile.as_chunks_mut::<4>().0.get_mut(index) {
        let (lo, hi) = simd.split_i32x8(v);
        let v: i16x8<S> = simd.saturating_narrow_i32x4(lo, hi);
        v.bitcast::<i32x4<S>>().store_slice(out);
    }
}

/// Group `index` of 8 tile samples in `words`.
#[inline]
fn tile_group<S: Simd>(simd: S, words: &[i32; 4]) -> i16x8<S> {
    i32x4::from_slice(simd, words).bitcast()
}

/// Writes group `index` of 8 rounded column sums to `out`: with `RESIDUAL`
/// their column `Clip3` to `±bound` as the residual, else into the tile.
/// The residual is an output of its own because widening it from the tile
/// costs the streams that read it (quantization matrices) 0.5%.
#[inline]
fn put_out<S: Simd, const RESIDUAL: bool>(
    simd: S,
    out: &mut [i32],
    index: usize,
    v: i32x8<S>,
    bound: i32,
) {
    if RESIDUAL {
        let v = v
            .max(i32x8::splat(simd, -bound))
            .min(i32x8::splat(simd, bound - 1));
        put(out, index, v);
    } else {
        put_tile(simd, out, index, v);
    }
}

/// The § 7.14.3 `Clip1(prediction + residual)` of the tile of a `w`-wide
/// block onto the 8-bit block at the start of `dst`, rows `stride` apart.
fn add_tile_u8<S: Simd>(simd: S, tile: &[i32], w: usize, dst: &mut [u8], stride: usize) {
    let groups = tile.as_chunks::<4>().0;
    if w == 4 {
        let rows = dst.chunks_mut(2 * stride.max(2));
        for (res, rows) in groups.iter().zip(rows) {
            let Some((a, b)) = rows.split_at_mut_checked(stride) else {
                return;
            };
            let (Some(a), Some(b)) = (a.first_chunk_mut::<4>(), b.first_chunk_mut::<4>()) else {
                return;
            };
            let pred = u64::from(u32::from_le_bytes(*a)) | u64::from(u32::from_le_bytes(*b)) << 32;
            let sum = add_u8x8(simd, pred, tile_group(simd, res));
            *a = (sum as u32).to_le_bytes();
            *b = ((sum >> 32) as u32).to_le_bytes();
        }
        return;
    }
    for (res, row) in groups
        .chunks_exact(w / 8)
        .zip(dst.chunks_mut(stride.max(1)))
    {
        for (res, out) in res.iter().zip(row.as_chunks_mut::<8>().0) {
            *out = add_u8x8(simd, u64::from_le_bytes(*out), tile_group(simd, res)).to_le_bytes();
        }
    }
}

/// [`add_tile_u8`] onto a 16-bit block of samples up to `max`, 16 samples at
/// a time in a block at least 16 wide.
fn add_tile_u16<S: Simd>(
    simd: S,
    tile: &[i32],
    w: usize,
    (dst, stride): (&mut [u16], usize),
    max: i16,
) {
    let groups = tile.as_chunks::<4>().0;
    if w == 4 {
        let rows = dst.chunks_mut(2 * stride.max(2));
        for (res, rows) in groups.iter().zip(rows) {
            let Some((a, b)) = rows.split_at_mut_checked(stride) else {
                return;
            };
            let (Some(a), Some(b)) = (a.first_chunk_mut::<4>(), b.first_chunk_mut::<4>()) else {
                return;
            };
            let pred = [a[0], a[1], a[2], a[3], b[0], b[1], b[2], b[3]];
            let sum = add_u16x8(simd, pred, tile_group(simd, res), max);
            let (first, second) = sum.split_at(4);
            a.copy_from_slice(first); // splot-copy-ok: one row of a 4-wide lane group
            b.copy_from_slice(second); // splot-copy-ok: one row of a 4-wide lane group
        }
        return;
    }
    if w == 8 {
        for (res, row) in groups.iter().zip(dst.chunks_mut(stride.max(1))) {
            if let Some(out) = row.first_chunk_mut::<8>() {
                *out = add_u16x8(simd, *out, tile_group(simd, res), max);
            }
        }
        return;
    }
    let rows = tile.chunks_exact(w / 2).zip(dst.chunks_mut(stride.max(1)));
    for (res, row) in rows {
        let res = res.as_chunks::<8>().0;
        for (res, out) in res.iter().zip(row.as_chunks_mut::<16>().0) {
            let res: i16x16<S> = i32x8::from_slice(simd, res).bitcast();
            let pred = u16x16::load_array(simd, *out).bitcast::<i16x16<S>>();
            let sum = simd
                .saturating_add_i16x16(pred, res)
                .max(i16x16::splat(simd, 0))
                .min(i16x16::splat(simd, max));
            *out = sum.bitcast::<u16x16<S>>().into();
        }
    }
}

/// `Clip1(p + residual)` over the `w x h` block at the start of `dst`, `w`
/// at least 8, for the column-clamped constant `residual` and samples up to
/// `max`.
#[inline]
fn add_constant_u16<S: Simd>(
    simd: S,
    dst: &mut [u16],
    (w, h, stride): (usize, usize, usize),
    (residual, max): (i32, i16),
) {
    let residual = i16x8::splat(simd, residual as i16);
    for row in dst.chunks_mut(stride.max(1)).take(h) {
        let Some(row) = row.get_mut(..w) else {
            continue;
        };
        for out in row.as_chunks_mut::<8>().0 {
            let pred = u16x8::load_array(simd, *out).bitcast::<i16x8<S>>();
            let sum = simd
                .saturating_add_i16x8(pred, residual)
                .max(i16x8::splat(simd, 0))
                .min(i16x8::splat(simd, max));
            *out = sum.bitcast::<u16x8<S>>().into();
        }
    }
}

/// Eight `Clip1(p + r)` of `pred` and the tile samples `res`, for samples up
/// to `max`.
#[inline]
fn add_u16x8<S: Simd>(simd: S, pred: [u16; 8], res: i16x8<S>, max: i16) -> [u16; 8] {
    let pred = u16x8::load_array(simd, pred).bitcast::<i16x8<S>>();
    let sum = simd
        .saturating_add_i16x8(pred, res)
        .max(i16x8::splat(simd, 0))
        .min(i16x8::splat(simd, max));
    sum.bitcast::<u16x8<S>>().into()
}

/// Eight `Clip1(p + r)` of the eight bytes of `pred` and the tile samples
/// `res`.
#[inline]
fn add_u8x8<S: Simd>(simd: S, pred: u64, res: i16x8<S>) -> u64 {
    let pred: u8x16<S> = u64x2::load_array(simd, [pred, 0]).bitcast();
    let (pred, _) = simd.widen_u8x16(pred);
    let sum = simd
        .saturating_add_i16x8(pred.bitcast::<i16x8<S>>(), res)
        .max(i16x8::splat(simd, 0));
    let packed = simd.saturating_narrow_u16x8(sum.bitcast::<u16x8<S>>(), u16x8::splat(simd, 0));
    packed.bitcast::<u64x2<S>>().as_array()[0]
}

/// One transform shape and type that has the fused form of the
/// § 7.15.4 process: the § 7.15.4.1 transform added in place as the § 7.14.3
/// `Clip1(prediction + Residual)`. A block resolves it with one table lookup.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InverseTransformAdd {
    row: InverseTransform2dDim,
    col: InverseTransform2dDim,
    w_log2: u8,
    h_log2: u8,
    row_shift: u8,
    col_shift: u8,
    rescale: bool,
    /// Whether both passes are `DCT`, whose DC-only residual is constant.
    flat: bool,
    #[cfg(target_arch = "x86_64")]
    passes: (avx2::Pass, avx2::Pass),
    #[cfg(target_arch = "aarch64")]
    passes: (neon::Pass, neon::Pass),
}

/// [`InverseTransformAdd`] by `[log2W - 2][log2H - 2][PlaneTxType][useDdt]`
/// for sides of 4 to 32 samples.
static FUSED: [[[[Option<InverseTransformAdd>; 2]; 16]; 4]; 4] = {
    let mut out = [[[[None; 2]; 16]; 4]; 4];
    let mut i = 0;
    while i < 4 * 4 * 16 * 2 {
        let (w, h, tx_type, ddt) = (i / 128, i / 32 % 4, i / 2 % 16, i % 2 == 1);
        out[w][h][tx_type][ddt as usize] =
            InverseTransformAdd::resolve(tx_type, w as u32 + 2, h as u32 + 2, ddt);
        i += 1;
    }
    out
};

impl InverseTransformAdd {
    /// The fused form of `PlaneTxType` `plane_tx_type` on the original
    /// `log2_width x log2_height` transform with the `use_ddt` substitution,
    /// as [`InverseTransform2dOuter::resolve`] derives its transform, or
    /// `None` for a 64-sample side or an invalid shape or type.
    ///
    /// [`InverseTransform2dOuter::resolve`]: crate::InverseTransform2dOuter::resolve
    #[inline]
    #[must_use]
    pub fn new(
        plane_tx_type: usize,
        log2_width: u32,
        log2_height: u32,
        use_ddt: bool,
    ) -> Option<Self> {
        *FUSED
            .get(log2_width.wrapping_sub(2) as usize)?
            .get(log2_height.wrapping_sub(2) as usize)?
            .get(plane_tx_type)?
            .get(usize::from(use_ddt))?
    }

    const fn resolve(
        plane_tx_type: usize,
        log2_width: u32,
        log2_height: u32,
        use_ddt: bool,
    ) -> Option<Self> {
        let Ok(outer) = crate::InverseTransform2dOuter::resolve(
            plane_tx_type,
            log2_width,
            log2_height,
            use_ddt,
            false,
            crate::BitDepth::Eight,
            None,
        ) else {
            return None;
        };
        Self::of(
            outer.row_type,
            outer.col_type,
            (log2_width, log2_height),
            (outer.row_shift, outer.col_shift),
            log2_width.abs_diff(log2_height) % 2 == 1,
        )
    }

    /// The fused form of the adjusted `log2W x log2H` transform with the
    /// given passes and shifts, and with `rescale` the `Round2(x * 2896, 12)`
    /// row input rescale.
    pub(crate) const fn of(
        row: InverseTransform2dDim,
        col: InverseTransform2dDim,
        (w_log2, h_log2): (u32, u32),
        (row_shift, col_shift): (u8, u8),
        rescale: bool,
    ) -> Option<Self> {
        let shifts_valid = row_shift >= 1 && row_shift <= 20 && col_shift >= 1 && col_shift <= 20;
        if w_log2 < 2 || w_log2 > 5 || h_log2 < 2 || h_log2 > 5 || !shifts_valid {
            return None;
        }
        #[cfg(target_arch = "x86_64")]
        let (Some(row_pass), Some(col_pass)) = (
            avx2::Pass::of(row, 1 << w_log2),
            avx2::Pass::of(col, 1 << h_log2),
        ) else {
            return None;
        };
        Some(Self {
            row,
            col,
            w_log2: w_log2 as u8,
            h_log2: h_log2 as u8,
            row_shift,
            col_shift,
            rescale,
            flat: matches!(
                (row, col),
                (
                    InverseTransform2dDim::Kernel(InverseTransform1dType::Dct),
                    InverseTransform2dDim::Kernel(InverseTransform1dType::Dct)
                )
            ),
            #[cfg(target_arch = "x86_64")]
            passes: (row_pass, col_pass),
            #[cfg(target_arch = "aarch64")]
            passes: (
                neon::Pass::of(row, 1 << w_log2),
                neon::Pass::of(col, 1 << h_log2),
            ),
        })
    }

    /// The adjusted width.
    const fn w(self) -> usize {
        1 << self.w_log2
    }

    /// The adjusted height.
    const fn h(self) -> usize {
        1 << self.h_log2
    }

    /// Adds the transform of `coeffs` at `bit_depth` to the block at the
    /// start of `dst`, whose rows are `stride` apart.
    ///
    /// With `dequant`, `coeffs` holds the quantized coefficients in raster
    /// order up to the last nonzero one, dequantized as they load; with
    /// `None` it is an already dequantized prefix of the block. Returns
    /// `false` with `dst` untouched when `coeffs` is longer than the block,
    /// the block does not fit `dst`, or `T` does not hold `bit_depth`.
    /// Adds the transform of `coeffs` using `backend`.
    #[inline]
    pub fn add_with<T: ReconSample>(
        self,
        backend: SimdBackend,
        coeffs: &[i32],
        dequant_depth: (Option<FlatDequant>, BitDepth),
        dst: &mut [T],
        stride: usize,
        scratch: &mut InverseTransformAddScratch,
    ) -> bool {
        self.add_with_runner(
            TypedTransformRunner(backend),
            coeffs,
            dequant_depth,
            dst,
            stride,
            scratch,
        )
    }

    /// Runs [`Self::put_prediction`] and adds the transform of `coeffs`,
    /// returning `false` with `dst` untouched when `coeffs` is longer than the block,
    /// the block does not fit `dst`, or `T` does not hold `bit_depth`.
    #[inline]
    pub fn add<T: ReconSample>(
        self,
        coeffs: &[i32],
        dequant_depth: (Option<FlatDequant>, BitDepth),
        dst: &mut [T],
        stride: usize,
        scratch: &mut InverseTransformAddScratch,
    ) -> bool {
        self.add_with_runner(
            BaselineTransformRunner,
            coeffs,
            dequant_depth,
            dst,
            stride,
            scratch,
        )
    }

    #[inline]
    fn add_with_runner<R: TransformRunner, T: ReconSample>(
        self,
        runner: R,
        coeffs: &[i32],
        (dequant, bit_depth): (Option<FlatDequant>, BitDepth),
        dst: &mut [T],
        stride: usize,
        scratch: &mut InverseTransformAddScratch,
    ) -> bool {
        let (w, h) = (self.w(), self.h());
        if !self.fits(dst.len(), stride) || coeffs.len() > w * h {
            return false;
        }
        let Some(dst) = T::u8_slice_mut(dst).filter(|_| bit_depth == BitDepth::Eight) else {
            return self.run_add(
                runner,
                self.transform8(dequant, bit_depth),
                (coeffs, w),
                dst,
                stride,
                scratch,
            );
        };
        #[cfg(target_arch = "x86_64")]
        if let Some(token) = runner.avx2_token() {
            let sink = avx2::Sink::Add { dst, stride };
            avx2::transform(token, &self, dequant, coeffs, false, sink, &mut scratch[0]);
            return true;
        }
        #[cfg(target_arch = "aarch64")]
        if let Some(token) = runner.neon_token() {
            match coeffs {
                [] => {}
                &[dc] if self.flat => neon::add_flat(token, self, (dc, dequant), (dst, stride)),
                _ if w * h <= 32 => neon::add_small(token, self, (coeffs, dequant), (dst, stride)),
                _ => neon::add(
                    token,
                    self,
                    (coeffs, w, dequant),
                    (dst, stride),
                    &mut scratch[0],
                ),
            }
            return true;
        }
        self.run_add(
            runner,
            self.transform8(dequant, bit_depth),
            (coeffs, w),
            dst,
            stride,
            scratch,
        )
    }

    /// [`Self::put_prediction_with`], then [`Self::add_with`] onto it using
    /// `backend`; `false` with `dst` untouched where either would return it.
    #[expect(clippy::too_many_arguments)]
    #[inline]
    pub fn add_onto_with<T: ReconSample>(
        self,
        backend: SimdBackend,
        prediction: &[T],
        coeffs: &[i32],
        dequant_depth: (Option<FlatDequant>, BitDepth),
        dst: &mut [T],
        stride: usize,
        scratch: &mut InverseTransformAddScratch,
    ) -> bool {
        self.add_onto_with_runner(
            TypedTransformRunner(backend),
            prediction,
            coeffs,
            dequant_depth,
            dst,
            stride,
            scratch,
        )
    }

    /// [`Self::put_prediction`], then [`Self::add`] onto it; `false` with
    /// `dst` untouched where either would return it.
    #[inline]
    pub fn add_onto<T: ReconSample>(
        self,
        prediction: &[T],
        coeffs: &[i32],
        dequant_depth: (Option<FlatDequant>, BitDepth),
        dst: &mut [T],
        stride: usize,
        scratch: &mut InverseTransformAddScratch,
    ) -> bool {
        self.add_onto_with_runner(
            BaselineTransformRunner,
            prediction,
            coeffs,
            dequant_depth,
            dst,
            stride,
            scratch,
        )
    }

    #[expect(clippy::too_many_arguments)]
    #[inline]
    fn add_onto_with_runner<R: TransformRunner, T: ReconSample>(
        self,
        runner: R,
        prediction: &[T],
        coeffs: &[i32],
        (dequant, bit_depth): (Option<FlatDequant>, BitDepth),
        dst: &mut [T],
        stride: usize,
        scratch: &mut InverseTransformAddScratch,
    ) -> bool {
        coeffs.len() <= self.w() * self.h()
            && self.put_prediction_with_runner(runner, prediction, bit_depth, dst, stride)
            && self.add_with_runner(runner, coeffs, (dequant, bit_depth), dst, stride, scratch)
    }

    /// Writes `prediction` into `dst` like [`Self::put_prediction`], checking
    /// sample range with `backend`.
    #[inline]
    pub fn put_prediction_with<T: ReconSample>(
        self,
        backend: SimdBackend,
        prediction: &[T],
        bit_depth: BitDepth,
        dst: &mut [T],
        stride: usize,
    ) -> bool {
        self.put_prediction_with_runner(
            TypedTransformRunner(backend),
            prediction,
            bit_depth,
            dst,
            stride,
        )
    }

    /// Writes the `w x h` block `prediction` (rows `w` apart) into the block
    /// at the start of `dst`, rows `stride` apart. Returns `false` with `dst`
    /// untouched when the block does not fit `dst`, `prediction` is not the
    /// block, or a sample of it is above `bit_depth` or `T` does not hold it.
    #[inline]
    pub fn put_prediction<T: ReconSample>(
        self,
        prediction: &[T],
        bit_depth: BitDepth,
        dst: &mut [T],
        stride: usize,
    ) -> bool {
        self.put_prediction_with_runner(BaselineTransformRunner, prediction, bit_depth, dst, stride)
    }

    #[inline]
    fn put_prediction_with_runner<R: TransformRunner, T: ReconSample>(
        self,
        runner: R,
        prediction: &[T],
        bit_depth: BitDepth,
        dst: &mut [T],
        stride: usize,
    ) -> bool {
        let in_range = match T::u16_slice(prediction) {
            Some(samples) => !runner.u16_samples_exceed(samples, bit_depth.max_sample()),
            None => bit_depth == BitDepth::Eight,
        };
        self.write_prediction_checked(prediction, dst, stride, in_range)
    }

    #[inline]
    fn write_prediction_checked<T: ReconSample>(
        self,
        prediction: &[T],
        dst: &mut [T],
        stride: usize,
        in_range: bool,
    ) -> bool {
        let w = self.w();
        if !self.fits(dst.len(), stride) || prediction.len() != w * self.h() || !in_range {
            return false;
        }
        macro_rules! put_rows {
            ($n:literal) => {
                for (row, pred) in prediction.as_chunks::<$n>().0.iter().enumerate() {
                    if let Some(out) = dst
                        .get_mut(row * stride..)
                        .and_then(<[T]>::first_chunk_mut::<$n>)
                    {
                        *out = *pred;
                    }
                }
            };
        }
        match w {
            4 => put_rows!(4),
            8 => put_rows!(8),
            16 => put_rows!(16),
            _ => put_rows!(32),
        }
        true
    }

    /// Adds the § 7.15.3 secondary inverse transform plus primary transform onto
    /// `dst` using `backend` and the wider kernels of `kernels`.
    ///
    /// # Errors
    /// Returns the same secondary-transform validation errors as
    /// [`Self::add_secondary`].
    #[expect(clippy::too_many_arguments)]
    pub fn add_secondary_with<T: ReconSample>(
        self,
        backend: SimdBackend,
        kernels: crate::KernelSelection,
        (quantizers, secondary): ((u32, u32, u32), &SecondaryInverseTransform),
        quant: &[i32],
        dequant: &mut [i32],
        (dst, stride): (&mut [T], usize),
        scratch: &mut InverseTransformAddScratch,
    ) -> crate::Result<bool> {
        self.add_secondary_quant_with(
            backend,
            kernels,
            (quantizers, secondary),
            SecondaryQuant::Raster(quant),
            dequant,
            (dst, stride),
            scratch,
        )
    }

    /// Adds the secondary and primary inverse transforms using either
    /// row-major coefficients or the active primary-scan prefix.
    ///
    /// # Errors
    /// As [`Self::add_secondary`], or if a scan prefix exceeds `secondary.n`.
    #[expect(clippy::too_many_arguments)]
    pub fn add_secondary_quant_with<T: ReconSample>(
        self,
        backend: SimdBackend,
        kernels: crate::KernelSelection,
        (quantizers, secondary): ((u32, u32, u32), &SecondaryInverseTransform),
        quant: SecondaryQuant<'_>,
        dequant: &mut [i32],
        (dst, stride): (&mut [T], usize),
        scratch: &mut InverseTransformAddScratch,
    ) -> crate::Result<bool> {
        self.add_secondary_with_runner(
            TypedTransformRunner(backend),
            kernels,
            (quantizers, secondary),
            quant,
            dequant,
            (dst, stride),
            scratch,
        )
    }

    /// Adds the transform of the § 7.15.3 secondary inverse transform of
    /// `quant`, under the flat § 7.14.4 dequantization by the `(dc_quant,
    /// ac_quant, dq_denom)` `quantizers`, to the block at the start of `dst`
    /// as [`Self::add`] does at the bit depth of `secondary`, with the
    /// block's `Dequant` in `dequant`.
    /// A block at least 16 wide, without an identity row pass, keeps only the
    /// 8 columns the secondary transform writes and runs the row pass
    /// straight from them.
    ///
    /// # Errors
    /// Returns the [`secondary_inverse_transform`](crate::secondary_inverse_transform)
    /// errors for invalid `secondary` parameters or a `dequant` that is not
    /// the block.
    pub fn add_secondary<T: ReconSample>(
        self,
        (quantizers, secondary): ((u32, u32, u32), &SecondaryInverseTransform),
        quant: &[i32],
        dequant: &mut [i32],
        (dst, stride): (&mut [T], usize),
        scratch: &mut InverseTransformAddScratch,
    ) -> crate::Result<bool> {
        self.add_secondary_with_runner(
            BaselineTransformRunner,
            crate::KernelSelection::default(),
            (quantizers, secondary),
            SecondaryQuant::Raster(quant),
            dequant,
            (dst, stride),
            scratch,
        )
    }

    #[expect(clippy::too_many_arguments)]
    fn add_secondary_with_runner<R: TransformRunner, T: ReconSample>(
        self,
        runner: R,
        kernels: crate::KernelSelection,
        (quantizers, secondary): ((u32, u32, u32), &SecondaryInverseTransform),
        quant: SecondaryQuant<'_>,
        dequant: &mut [i32],
        (dst, stride): (&mut [T], usize),
        scratch: &mut InverseTransformAddScratch,
    ) -> crate::Result<bool> {
        let bit_depth = secondary.bit_depth;
        if self.w_log2 >= 4
            && self.fits(dst.len(), stride)
            && let Some(rows) = dequant.first_chunk_mut::<64>()
        {
            let len = runner.secondary_quant(
                kernels,
                quant,
                quantizers,
                secondary,
                SecondaryOutput::Rows8(rows),
            )?;
            #[cfg(target_arch = "x86_64")]
            if let (Some(token), Some(dst), BitDepth::Eight) =
                (runner.avx2_token(), T::u8_slice_mut(&mut *dst), bit_depth)
                && self.row != InverseTransform2dDim::Identity
            {
                let sink = avx2::Sink::Add { dst, stride };
                avx2::transform(
                    token,
                    &self,
                    None,
                    &rows[..len],
                    true,
                    sink,
                    &mut scratch[0],
                );
                return Ok(true);
            }
            #[cfg(target_arch = "aarch64")]
            if let (Some(token), Some(dst), BitDepth::Eight) =
                (runner.neon_token(), T::u8_slice_mut(&mut *dst), bit_depth)
            {
                if len > 0 {
                    let scratch = &mut scratch[0];
                    neon::add(token, self, (&rows[..len], 8, None), (dst, stride), scratch);
                }
                return Ok(true);
            }
            let t = self.transform8(None, bit_depth);
            return Ok(self.run_add(runner, t, (&rows[..len], 8), dst, stride, scratch));
        }
        let coded = runner.secondary_quant(
            kernels,
            quant,
            quantizers,
            secondary,
            SecondaryOutput::Full(dequant),
        )?;
        Ok(self.add_with_runner(
            runner,
            &dequant[..coded],
            (None, bit_depth),
            dst,
            stride,
            scratch,
        ))
    }

    /// Runs `t` over `input` into the tile, then adds it to the `u8` or
    /// `u16` block at the start of `dst`.
    fn run_add<R: TransformRunner, T: ReconSample>(
        self,
        runner: R,
        t: Transform8,
        input: (&[i32], usize),
        dst: &mut [T],
        stride: usize,
        [rows, tile]: &mut InverseTransformAddScratch,
    ) -> bool {
        let w = self.w();
        let tile = &mut tile[..w * self.h() / 2];
        if let Some(dst) = T::u8_slice_mut(&mut *dst) {
            if t.bit_depth != BitDepth::Eight || !run::<R, false>(runner, t, input, tile, rows) {
                return false;
            }
            runner.add_tile_u8(tile, w, dst, stride);
            return true;
        }
        let Some(dst) = T::u16_slice_mut(dst) else {
            return false;
        };
        let max = t.bit_depth.max_sample() as i16;
        if w >= 8
            && input.1 == w
            && let Some(value) = self.flat_residual(input.0, t.dequant, t.bit_depth)
        {
            let geometry = (w, self.h(), stride);
            runner.add_constant_u16(dst, geometry, (value, max));
            return true;
        }
        if !run::<R, false>(runner, t, input, tile, rows) {
            return false;
        }
        runner.add_tile_u16(tile, w, (dst, stride), max);
        true
    }

    /// Whether a `w x h` block with rows `stride` apart fits `len` samples.
    fn fits(self, len: usize, stride: usize) -> bool {
        (self.h() - 1)
            .checked_mul(stride)
            .and_then(|span| span.checked_add(self.w()))
            .is_some_and(|span| stride >= self.w() && span <= len)
    }

    const fn transform8(self, dequant: Option<FlatDequant>, bit_depth: BitDepth) -> Transform8 {
        Transform8 {
            bit_depth,
            row: self.row,
            col: self.col,
            w: self.w(),
            h: self.h(),
            rescale: self.rescale,
            row_shift: self.row_shift,
            col_shift: self.col_shift,
            dequant,
        }
    }

    /// The constant residual at `bit_depth` of a `DCT_DCT` block whose only
    /// nonzero coefficient is the first of `coeffs` (as [`Self::add`] takes
    /// them), or `None` when the block's residual is not constant.
    #[inline]
    #[must_use]
    pub fn flat_residual(
        &self,
        coeffs: &[i32],
        dequant: Option<FlatDequant>,
        bit_depth: BitDepth,
    ) -> Option<i32> {
        if !self.flat {
            return None;
        }
        let (&dc, rest) = coeffs.split_first()?;
        if rest.len() >= self.w() || rest.iter().any(|&coeff| coeff != 0) {
            return None;
        }
        let dc = dequant.map_or(dc, |dq| {
            dequant_coefficient(dc, dq.dc, 1 << dq.denom_shift, bit_depth)
        });
        let (row_bound, col_bound) = bounds(bit_depth);
        let round = |v: i32, shift: u8| (v + (1 << (shift - 1))) >> shift;
        let dc = if self.rescale {
            dc.wrapping_mul(2896).wrapping_add(1 << 11) >> 12
        } else {
            dc
        };
        let dc = dc.clamp(-row_bound, row_bound - 1);
        let row = round(dc * 64, self.row_shift).clamp(-row_bound, row_bound - 1);
        Some(round(row * 64, self.col_shift).clamp(-col_bound, col_bound - 1))
    }
}

macro_rules! transform_runner_methods {
    ($runner:ident, $($mode:tt)+) => {
        simd_runner_methods! {
            $($mode)+;

            #[inline]
            (fn run_dc_only<const RESIDUAL: bool>(
                $runner,
                t: Transform8,
                dc: i32,
                passes: (Pass1d, Pass1d),
                out: &mut [i32],
            )) simd =>
                dc_only_transform::<_, RESIDUAL>(simd, t, dc, passes, out);

            #[inline]
            (fn run_transform<const RESIDUAL: bool>(
                $runner,
                passes: (Pass1d, Pass1d),
                rows: Rows<'_>,
                pass: ColPass,
                out: &mut [i32],
                scratch: &mut [i32; 32 * 32],
            )) simd =>
                transform::<_, RESIDUAL>(simd, passes, rows, pass, out, scratch);

            #[inline]
            (fn add_tile_u8($runner, tile: &[i32], w: usize, dst: &mut [u8], stride: usize)) simd =>
                add_tile_u8(simd, tile, w, dst, stride);

            #[inline]
            (fn add_constant_u16(
                $runner,
                dst: &mut [u16],
                geometry: (usize, usize, usize),
                values: (i32, i16),
            )) simd =>
                add_constant_u16(simd, dst, geometry, values);

            #[inline]
            (fn add_tile_u16(
                $runner,
                tile: &[i32],
                w: usize,
                dst_stride: (&mut [u16], usize),
                max: i16,
            )) simd =>
                add_tile_u16(simd, tile, w, dst_stride, max);
        }
    };
}

trait TransformRunner: Copy {
    #[cfg(target_arch = "x86_64")]
    fn avx2_token(self) -> Option<fearless_simd::Avx2>;

    #[cfg(target_arch = "aarch64")]
    fn neon_token(self) -> Option<fearless_simd::Neon>;

    fn u16_samples_exceed(self, samples: &[u16], max: u16) -> bool;

    fn secondary_quant(
        self,
        kernels: crate::KernelSelection,
        quant: SecondaryQuant<'_>,
        quantizers: (u32, u32, u32),
        secondary: &SecondaryInverseTransform,
        dequant: SecondaryOutput<'_>,
    ) -> crate::Result<usize>;

    transform_runner_methods!(self, declare);
}

#[derive(Clone, Copy)]
struct BaselineTransformRunner;

impl TransformRunner for BaselineTransformRunner {
    #[cfg(target_arch = "x86_64")]
    #[inline]
    fn avx2_token(self) -> Option<fearless_simd::Avx2> {
        Level::baseline().as_avx2()
    }

    #[cfg(target_arch = "aarch64")]
    #[inline]
    fn neon_token(self) -> Option<fearless_simd::Neon> {
        fearless_simd::Level::baseline().as_neon()
    }

    #[inline]
    fn u16_samples_exceed(self, samples: &[u16], max: u16) -> bool {
        crate::sample_range::u16_samples_exceed(samples, max)
    }

    #[inline]
    fn secondary_quant(
        self,
        _kernels: crate::KernelSelection,
        quant: SecondaryQuant<'_>,
        quantizers: (u32, u32, u32),
        secondary: &SecondaryInverseTransform,
        dequant: SecondaryOutput<'_>,
    ) -> crate::Result<usize> {
        secondary_inverse_transform_quant(quant, quantizers, secondary, dequant)
    }

    transform_runner_methods!(self, baseline);
}

#[derive(Clone, Copy)]
struct TypedTransformRunner(SimdBackend);

impl TransformRunner for TypedTransformRunner {
    #[cfg(target_arch = "x86_64")]
    #[inline]
    fn avx2_token(self) -> Option<fearless_simd::Avx2> {
        self.0.avx2_token()
    }

    #[cfg(target_arch = "aarch64")]
    #[inline]
    fn neon_token(self) -> Option<fearless_simd::Neon> {
        self.0.neon_token()
    }

    #[inline]
    fn u16_samples_exceed(self, samples: &[u16], max: u16) -> bool {
        crate::sample_range::u16_samples_exceed_with(self.0, samples, max)
    }

    #[inline]
    fn secondary_quant(
        self,
        kernels: crate::KernelSelection,
        quant: SecondaryQuant<'_>,
        quantizers: (u32, u32, u32),
        secondary: &SecondaryInverseTransform,
        dequant: SecondaryOutput<'_>,
    ) -> crate::Result<usize> {
        secondary_inverse_transform_quant_with(
            self.0, kernels, quant, quantizers, secondary, dequant,
        )
    }

    transform_runner_methods!(self, typed(self.0));
}

/// Runs the transform of `dequant` (a whole-row prefix of the `w * h` block)
/// into the `w * h` `residual`, or returns `false` with nothing written when
/// the shape has no lane form.
pub(crate) fn inverse_transform_2d_8bit(
    t: Transform8,
    dequant: &[i32],
    residual: &mut [i32],
    scratch: &mut [i32; 32 * 32],
) -> bool {
    inverse_transform_2d_8bit_with_runner(BaselineTransformRunner, t, dequant, residual, scratch)
}

#[inline]
pub(crate) fn inverse_transform_2d_8bit_with(
    backend: SimdBackend,
    t: Transform8,
    dequant: &[i32],
    residual: &mut [i32],
    scratch: &mut [i32; 32 * 32],
) -> bool {
    inverse_transform_2d_8bit_with_runner(
        TypedTransformRunner(backend),
        t,
        dequant,
        residual,
        scratch,
    )
}

fn inverse_transform_2d_8bit_with_runner<R: TransformRunner>(
    runner: R,
    t: Transform8,
    dequant: &[i32],
    residual: &mut [i32],
    scratch: &mut [i32; 32 * 32],
) -> bool {
    if residual.len() != t.w * t.h || dequant.len() > t.w * t.h {
        return false;
    }
    #[cfg(target_arch = "x86_64")]
    if let Some(token) = runner.avx2_token()
        && t.bit_depth == crate::BitDepth::Eight
        && let Some(shape) = InverseTransformAdd::of(
            t.row,
            t.col,
            (t.w.trailing_zeros(), t.h.trailing_zeros()),
            (t.row_shift, t.col_shift),
            t.rescale,
        )
        .filter(|s| s.w() == t.w && s.h() == t.h)
    {
        let sink = avx2::Sink::Residual(residual);
        avx2::transform(token, &shape, t.dequant, dequant, false, sink, scratch);
        return true;
    }
    run::<R, true>(runner, t, (dequant, t.w), residual, scratch)
}

/// Whether `t` and `dequant` have a lane form.
fn valid(t: &Transform8, dequant: &[i32]) -> bool {
    dequant.len() <= t.w * t.h && matches!(t.w, 4 | 8 | 16 | 32) && matches!(t.h, 4 | 8 | 16 | 32)
}

/// The DC coefficient of a block whose only non-zero coefficient it is, with
/// input rows `in_w` apart.
fn dc_only(t: &Transform8, dequant: &[i32], in_w: usize) -> Option<i32> {
    let (&dc, rest) = dequant.split_first()?;
    (rest.len() < in_w && rest.iter().all(|&coeff| coeff == 0)).then(|| {
        t.dequant.map_or(dc, |dq| {
            dequant_coefficient(dc, dq.dc, 1 << dq.denom_shift, t.bit_depth)
        })
    })
}

/// The row and column `Clip3` bounds `2^(BitDepth + 7)` and `2^BitDepth`.
const fn bounds(bit_depth: crate::BitDepth) -> (i32, i32) {
    let bits = bit_depth.bits() as u32;
    (1 << (bits + 7), 1 << bits)
}

/// One resolved pass: `None` for the § 7.15.2.3 identity, or the § 7.15.2.1
/// kernel and whether its type is `DCT`.
type Pass1d = Option<(Kernel1d, bool)>;

/// `kernel[j][N - 1 - i] == (-1)^j * kernel[j][i]`, the `DCT` symmetry the
/// split passes take.
const fn dct_symmetric<const N: usize>(kernel: &[[i32; N]; N]) -> bool {
    let mut j = 0;
    while j < N {
        let mut i = 0;
        while i < N {
            let sign = if j % 2 == 0 { 1 } else { -1 };
            if kernel[j][N - 1 - i] != sign * kernel[j][i] {
                return false;
            }
            i += 1;
        }
        j += 1;
    }
    true
}

const _: () = assert!(
    dct_symmetric(&DCT_KERNEL8) && dct_symmetric(&DCT_KERNEL16) && dct_symmetric(&DCT_KERNEL32)
);

/// Runs `t` over the whole-row prefix `dequant`, whose input rows are `in_w`
/// apart (`t.w`, or 8 for a 16- or 32-wide block whose inputs are zero past
/// column 8), into `out`: the `w * h` residual with `RESIDUAL`, else the
/// `w * h / 2` words of the tile. Returns `false` with nothing written when
/// the shape or a shift has no lane form.
fn run<R: TransformRunner, const RESIDUAL: bool>(
    runner: R,
    t: Transform8,
    (dequant, in_w): (&[i32], usize),
    out: &mut [i32],
    scratch: &mut [i32; 32 * 32],
) -> bool {
    if !valid(&t, dequant)
        || out.len() != t.w * t.h / if RESIDUAL { 1 } else { 2 }
        || !(in_w == t.w || (in_w == 8 && t.w >= 16))
    {
        return false;
    }
    let pass = |dim, len| match dim {
        InverseTransform2dDim::Kernel(tx_type) => select_kernel(len, tx_type)
            .map(|kernel| Some((kernel, tx_type == InverseTransform1dType::Dct))),
        InverseTransform2dDim::Identity => Some(None),
    };
    let (Some(row), Some(col)) = (pass(t.row, t.w), pass(t.col, t.h)) else {
        return false;
    };
    if let Some(dc) = dc_only(&t, dequant, in_w) {
        runner.run_dc_only::<RESIDUAL>(t, dc, (row, col), out);
        return true;
    }
    let rows = Rows {
        input: dequant,
        in_w,
        coded: dequant.len().div_ceil(in_w).min(t.h),
        load: Load {
            rescale: t.rescale,
            dequant: t.dequant,
        },
        shift: u32::from(t.row_shift).min(31),
        bound: bounds(t.bit_depth).0,
    };
    let pass = ColPass {
        w: t.w,
        h: t.h,
        shift: u32::from(t.col_shift).min(31),
        bound: bounds(t.bit_depth).1,
    };
    runner.run_transform::<RESIDUAL>((row, col), rows, pass, out, scratch);
    true
}

/// The row pass into `scratch`, then the column pass over it into `out`.
/// A § 7.15.2.3 identity column output depends on its own sample only.
#[allow(
    clippy::inline_always,
    reason = "measured § 7.15.4.1 transform hot path"
)]
#[inline(always)]
fn transform<S: Simd, const RESIDUAL: bool>(
    simd: S,
    (row, col): (Pass1d, Pass1d),
    rows: Rows<'_>,
    pass: ColPass,
    out: &mut [i32],
    scratch: &mut [i32; 32 * 32],
) {
    row_dispatch(simd, row, pass.w, rows, &mut scratch[..]);
    if col.is_none() {
        let scale = identity_scale(pass.h);
        let coded = (rows.coded * pass.w).div_ceil(8);
        let row_outputs = scratch.as_chunks::<8>().0;
        for (index, chunk) in row_outputs.iter().enumerate().take(coded) {
            let v = i32x8::from_slice(simd, chunk) * scale;
            let v = round_lanes(v, pass.shift);
            put_out::<_, RESIDUAL>(simd, out, index, v, pass.bound);
        }
        let written = coded * if RESIDUAL { 2 } else { 1 };
        for words in out.as_chunks_mut::<4>().0.iter_mut().skip(written) {
            *words = [0; 4];
        }
        return;
    }
    let input_rows = col_input_rows(pass.h, rows.coded);
    let chunks = scratch.as_chunks_mut::<8>().0.iter_mut();
    for chunk in chunks
        .take(input_rows * pass.w / 8)
        .skip((rows.coded * pass.w).div_ceil(8))
    {
        *chunk = [0; 8];
    }
    let pass = (pass, rows.coded);
    if pass.0.w == 4 {
        col_dispatch::<_, i32x4<S>, RESIDUAL>(simd, col, scratch, pass, out);
    } else {
        col_dispatch::<_, i32x8<S>, RESIDUAL>(simd, col, scratch, pass, out);
    }
}

/// The row pass of `row` over a `w`-wide block into `out`, every coded row
/// written.
#[expect(
    clippy::inline_always,
    reason = "measured § 7.15.4.1 row-pass hot path"
)]
#[inline(always)]
fn row_dispatch<S: Simd>(simd: S, row: Pass1d, w: usize, rows: Rows<'_>, out: &mut [i32]) {
    match row {
        None => row_pass_identity(simd, w, rows, out),
        Some((Kernel1d::N4(k), _)) => row_pass_4(simd, k, rows, out),
        Some((Kernel1d::N8(k, true), _)) => row_pass::<_, 8, false, true>(simd, k, rows, out),
        Some((Kernel1d::N8(k, false), true)) => {
            row_pass::<_, 8, true, false>(simd, k, rows, out);
        }
        Some((Kernel1d::N8(k, false), false)) => {
            row_pass::<_, 8, false, false>(simd, k, rows, out);
        }
        Some((Kernel1d::N16(k, true), _)) => row_pass::<_, 16, false, true>(simd, k, rows, out),
        Some((Kernel1d::N16(k, false), true)) => {
            row_pass::<_, 16, true, false>(simd, k, rows, out);
        }
        Some((Kernel1d::N16(k, false), false)) => {
            row_pass::<_, 16, false, false>(simd, k, rows, out);
        }
        Some((Kernel1d::N32(k), _)) => row_pass::<_, 32, true, false>(simd, k, rows, out),
    }
}

/// § 7.15.4.1 `get_identity_scale` of an adjusted side `len`, which a
/// 64-sample side shares with a 32-sample one.
const fn identity_scale(len: usize) -> i32 {
    match len {
        4 => 128,
        8 => 181,
        16 => 256,
        _ => 362,
    }
}

/// Row 0 of the § 7.15.2.1 kernel of `pass` at length `len`, reversed for
/// `FDDT` (the identity scale at index 0 for the § 7.15.2.3 identity).
fn first_kernel_row(pass: Pass1d, len: usize) -> [i32; 32] {
    let mut row = [0; 32];
    let (first, reversed): (&[i32], bool) = match pass {
        None => {
            row[0] = identity_scale(len);
            return row;
        }
        Some((Kernel1d::N4(k), _)) => (&k[0], false),
        Some((Kernel1d::N8(k, reversed), _)) => (&k[0], reversed),
        Some((Kernel1d::N16(k, reversed), _)) => (&k[0], reversed),
        Some((Kernel1d::N32(k), _)) => (&k[0], false),
    };
    let len = first.len();
    for (i, slot) in row.iter_mut().take(len).enumerate() {
        *slot = first[if reversed { len - 1 - i } else { i }];
    }
    row
}

/// The transform of a block whose only non-zero coefficient is `dc`: the
/// row pass leaves only row 0, `Clip3(Round2(dc * kr[c], rowShift))` for
/// row-kernel row 0 `kr`, and every column sum has the one term
/// `kc[r] * row0[c]` for column-kernel row 0 `kc`. With `DCT` both ways every
/// kernel row 0 is constant and so is the residual.
fn dc_only_transform<S: Simd, const RESIDUAL: bool>(
    simd: S,
    t: Transform8,
    dc: i32,
    (row, col): (Pass1d, Pass1d),
    out: &mut [i32],
) {
    let (row_bound, col_bound) = bounds(t.bit_depth);
    let dc = row_input(i32x8::splat(simd, dc), t.rescale, row_bound);
    let (row_shift, col_shift) = (
        u32::from(t.row_shift).min(31),
        u32::from(t.col_shift).min(31),
    );
    let kr = first_kernel_row(row, t.w);
    let kc = first_kernel_row(col, t.h);
    let row0 = |c: usize| {
        let k = kr.as_chunks::<8>().0.get(c).map_or([0; 8], |k| *k);
        clip_round(i32x8::from_slice(simd, &k) * dc, row_shift, row_bound)
    };
    let dct = |dim| dim == InverseTransform2dDim::Kernel(InverseTransform1dType::Dct);
    if dct(t.row) && dct(t.col) {
        let value: [i32; 8] = round_lanes(row0(0) * kc[0], col_shift).into();
        let value = i32x8::splat(simd, value[0]);
        for index in 0..t.w * t.h / 8 {
            put_out::<_, RESIDUAL>(simd, out, index, value, col_bound);
        }
        return;
    }
    if t.w == 4 {
        let (row0, _) = simd.split_i32x8(row0(0));
        let row0 = simd.combine_i32x4(row0, row0);
        for (index, &[a, b]) in kc.as_chunks::<2>().0.iter().take(t.h / 2).enumerate() {
            let k = i32x8::from_slice(simd, &[a, a, a, a, b, b, b, b]);
            let v = round_lanes(row0 * k, col_shift);
            put_out::<_, RESIDUAL>(simd, out, index, v, col_bound);
        }
        return;
    }
    let chunks = t.w / 8;
    let mut row0s = [i32x8::splat(simd, 0); 4];
    for (c, slot) in row0s.iter_mut().enumerate().take(chunks) {
        *slot = row0(c);
    }
    for (r, &k) in kc.iter().take(t.h).enumerate() {
        for (c, &row0) in row0s.iter().take(chunks).enumerate() {
            let v = round_lanes(row0 * k, col_shift);
            put_out::<_, RESIDUAL>(simd, out, r * chunks + c, v, col_bound);
        }
    }
}

/// The optional `Round2(x * 2896, 12)` rescale.
#[inline]
fn rescaled<S: Simd>(v: i32x8<S>, rescale: bool) -> i32x8<S> {
    if rescale {
        (v * 2896 + (1 << 11)) >> 12
    } else {
        v
    }
}

/// The optional rescale and the row input clamp to `±bound`.
#[inline]
fn row_input<S: Simd>(v: i32x8<S>, rescale: bool, bound: i32) -> i32x8<S> {
    let v = rescaled(v, rescale);
    v.max(i32x8::splat(v.simd, -bound))
        .min(i32x8::splat(v.simd, bound - 1))
}

/// The `Clip3` to `±bound` of `Round2(sum, shift)`.
#[inline]
fn clip_round<S: Simd>(sum: i32x8<S>, shift: u32, bound: i32) -> i32x8<S> {
    round_lanes(sum, shift)
        .max(i32x8::splat(sum.simd, -bound))
        .min(i32x8::splat(sum.simd, bound - 1))
}

/// `Round2(sum, shift)` over `V` lanes for `shift <= 31` and `|sum| < 2^30`
/// (zero at 31).
#[inline]
fn round_lanes<S: Simd, V: SimdInt<S, Element = i32>>(sum: V, shift: u32) -> V {
    (sum + ((1u32 << shift) >> 1) as i32) >> shift
}

/// The coded input rows of a row pass and how they load.
#[derive(Clone, Copy)]
struct Rows<'a> {
    /// The whole-row prefix, rows `in_w` apart.
    input: &'a [i32],
    in_w: usize,
    /// Rows the prefix holds, at most the block height.
    coded: usize,
    load: Load,
    /// Row-pass down-shift, at most 31.
    shift: u32,
    /// The row input and output clamp `2^(BitDepth + 7)`.
    bound: i32,
}

impl Rows<'_> {
    /// The clamped § 7.15.2.1 inputs of the eight coefficients of `row` from
    /// `8 * k`; `dc` marks lane 0 as the DC coefficient.
    #[inline]
    fn inputs<S: Simd>(self, simd: S, row: &[i32], k: usize, dc: bool) -> i32x8<S> {
        row_input(
            self.load.coeffs(simd, row, (8 * k, self.bound), dc),
            self.load.rescale,
            self.bound,
        )
    }
}

/// Whether any lane of `v` is non-zero.
#[inline]
fn any<S: Simd>(simd: S, v: i32x4<S>) -> bool {
    simd.any_true_mask32x4(v.simd_ne(i32x4::splat(simd, 0)))
}

/// The row pass of an `N`-wide block, `N >= 8`, into chunks `r * N / 8..` of
/// `out` for every coded row `r`. Each output lane takes one multiply-add
/// per coded group of four inputs, four outputs at a time, from the `i32`
/// kernel row of each input; a 32-wide row skips its all-zero groups. Rows
/// below 32 wide run `32 / N` at a time, so one kernel load serves them all,
/// and fixed trip counts keep the inputs in registers.
///
/// `SPLIT` takes the `DCT` symmetry `kernel[j][N - 1 - i] == (-1)^j *
/// kernel[j][i]`: the even inputs sum `E` and the odd inputs `O` over the
/// first `N / 2` outputs, and output `N - 1 - i` is `E - O` where output `i`
/// is `E + O`. `REV` reverses the outputs (`FDDT`).
#[expect(
    clippy::inline_always,
    reason = "measured § 7.15.4.1 row-pass hot path"
)]
#[inline(always)]
fn row_pass<S: Simd, const N: usize, const SPLIT: bool, const REV: bool>(
    simd: S,
    kernel: &[[i32; N]; N],
    rows: Rows<'_>,
    out: &mut [i32],
) {
    let zero = i32x4::splat(simd, 0);
    let step = if N >= 32 { 1 } else { 32 / N };
    for r0 in (0..rows.coded).step_by(step) {
        let mut x = [[zero; 8]; 4];
        for (r, x) in (r0..).zip(&mut x).take(step) {
            let row = input_row(rows.input, r, rows.in_w);
            for (k, pair) in x.as_chunks_mut::<2>().0.iter_mut().take(N / 8).enumerate() {
                let v = rows.inputs(simd, row, k, r == 0 && k == 0);
                (pair[0], pair[1]) = simd.split_i32x8(v);
            }
        }
        let mut sums = [[[zero; 8]; 2]; 4];
        for (g, k) in kernel.as_chunks::<4>().0.iter().enumerate() {
            if 4 * g >= rows.in_w || (N >= 32 && !any(simd, x[0][g])) {
                continue;
            }
            for (lane, k) in k.iter().enumerate() {
                let (parity, len) = if SPLIT { (lane % 2, N / 8) } else { (0, N / 4) };
                for (v, k) in k.as_chunks::<4>().0.iter().take(len).enumerate() {
                    let k = i32x4::from_slice(simd, k);
                    for (sums, x) in sums.iter_mut().zip(&x).take(step) {
                        sums[parity][v] += k * x[g][lane];
                    }
                }
            }
        }
        for (r, [first, second]) in (r0..rows.coded).zip(sums).take(step) {
            let mut y = [zero; 8];
            for (i, y) in y.iter_mut().enumerate().take(N / 4) {
                *y = if SPLIT && i < N / 8 {
                    first[i] + second[i]
                } else if SPLIT {
                    let i = N / 4 - 1 - i;
                    (first[i] - second[i]).reverse()
                } else if REV {
                    first[N / 4 - 1 - i].reverse()
                } else {
                    first[i]
                };
            }
            for (j, y) in y.as_chunks::<2>().0.iter().take(N / 8).enumerate() {
                let v = clip_round(simd.combine_i32x4(y[0], y[1]), rows.shift, rows.bound);
                put(out, r * N / 8 + j, v);
            }
        }
    }
}

/// [`row_pass`] of a 4-wide block, two rows per input load.
fn row_pass_4<S: Simd>(simd: S, kernel: &[[i32; 4]; 4], rows: Rows<'_>, out: &mut [i32]) {
    let k = kernel.map(|k| i32x4::from_slice(simd, &k));
    let transform = |x: i32x4<S>| k[0] * x[0] + k[1] * x[1] + k[2] * x[2] + k[3] * x[3];
    for q in 0..rows.coded.div_ceil(2) {
        let (a, b) = simd.split_i32x8(rows.inputs(simd, rows.input, q, q == 0));
        let sum = simd.combine_i32x4(transform(a), transform(b));
        put(out, q, clip_round(sum, rows.shift, rows.bound));
    }
}

/// [`row_pass`] for the § 7.15.2.3 identity transform: each row output is
/// its own clamped input times the identity scale, in the row `Clip3`.
fn row_pass_identity<S: Simd>(simd: S, w: usize, rows: Rows<'_>, out: &mut [i32]) {
    let scale = identity_scale(w);
    let scaled = |v: i32x8<S>| clip_round(v * scale, rows.shift, rows.bound);
    if rows.in_w == w {
        for k in 0..(rows.coded * w).div_ceil(8) {
            put(out, k, scaled(rows.inputs(simd, rows.input, k, k == 0)));
        }
        return;
    }
    for r in 0..rows.coded {
        let row = input_row(rows.input, r, rows.in_w);
        for k in 0..w / 8 {
            let v = rows.inputs(simd, row, k, r == 0 && k == 0);
            put(out, r * w / 8 + k, scaled(v));
        }
    }
}

/// The column-pass geometry of one block.
#[derive(Clone, Copy)]
struct ColPass {
    /// Block width and height.
    w: usize,
    h: usize,
    /// Column-pass down-shift, at most 31.
    shift: u32,
    /// The column `Clip3` bound `2^BitDepth`.
    bound: i32,
}

/// The column lanes of one column pass: 4 for a 4-wide block, else 8.
trait Lanes<S: Simd>: SimdInt<S, Element = i32> {
    /// Puts the column `Clip3` outputs `v` of rows `start..start + 4`
    /// (`start` a multiple of 4), lanes `c * LEN..`, of a `w`-wide block.
    fn put<const RESIDUAL: bool>(
        simd: S,
        at: (usize, usize, usize),
        v: [Self; 4],
        out: (&mut [i32], i32),
    );
}

impl<S: Simd> Lanes<S> for i32x4<S> {
    #[expect(
        clippy::inline_always,
        reason = "keeps the column outputs in registers"
    )]
    #[inline(always)]
    fn put<const RESIDUAL: bool>(
        simd: S,
        (_, _, start): (usize, usize, usize),
        v: [Self; 4],
        (out, bound): (&mut [i32], i32),
    ) {
        let (low, high) = (
            simd.combine_i32x4(v[0], v[1]),
            simd.combine_i32x4(v[2], v[3]),
        );
        put_out::<_, RESIDUAL>(simd, out, start / 2, low, bound);
        put_out::<_, RESIDUAL>(simd, out, start / 2 + 1, high, bound);
    }
}

impl<S: Simd> Lanes<S> for i32x8<S> {
    #[expect(
        clippy::inline_always,
        reason = "keeps the column outputs in registers"
    )]
    #[inline(always)]
    fn put<const RESIDUAL: bool>(
        simd: S,
        (w, c, start): (usize, usize, usize),
        v: [Self; 4],
        (out, bound): (&mut [i32], i32),
    ) {
        let chunks = w / 8;
        for (row, v) in (start..).zip(v) {
            put_out::<_, RESIDUAL>(simd, out, row * chunks + c, v, bound);
        }
    }
}

fn col_input_rows(height: usize, coded: usize) -> usize {
    if height == 32 {
        coded.min(height)
    } else if height >= 8 && coded <= height / 2 {
        height / 2
    } else {
        height
    }
}

/// The kernel column pass of `col` over the first `coded` rows of
/// `scratch`, with any padding up to the selected input height zeroed.
/// Identity columns run in place after the row pass instead.
#[allow(
    clippy::inline_always,
    reason = "measured § 7.15.4.1 column-pass hot path"
)]
#[inline(always)]
fn col_dispatch<S: Simd, V: Lanes<S>, const RESIDUAL: bool>(
    simd: S,
    col: Pass1d,
    scratch: &[i32; 32 * 32],
    pass: (ColPass, usize),
    out: &mut [i32],
) {
    match col {
        None => {}
        Some((Kernel1d::N4(k), _)) => {
            col_pass::<_, V, 4, false, false, RESIDUAL>(simd, k, scratch, pass, out);
        }
        Some((Kernel1d::N8(k, true), _)) => {
            col_pass::<_, V, 8, false, true, RESIDUAL>(simd, k, scratch, pass, out);
        }
        Some((Kernel1d::N8(k, false), true)) => {
            col_pass::<_, V, 8, true, false, RESIDUAL>(simd, k, scratch, pass, out);
        }
        Some((Kernel1d::N8(k, false), false)) => {
            col_pass::<_, V, 8, false, false, RESIDUAL>(simd, k, scratch, pass, out);
        }
        Some((Kernel1d::N16(k, true), _)) => {
            col_pass::<_, V, 16, false, true, RESIDUAL>(simd, k, scratch, pass, out);
        }
        Some((Kernel1d::N16(k, false), true)) => {
            col_pass::<_, V, 16, true, false, RESIDUAL>(simd, k, scratch, pass, out);
        }
        Some((Kernel1d::N16(k, false), false)) => {
            col_pass::<_, V, 16, false, false, RESIDUAL>(simd, k, scratch, pass, out);
        }
        Some((Kernel1d::N32(k), _)) => {
            col_pass::<_, V, 32, true, false, RESIDUAL>(simd, k, scratch, pass, out);
        }
    }
}

/// The column pass of an `M`-tall block over the `V` column lanes of the
/// first `coded` rows of `scratch`, four outputs at a time: each output lane
/// takes one multiply-add per input row. Below 32, the input height is fixed
/// at `M`, or `M / 2` if every coded row fits. `SPLIT` and `REV` are as in
/// [`row_pass`], over the rows.
#[expect(
    clippy::inline_always,
    reason = "measured § 7.15.4.1 column-pass hot path"
)]
#[inline(always)]
fn col_pass<
    S: Simd,
    V: Lanes<S>,
    const M: usize,
    const SPLIT: bool,
    const REV: bool,
    const RESIDUAL: bool,
>(
    simd: S,
    kernel: &[[i32; M]; M],
    scratch: &[i32; 32 * 32],
    (
        ColPass {
            w, shift, bound, ..
        },
        coded,
    ): (ColPass, usize),
    out: &mut [i32],
) {
    let rows = col_input_rows(M, coded);
    let zero = V::splat(simd, 0);
    let outputs = if SPLIT { M / 2 } else { M };
    let round = |v: V| round_lanes(v, shift);
    for c in 0..w / V::LEN {
        let at = |r: usize| {
            let start = r * w + c * V::LEN;
            scratch
                .get(start..start + V::LEN)
                .map(|lanes| V::from_slice(simd, lanes))
        };
        for base in (0..outputs).step_by(4) {
            let mut sums = [[zero; 4]; 2];
            for (parity, sum) in sums.iter_mut().enumerate().take(1 + usize::from(SPLIT)) {
                let step = 1 + usize::from(SPLIT);
                for r in (parity..rows).step_by(step) {
                    let k = kernel.get(r).and_then(|k| k.get(base..base + 4));
                    let (Some(y), Some(k)) = (at(r), k) else {
                        break;
                    };
                    for (sum, &k) in sum.iter_mut().zip(k) {
                        *sum += y * k;
                    }
                }
            }
            let [first, second] = sums;
            if SPLIT {
                let low = core::array::from_fn(|o| round(first[o] + second[o]));
                V::put::<RESIDUAL>(simd, (w, c, base), low, (&mut *out, bound));
                let high = core::array::from_fn(|o| round(first[3 - o] - second[3 - o]));
                V::put::<RESIDUAL>(simd, (w, c, M - 4 - base), high, (&mut *out, bound));
            } else if REV {
                let high = core::array::from_fn(|o| round(first[3 - o]));
                V::put::<RESIDUAL>(simd, (w, c, M - 4 - base), high, (&mut *out, bound));
            } else {
                V::put::<RESIDUAL>(simd, (w, c, base), first.map(round), (&mut *out, bound));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    /// A tile of every `Sat16` extreme and a spread of values in between.
    fn tile_samples(count: usize, seed: u32) -> Vec<i16> {
        let mut state = seed;
        (0..count)
            .map(|i| {
                state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                match i % 7 {
                    0 => i16::MIN,
                    1 => i16::MAX,
                    2 => (state >> 20) as i16 % 300,
                    _ => (state >> 16) as i16,
                }
            })
            .collect()
    }

    fn tile_words(samples: &[i16]) -> Vec<i32> {
        samples
            .as_chunks::<2>()
            .0
            .iter()
            .map(|&[low, high]| i32::from(low as u16) | i32::from(high) << 16)
            .collect()
    }

    /// The tile sinks must match the per-sample § 7.14.3 add for every width, into a strided destination whose samples
    /// outside the block stay untouched.
    #[test]
    fn tile_sinks_match_the_per_sample_oracle() {
        for (w, h) in [
            (4, 4),
            (4, 16),
            (8, 8),
            (16, 4),
            (16, 16),
            (32, 8),
            (32, 32),
        ] {
            let samples = tile_samples(w * h, (w * 31 + h) as u32);
            let tile = tile_words(&samples);
            let stride = w + 3;
            let len = (h - 1) * stride + w;
            for max in [255u16, 1023, 4095] {
                let pred: Vec<u16> = (0..len)
                    .map(|i| [0, max, (i as u16 * 37) % (max + 1)][i % 3])
                    .collect();
                let expected = |i: usize, p: u16| {
                    let (row, col) = (i / stride, i % stride);
                    if col >= w {
                        return p;
                    }
                    (i32::from(p) + i32::from(samples[row * w + col])).clamp(0, i32::from(max))
                        as u16
                };
                let want: Vec<u16> = pred
                    .iter()
                    .enumerate()
                    .map(|(i, &p)| expected(i, p))
                    .collect();
                let mut wide = pred.clone();
                baseline_simd!(simd => add_tile_u16(simd, &tile, w, (&mut wide, stride), max as i16));
                assert_eq!(wide, want, "{w}x{h} u16 max {max}");
                if max == 255 {
                    let mut narrow: Vec<u8> = pred.iter().map(|&p| p as u8).collect();
                    baseline_simd!(simd => add_tile_u8(simd, &tile, w, &mut narrow, stride));
                    let want: Vec<u8> = want.iter().map(|&p| p as u8).collect();
                    assert_eq!(narrow, want, "{w}x{h} u8");
                }
            }
        }
    }

    #[test]
    fn typed_backend_matches_compatibility_routes() {
        use splot_core::simd::{SimdBackend, SimdPolicy, resolve_backend};

        use crate::coefficient_scan::TransformClass;

        fn check(backend: SimdBackend, kernels: crate::KernelSelection) {
            let dq = FlatDequant {
                dc: 12,
                ac: 9,
                denom_shift: 1,
            };
            for (w_log2, h_log2) in [(3, 3), (4, 3), (4, 4)] {
                let add = InverseTransformAdd::new(0, w_log2, h_log2, false).unwrap();
                let w = add.w();
                let h = add.h();
                let stride = w + 5;
                let len = (h - 1) * stride + w;
                let coeffs: Vec<i32> = (0..w * h)
                    .map(|i| {
                        if i < 12 {
                            ((i as i32 * 17) % 91) - 45
                        } else {
                            0
                        }
                    })
                    .collect();

                let pred_u8: Vec<u8> = (0..w * h).map(|i| ((i * 29 + 11) % 251) as u8).collect();
                let mut compat_u8 = vec![7u8; len];
                let mut typed_u8 = vec![7u8; len];
                let mut scratch_a = Box::new([[0i32; 32 * 32]; 2]);
                let mut scratch_b = Box::new([[0i32; 32 * 32]; 2]);
                assert!(add.add_onto(
                    &pred_u8,
                    &coeffs,
                    (Some(dq), BitDepth::Eight),
                    &mut compat_u8,
                    stride,
                    &mut scratch_a,
                ));
                assert!(add.add_onto_with(
                    backend,
                    &pred_u8,
                    &coeffs,
                    (Some(dq), BitDepth::Eight),
                    &mut typed_u8,
                    stride,
                    &mut scratch_b,
                ));
                assert_eq!(typed_u8, compat_u8);

                let sec = SecondaryInverseTransform {
                    w,
                    h,
                    n: 16,
                    kernel: 0,
                    sec_tx_type: 1,
                    primary_scan_class: TransformClass::TwoD,
                    transpose: false,
                    bit_depth: BitDepth::Eight,
                };
                let scan = crate::coefficient_scan_slice(w, h, sec.primary_scan_class).unwrap();
                for (prefix, denom) in [(0, 1), (1, 2), (7, 3), (sec.n, 8)] {
                    let compact: Vec<_> = (0..prefix).map(|i| i as i32 * 7 - 31).collect();
                    let mut raster = vec![0; w * h];
                    for (&value, &pos) in compact.iter().zip(scan) {
                        raster[usize::from(pos)] = value;
                    }
                    let (mut expected, mut actual) = (compat_u8.clone(), compat_u8.clone());
                    let mut expected_dq = vec![99; w * h];
                    let mut actual_dq = expected_dq.clone();
                    let params = ((13, 29, denom), &sec);
                    assert_eq!(
                        add.add_secondary_with(
                            backend,
                            kernels,
                            params,
                            &raster,
                            &mut expected_dq,
                            (&mut expected, stride),
                            &mut scratch_a
                        )
                        .unwrap(),
                        add.add_secondary_quant_with(
                            backend,
                            kernels,
                            params,
                            SecondaryQuant::Scan(&compact),
                            &mut actual_dq,
                            (&mut actual, stride),
                            &mut scratch_b
                        )
                        .unwrap(),
                    );
                    assert_eq!(actual_dq, expected_dq);
                    assert_eq!(actual, expected);
                }
                let quant = vec![24i32; 16];
                let mut deq_a = vec![0i32; w * h];
                let mut deq_b = vec![0i32; w * h];
                assert!(
                    add.add_secondary(
                        ((8, 12, 2), &sec),
                        &quant,
                        &mut deq_a,
                        (&mut compat_u8, stride),
                        &mut scratch_a,
                    )
                    .unwrap()
                );
                assert!(
                    add.add_secondary_with(
                        backend,
                        kernels,
                        ((8, 12, 2), &sec),
                        &quant,
                        &mut deq_b,
                        (&mut typed_u8, stride),
                        &mut scratch_b,
                    )
                    .unwrap()
                );
                assert_eq!(typed_u8, compat_u8);

                let pred_u16: Vec<u16> =
                    (0..w * h).map(|i| ((i * 113 + 7) % 1020) as u16).collect();
                let dc_only = [37i32; 1];
                let mut compat_u16 = vec![3u16; len];
                let mut typed_u16 = vec![3u16; len];
                assert!(add.put_prediction(&pred_u16, BitDepth::Ten, &mut compat_u16, stride,));
                assert!(add.put_prediction_with(
                    backend,
                    &pred_u16,
                    BitDepth::Ten,
                    &mut typed_u16,
                    stride,
                ));
                assert!(add.add(
                    &dc_only,
                    (Some(dq), BitDepth::Ten),
                    &mut compat_u16,
                    stride,
                    &mut scratch_a,
                ));
                assert!(add.add_with(
                    backend,
                    &dc_only,
                    (Some(dq), BitDepth::Ten),
                    &mut typed_u16,
                    stride,
                    &mut scratch_b,
                ));
                assert_eq!(typed_u16, compat_u16);

                assert!(add.add_onto(
                    &pred_u16,
                    &coeffs,
                    (Some(dq), BitDepth::Ten),
                    &mut compat_u16,
                    stride,
                    &mut scratch_a,
                ));
                assert!(add.add_onto_with(
                    backend,
                    &pred_u16,
                    &coeffs,
                    (Some(dq), BitDepth::Ten),
                    &mut typed_u16,
                    stride,
                    &mut scratch_b,
                ));
                assert_eq!(typed_u16, compat_u16);

                let t = Transform8 {
                    bit_depth: BitDepth::Eight,
                    row: InverseTransform2dDim::Kernel(InverseTransform1dType::Dct),
                    col: InverseTransform2dDim::Kernel(InverseTransform1dType::Dct),
                    w,
                    h,
                    rescale: w != h,
                    row_shift: 7,
                    col_shift: 10,
                    dequant: Some(dq),
                };
                let mut res_a = vec![0i32; w * h];
                let mut res_b = vec![0i32; w * h];
                let mut scratch_r_a = [0i32; 32 * 32];
                let mut scratch_r_b = [0i32; 32 * 32];
                assert!(inverse_transform_2d_8bit(
                    t,
                    &coeffs,
                    &mut res_a,
                    &mut scratch_r_a,
                ));
                assert!(inverse_transform_2d_8bit_with(
                    backend,
                    t,
                    &coeffs,
                    &mut res_b,
                    &mut scratch_r_b,
                ));
                assert_eq!(res_b, res_a);
            }
        }

        let resolved = resolve_backend(SimdPolicy::Auto).expect("auto backend");
        check(resolved.backend(), crate::KernelSelection::of(resolved));
    }
}
