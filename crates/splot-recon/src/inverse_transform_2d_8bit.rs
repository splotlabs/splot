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
    Bytes as _, Simd, SimdBase as _, SimdInt, i16x8, i32x4, i32x8, u8x16, u16x8, u32x8, u64x2,
};

use splot_tables::tables::transform_1d::{DCT_KERNEL8, DCT_KERNEL16, DCT_KERNEL32};

use crate::dequant_process::{dequant_coefficient, dequant_lanes};
use crate::inverse_transform::{InverseTransform1dType, Kernel1d, select_kernel};
use crate::inverse_transform_2d::InverseTransform2dDim;
use crate::secondary_transform::{
    SecondaryInverseTransform, secondary_inverse_transform_quant,
    secondary_inverse_transform_quant_rows8,
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

/// Where the column pass writes its `w * h` output in 8-sample chunks:
/// chunk `i` is row `8 * i / w` from column `8 * i % w`, or for a 4-wide
/// block rows `2 * i` and `2 * i + 1`.
pub(crate) trait Output {
    /// Whether the sink adds onto the samples in place: [`Self::put`] applies
    /// the column `Clip3` itself, so the column pass hands it the rounded
    /// sums, and a chunk it never gets is a zero residual.
    const ADDS: bool = false;

    /// Writes chunk `index` of column-pass output.
    fn put<S: Simd>(&mut self, simd: S, index: usize, v: i32x8<S>);
}

impl Output for [i32] {
    #[inline]
    fn put<S: Simd>(&mut self, _: S, index: usize, v: i32x8<S>) {
        if let Some(out) = self.as_chunks_mut::<8>().0.get_mut(index) {
            v.store_slice(out);
        }
    }
}

/// The § 7.14.3 `Clip1(prediction + residual)` of the column pass into a
/// `w`-wide 8-bit block of `dst` with row stride `stride`, in place.
pub(crate) struct AddU8<'a> {
    dst: &'a mut [u8],
    stride: usize,
    /// `log2(w / 8)`, or `None` for a 4-wide block.
    chunks_log2: Option<u32>,
}

impl Output for AddU8<'_> {
    const ADDS: bool = true;

    #[inline]
    fn put<S: Simd>(&mut self, simd: S, index: usize, v: i32x8<S>) {
        let stride = self.stride;
        let Some(log2) = self.chunks_log2 else {
            let first = 2 * index * stride;
            let word = |dst: &[u8], at: usize| {
                dst.get(at..)
                    .and_then(<[u8]>::first_chunk::<4>)
                    .map_or(0, |bytes| u32::from_le_bytes(*bytes))
            };
            let pred =
                u64::from(word(self.dst, first)) | u64::from(word(self.dst, first + stride)) << 32;
            let sum = add_u8x8(simd, pred, v);
            for (at, half) in [(first, sum as u32), (first + stride, (sum >> 32) as u32)] {
                if let Some(out) = self
                    .dst
                    .get_mut(at..)
                    .and_then(<[u8]>::first_chunk_mut::<4>)
                {
                    *out = half.to_le_bytes();
                }
            }
            return;
        };
        let at = (index >> log2) * stride + 8 * (index & ((1 << log2) - 1));
        if let Some(out) = self
            .dst
            .get_mut(at..)
            .and_then(<[u8]>::first_chunk_mut::<8>)
        {
            *out = add_u8x8(simd, u64::from_le_bytes(*out), v).to_le_bytes();
        }
    }
}

/// The § 7.14.3 `Clip1(prediction + residual)` of the column pass into a
/// `w`-wide 16-bit block of `dst` with row stride `stride`, in place, for
/// samples up to `max`.
pub(crate) struct AddU16<'a> {
    dst: &'a mut [u16],
    stride: usize,
    /// `log2(w / 8)`, or `None` for a 4-wide block.
    chunks_log2: Option<u32>,
    max: i16,
}

impl Output for AddU16<'_> {
    const ADDS: bool = true;

    #[inline]
    fn put<S: Simd>(&mut self, simd: S, index: usize, v: i32x8<S>) {
        let (stride, max) = (self.stride, self.max);
        let Some(log2) = self.chunks_log2 else {
            let first = 2 * index * stride;
            let half =
                |dst: &[u16], at: usize| dst.get(at..).and_then(<[u16]>::first_chunk::<4>).copied();
            let (Some(a), Some(b)) = (half(self.dst, first), half(self.dst, first + stride)) else {
                return;
            };
            let sum = add_u16x8(
                simd,
                [a[0], a[1], a[2], a[3], b[0], b[1], b[2], b[3]],
                v,
                max,
            );
            for (at, half) in [(first, &sum[..4]), (first + stride, &sum[4..])] {
                if let Some(out) = self.dst.get_mut(at..at + 4) {
                    out.copy_from_slice(half); // splot-copy-ok: one row of a 4-wide lane group
                }
            }
            return;
        };
        let at = (index >> log2) * stride + 8 * (index & ((1 << log2) - 1));
        if let Some(out) = self
            .dst
            .get_mut(at..)
            .and_then(<[u16]>::first_chunk_mut::<8>)
        {
            *out = add_u16x8(simd, *out, v, max);
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

/// Eight `Clip1(p + Clip3(-2^BitDepth, 2^BitDepth - 1, r))` of `pred` and
/// the column sums `res`, as `Clip1(p + Sat16(r))`: both saturate alike for a
/// `p` up to `max < 2^15`.
#[inline]
fn add_u16x8<S: Simd>(simd: S, pred: [u16; 8], res: i32x8<S>, max: i16) -> [u16; 8] {
    let (lo, hi) = simd.split_i32x8(res);
    let res: i16x8<S> = simd.saturating_narrow_i32x4(lo, hi);
    let pred = u16x8::load_array(simd, pred).bitcast::<i16x8<S>>();
    let sum = simd
        .saturating_add_i16x8(pred, res)
        .max(i16x8::splat(simd, 0))
        .min(i16x8::splat(simd, max));
    sum.bitcast::<u16x8<S>>().into()
}

/// Eight `Clip1(p + Clip3(-256, 255, r))` of the eight bytes of `pred` and
/// the column sums `res`, as `Clip1(p + Sat16(r))`: both saturate alike for
/// an 8-bit `p`.
#[inline]
fn add_u8x8<S: Simd>(simd: S, pred: u64, res: i32x8<S>) -> u64 {
    let (lo, hi) = simd.split_i32x8(res);
    let res: i16x8<S> = simd.saturating_narrow_i32x4(lo, hi);
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
    #[inline]
    pub fn add<T: ReconSample>(
        self,
        coeffs: &[i32],
        (dequant, bit_depth): (Option<FlatDequant>, BitDepth),
        dst: &mut [T],
        stride: usize,
        scratch: &mut [i32; 32 * 32],
    ) -> bool {
        let (w, h) = (self.w(), self.h());
        if !self.fits(dst.len(), stride) || coeffs.len() > w * h {
            return false;
        }
        let Some(dst) = T::u8_slice_mut(dst).filter(|_| bit_depth == BitDepth::Eight) else {
            return self.run_add(
                self.transform8(dequant, bit_depth),
                (coeffs, w),
                dst,
                stride,
                scratch,
            );
        };
        #[cfg(target_arch = "x86_64")]
        if let Some(token) = Level::baseline().as_avx2() {
            let small = self.w_log2 <= 3 && self.h_log2 <= 4;
            if !(small && avx2::add_small_block(token, &self, dequant, coeffs, dst, stride)) {
                avx2::transform(
                    token,
                    &self,
                    dequant,
                    coeffs,
                    false,
                    avx2::Sink::Add { dst, stride },
                    scratch,
                );
            }
            return true;
        }
        #[cfg(target_arch = "aarch64")]
        if let Some(token) = fearless_simd::Level::baseline().as_neon() {
            match coeffs {
                [] => {}
                &[dc] if self.flat => neon::add_flat(token, self, (dc, dequant), (dst, stride)),
                _ if w * h <= 32 => neon::add_small(token, self, (coeffs, dequant), (dst, stride)),
                _ => neon::add(token, self, (coeffs, w, dequant), (dst, stride), scratch),
            }
            return true;
        }
        self.run_add(
            self.transform8(dequant, bit_depth),
            (coeffs, w),
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
        (dequant, bit_depth): (Option<FlatDequant>, BitDepth),
        dst: &mut [T],
        stride: usize,
        scratch: &mut [i32; 32 * 32],
    ) -> bool {
        coeffs.len() <= self.w() * self.h()
            && self.put_prediction(prediction, bit_depth, dst, stride)
            && self.add(coeffs, (dequant, bit_depth), dst, stride, scratch)
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
        let w = self.w();
        let in_range = match T::u16_slice(prediction) {
            Some(samples) => {
                !crate::sample_range::u16_samples_exceed(samples, bit_depth.max_sample())
            }
            None => bit_depth == BitDepth::Eight,
        };
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
        scratch: &mut [i32; 32 * 32],
    ) -> crate::Result<bool> {
        let bit_depth = secondary.bit_depth;
        #[cfg(target_arch = "x86_64")]
        if self.w_log2 >= 4
            && self.row != InverseTransform2dDim::Identity
            && self.fits(dst.len(), stride)
            && bit_depth == BitDepth::Eight
            && let Some(dst) = T::u8_slice_mut(dst)
            && let Some(token) = Level::baseline().as_avx2()
            && let Some(rows) = dequant.first_chunk_mut::<64>()
        {
            let len = secondary_inverse_transform_quant_rows8(quant, quantizers, secondary, rows)?;
            let sink = avx2::Sink::Add { dst, stride };
            avx2::transform(token, &self, None, &rows[..len], true, sink, scratch);
            return Ok(true);
        }
        #[cfg(target_arch = "x86_64")]
        let wide_rows = self.w_log2 >= 4
            && (bit_depth != BitDepth::Eight || Level::baseline().as_avx2().is_none());
        #[cfg(not(target_arch = "x86_64"))]
        let wide_rows = self.w_log2 >= 4;
        if wide_rows
            && self.fits(dst.len(), stride)
            && let Some(rows) = dequant.first_chunk_mut::<64>()
        {
            let len = secondary_inverse_transform_quant_rows8(quant, quantizers, secondary, rows)?;
            #[cfg(target_arch = "aarch64")]
            if let (Some(token), Some(dst), BitDepth::Eight) = (
                fearless_simd::Level::baseline().as_neon(),
                T::u8_slice_mut(&mut *dst),
                bit_depth,
            ) {
                if len > 0 {
                    neon::add(token, self, (&rows[..len], 8, None), (dst, stride), scratch);
                }
                return Ok(true);
            }
            let t = self.transform8(None, bit_depth);
            return Ok(self.run_add(t, (&rows[..len], 8), dst, stride, scratch));
        }
        let coded = secondary_inverse_transform_quant(quant, quantizers, secondary, dequant)?;
        Ok(self.add(&dequant[..coded], (None, bit_depth), dst, stride, scratch))
    }

    /// Runs `t` over `input` onto the block at the start of `dst`, through
    /// the `u8` or `u16` column-pass sink.
    fn run_add<T: ReconSample>(
        self,
        t: Transform8,
        input: (&[i32], usize),
        dst: &mut [T],
        stride: usize,
        scratch: &mut [i32; 32 * 32],
    ) -> bool {
        let w = self.w();
        let chunks_log2 = (w >= 8).then(|| (w / 8).trailing_zeros());
        if let Some(dst) = T::u8_slice_mut(&mut *dst) {
            let sink = &mut AddU8 {
                dst,
                stride,
                chunks_log2,
            };
            return t.bit_depth == BitDepth::Eight && run(t, input, sink, scratch);
        }
        let Some(dst) = T::u16_slice_mut(dst) else {
            return false;
        };
        let max = t.bit_depth.max_sample() as i16;
        if w >= 8
            && input.1 == w
            && let Some(residual) = self.flat_residual(input.0, t.dequant, t.bit_depth)
        {
            let geometry = (w, self.h(), stride);
            baseline_simd!(simd => add_constant_u16(simd, dst, geometry, (residual, max)));
            return true;
        }
        run(
            t,
            input,
            &mut AddU16 {
                dst,
                stride,
                chunks_log2,
                max,
            },
            scratch,
        )
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

/// Runs the transform of `dequant` (a whole-row prefix of the `w * h` block)
/// into the `w * h` `residual`, or returns `false` with nothing written when
/// the shape has no lane form.
pub(crate) fn inverse_transform_2d_8bit(
    t: Transform8,
    dequant: &[i32],
    residual: &mut [i32],
    scratch: &mut [i32; 32 * 32],
) -> bool {
    if residual.len() != t.w * t.h || dequant.len() > t.w * t.h {
        return false;
    }
    #[cfg(target_arch = "x86_64")]
    if let Some(token) = Level::baseline().as_avx2()
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
    run(t, (dequant, t.w), residual, scratch)
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
/// column 8), into `residual`. Returns `false` with nothing written when the
/// shape or a shift has no lane form.
fn run<O: Output + ?Sized>(
    t: Transform8,
    (dequant, in_w): (&[i32], usize),
    residual: &mut O,
    scratch: &mut [i32; 32 * 32],
) -> bool {
    if !valid(&t, dequant) || !(in_w == t.w || (in_w == 8 && t.w >= 16)) {
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
        baseline_simd!(simd => dc_only_transform(simd, t, dc, (row, col), residual));
        return true;
    }
    let (row_bound, col_bound) = bounds(t.bit_depth);
    let rows = Rows {
        input: dequant,
        in_w,
        coded: dequant.len().div_ceil(in_w).min(t.h),
        load: Load {
            rescale: t.rescale,
            dequant: t.dequant,
        },
        shift: u32::from(t.row_shift).min(31),
        bound: row_bound,
    };
    let pass = ColPass {
        w: t.w,
        h: t.h,
        shift: u32::from(t.col_shift).min(31),
        bound: col_bound,
    };
    baseline_simd!(simd => transform(simd, (row, col), rows, pass, residual, scratch));
    true
}

/// The row pass, then the column pass over the row outputs in `scratch`,
/// or for identity columns the row pass straight into `residual`.
#[allow(
    clippy::inline_always,
    reason = "measured § 7.15.4.1 transform hot path"
)]
#[inline(always)]
fn transform<S: Simd, O: Output + ?Sized>(
    simd: S,
    (row, col): (Pass1d, Pass1d),
    rows: Rows<'_>,
    pass: ColPass,
    residual: &mut O,
    scratch: &mut [i32; 32 * 32],
) {
    if col.is_none() {
        let out = &mut IdentityCol {
            inner: residual,
            scale: identity_scale(pass.h),
            shift: pass.shift,
            bound: pass.bound,
        };
        row_dispatch(simd, row, pass.w, rows, out);
        if !O::ADDS {
            for index in (rows.coded * pass.w).div_ceil(8)..pass.h * pass.w / 8 {
                out.inner.put(simd, index, i32x8::splat(simd, 0));
            }
        }
        return;
    }
    row_dispatch(simd, row, pass.w, rows, &mut scratch[..]);
    if pass.h < 32 {
        let chunks = scratch.as_chunks_mut::<8>().0.iter_mut();
        for chunk in chunks
            .take(pass.h * pass.w / 8)
            .skip((rows.coded * pass.w).div_ceil(8))
        {
            *chunk = [0; 8];
        }
    }
    let pass = (pass, rows.coded);
    if pass.0.w == 4 {
        col_dispatch::<_, i32x4<S>, O>(simd, col, scratch, pass, residual);
    } else {
        col_dispatch::<_, i32x8<S>, O>(simd, col, scratch, pass, residual);
    }
}

/// The row pass of `row` over a `w`-wide block into `out`, every coded row
/// written.
#[allow(
    clippy::inline_always,
    reason = "measured § 7.15.4.1 row-pass hot path"
)]
#[inline(always)]
fn row_dispatch<S: Simd, O: Output + ?Sized>(
    simd: S,
    row: Pass1d,
    w: usize,
    rows: Rows<'_>,
    out: &mut O,
) {
    match row {
        None => row_pass_identity(simd, w, rows, out),
        Some((Kernel1d::N4(k), _)) => row_pass_4(simd, k, rows, out),
        Some((Kernel1d::N8(k, true), _)) => row_pass::<_, _, 8, false, true>(simd, k, rows, out),
        Some((Kernel1d::N8(k, false), true)) => {
            row_pass::<_, _, 8, true, false>(simd, k, rows, out);
        }
        Some((Kernel1d::N8(k, false), false)) => {
            row_pass::<_, _, 8, false, false>(simd, k, rows, out);
        }
        Some((Kernel1d::N16(k, true), _)) => row_pass::<_, _, 16, false, true>(simd, k, rows, out),
        Some((Kernel1d::N16(k, false), true)) => {
            row_pass::<_, _, 16, true, false>(simd, k, rows, out);
        }
        Some((Kernel1d::N16(k, false), false)) => {
            row_pass::<_, _, 16, false, false>(simd, k, rows, out);
        }
        Some((Kernel1d::N32(k), _)) => row_pass::<_, _, 32, true, false>(simd, k, rows, out),
    }
}

/// The § 7.15.2.3 identity column pass applied to each row-pass chunk as it
/// is written: an identity column output depends on its own sample only.
struct IdentityCol<'a, O: ?Sized> {
    inner: &'a mut O,
    scale: i32,
    shift: u32,
    bound: i32,
}

impl<O: Output + ?Sized> Output for IdentityCol<'_, O> {
    #[inline]
    fn put<S: Simd>(&mut self, simd: S, index: usize, v: i32x8<S>) {
        let v = clip_lanes(simd, v * self.scale, self.shift, self.bound, !O::ADDS);
        self.inner.put(simd, index, v);
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
fn dc_only_transform<S: Simd, O: Output + ?Sized>(
    simd: S,
    t: Transform8,
    dc: i32,
    (row, col): (Pass1d, Pass1d),
    residual: &mut O,
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
        clip_row(i32x8::from_slice(simd, &k) * dc, row_shift, row_bound)
    };
    let dct = |dim| dim == InverseTransform2dDim::Kernel(InverseTransform1dType::Dct);
    if dct(t.row) && dct(t.col) {
        let value: [i32; 8] = clip_col(row0(0) * kc[0], col_shift, col_bound).into();
        let value = i32x8::splat(simd, value[0]);
        for index in 0..t.w * t.h / 8 {
            residual.put(simd, index, value);
        }
        return;
    }
    if t.w == 4 {
        let (row0, _) = simd.split_i32x8(row0(0));
        let row0 = simd.combine_i32x4(row0, row0);
        for (index, &[a, b]) in kc.as_chunks::<2>().0.iter().take(t.h / 2).enumerate() {
            let k = i32x8::from_slice(simd, &[a, a, a, a, b, b, b, b]);
            residual.put(simd, index, clip_col(row0 * k, col_shift, col_bound));
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
            residual.put(
                simd,
                r * chunks + c,
                clip_col(row0 * k, col_shift, col_bound),
            );
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

/// The row `Clip3` to `±bound` of `Round2(sum, shift)`.
#[inline]
fn clip_row<S: Simd>(sum: i32x8<S>, shift: u32, bound: i32) -> i32x8<S> {
    clip_lanes(sum.simd, sum, shift, bound, true)
}

/// The column `Clip3` to `±bound` of `Round2(sum, shift)`.
#[inline]
fn clip_col<S: Simd>(sum: i32x8<S>, shift: u32, bound: i32) -> i32x8<S> {
    clip_lanes(sum.simd, sum, shift, bound, true)
}

/// `Round2(sum, shift)` over `V` lanes for `shift <= 31` and `|sum| < 2^30`
/// (zero at 31), then with `clip` its `Clip3` to `±bound`.
#[inline]
fn clip_lanes<S: Simd, V: SimdInt<S, Element = i32>>(
    simd: S,
    sum: V,
    shift: u32,
    bound: i32,
    clip: bool,
) -> V {
    let rounded = (sum + ((1u32 << shift) >> 1) as i32) >> shift;
    if clip {
        rounded
            .max(V::splat(simd, -bound))
            .min(V::splat(simd, bound - 1))
    } else {
        rounded
    }
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
#[allow(
    clippy::inline_always,
    reason = "measured § 7.15.4.1 row-pass hot path"
)]
#[inline(always)]
fn row_pass<S: Simd, O: Output + ?Sized, const N: usize, const SPLIT: bool, const REV: bool>(
    simd: S,
    kernel: &[[i32; N]; N],
    rows: Rows<'_>,
    out: &mut O,
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
                let v = clip_row(simd.combine_i32x4(y[0], y[1]), rows.shift, rows.bound);
                out.put(simd, r * N / 8 + j, v);
            }
        }
    }
}

/// [`row_pass`] of a 4-wide block, two rows per input load.
fn row_pass_4<S: Simd, O: Output + ?Sized>(
    simd: S,
    kernel: &[[i32; 4]; 4],
    rows: Rows<'_>,
    out: &mut O,
) {
    let k = kernel.map(|k| i32x4::from_slice(simd, &k));
    let transform = |x: i32x4<S>| k[0] * x[0] + k[1] * x[1] + k[2] * x[2] + k[3] * x[3];
    for q in 0..rows.coded.div_ceil(2) {
        let (a, b) = simd.split_i32x8(rows.inputs(simd, rows.input, q, q == 0));
        let sum = simd.combine_i32x4(transform(a), transform(b));
        out.put(simd, q, clip_row(sum, rows.shift, rows.bound));
    }
}

/// [`row_pass`] for the § 7.15.2.3 identity transform: each row output is
/// its own clamped input times the identity scale, in the row `Clip3`.
fn row_pass_identity<S: Simd, O: Output + ?Sized>(simd: S, w: usize, rows: Rows<'_>, out: &mut O) {
    let scale = identity_scale(w);
    let scaled = |v: i32x8<S>| clip_row(v * scale, rows.shift, rows.bound);
    if rows.in_w == w {
        for k in 0..(rows.coded * w).div_ceil(8) {
            out.put(simd, k, scaled(rows.inputs(simd, rows.input, k, k == 0)));
        }
        return;
    }
    for r in 0..rows.coded {
        let row = input_row(rows.input, r, rows.in_w);
        for k in 0..w / 8 {
            let v = rows.inputs(simd, row, k, r == 0 && k == 0);
            out.put(simd, r * w / 8 + k, scaled(v));
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
    fn put<O: Output + ?Sized>(simd: S, at: (usize, usize, usize), v: [Self; 4], out: &mut O);
}

impl<S: Simd> Lanes<S> for i32x4<S> {
    #[allow(
        clippy::inline_always,
        reason = "keeps the column outputs in registers"
    )]
    #[inline(always)]
    fn put<O: Output + ?Sized>(
        simd: S,
        (_, _, start): (usize, usize, usize),
        v: [Self; 4],
        out: &mut O,
    ) {
        out.put(simd, start / 2, simd.combine_i32x4(v[0], v[1]));
        out.put(simd, start / 2 + 1, simd.combine_i32x4(v[2], v[3]));
    }
}

impl<S: Simd> Lanes<S> for i32x8<S> {
    #[allow(
        clippy::inline_always,
        reason = "keeps the column outputs in registers"
    )]
    #[inline(always)]
    fn put<O: Output + ?Sized>(
        simd: S,
        (w, c, start): (usize, usize, usize),
        v: [Self; 4],
        out: &mut O,
    ) {
        let chunks = w / 8;
        for (row, v) in (start..).zip(v) {
            out.put(simd, row * chunks + c, v);
        }
    }
}

/// The kernel column pass of `col` over the first `coded` rows of
/// `scratch`, the rows below them zero up to a height below 32. Identity
/// columns run inside the row pass instead ([`IdentityCol`]).
#[allow(
    clippy::inline_always,
    reason = "measured § 7.15.4.1 column-pass hot path"
)]
#[inline(always)]
fn col_dispatch<S: Simd, V: Lanes<S>, O: Output + ?Sized>(
    simd: S,
    col: Pass1d,
    scratch: &[i32; 32 * 32],
    pass: (ColPass, usize),
    residual: &mut O,
) {
    match col {
        None => {}
        Some((Kernel1d::N4(k), _)) => {
            col_pass::<_, V, _, 4, false, false>(simd, k, scratch, pass, residual);
        }
        Some((Kernel1d::N8(k, true), _)) => {
            col_pass::<_, V, _, 8, false, true>(simd, k, scratch, pass, residual);
        }
        Some((Kernel1d::N8(k, false), true)) => {
            col_pass::<_, V, _, 8, true, false>(simd, k, scratch, pass, residual);
        }
        Some((Kernel1d::N8(k, false), false)) => {
            col_pass::<_, V, _, 8, false, false>(simd, k, scratch, pass, residual);
        }
        Some((Kernel1d::N16(k, true), _)) => {
            col_pass::<_, V, _, 16, false, true>(simd, k, scratch, pass, residual);
        }
        Some((Kernel1d::N16(k, false), true)) => {
            col_pass::<_, V, _, 16, true, false>(simd, k, scratch, pass, residual);
        }
        Some((Kernel1d::N16(k, false), false)) => {
            col_pass::<_, V, _, 16, false, false>(simd, k, scratch, pass, residual);
        }
        Some((Kernel1d::N32(k), _)) => {
            col_pass::<_, V, _, 32, true, false>(simd, k, scratch, pass, residual);
        }
    }
}

/// The column pass of an `M`-tall block over the `V` column lanes of the
/// first `coded` rows of `scratch`, four outputs at a time: each output lane
/// takes one multiply-add per row, every row for `M < 32` (a fixed count
/// beats a data-dependent one there). `SPLIT` and `REV` are as in
/// [`row_pass`], over the rows.
#[allow(
    clippy::inline_always,
    reason = "measured § 7.15.4.1 column-pass hot path"
)]
#[inline(always)]
fn col_pass<
    S: Simd,
    V: Lanes<S>,
    O: Output + ?Sized,
    const M: usize,
    const SPLIT: bool,
    const REV: bool,
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
    residual: &mut O,
) {
    let rows = if M < 32 { M } else { coded.min(M) };
    let zero = V::splat(simd, 0);
    let outputs = if SPLIT { M / 2 } else { M };
    let clip = |v: V| clip_lanes(simd, v, shift, bound, !O::ADDS);
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
                let low = core::array::from_fn(|o| clip(first[o] + second[o]));
                V::put(simd, (w, c, base), low, residual);
                let high = core::array::from_fn(|o| clip(first[3 - o] - second[3 - o]));
                V::put(simd, (w, c, M - 4 - base), high, residual);
            } else if REV {
                let high = core::array::from_fn(|o| clip(first[3 - o]));
                V::put(simd, (w, c, M - 4 - base), high, residual);
            } else {
                V::put(simd, (w, c, base), first.map(clip), residual);
            }
        }
    }
}
