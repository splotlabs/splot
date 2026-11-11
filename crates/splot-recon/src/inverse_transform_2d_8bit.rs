// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 § 7.15.4.1 kernel-by-kernel 2D matrix transform at 8-bit depth.
//!
//! At `BitDepth == 8` every § 7.15.2.1 input lies in the row clamp
//! `[-2^15, 2^15 - 1]`, every row output in the row `Clip3` bound of the same
//! range, and every kernel entry below `2^7`, so `|sum| < 32 * 2^7 * 2^15`
//! never wraps an `i32` and the re-associated sums are the scalar sums.
//!
//! The lane form here is one `i32` multiply-add per input and output, four
//! outputs per vector: the row pass vectorizes over a row's outputs, the
//! column pass over the block's columns, and the `DCT` passes split into
//! even and odd halves. The AVX2 form in [`avx2`] runs on `pmaddwd` pairs
//! instead.

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

#[cfg(target_arch = "x86_64")]
mod avx2;

/// One 8-bit § 7.15.4.1 kernel-by-kernel transform.
#[derive(Clone, Copy)]
pub(crate) struct Transform8 {
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

/// The § 7.14.4 `useQm == 0` dequantization of an 8-bit transform block with
/// a power-of-two `dq_denom`, applied as the row pass loads its input.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FlatDequant {
    dc: u32,
    ac: u32,
    denom_shift: u32,
    bound: i32,
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
            bound: 1 << (7 + 8),
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
    /// dequantized when the input is quantized; `dc` marks lane 0 as the DC
    /// coefficient. No rescale or clamp.
    #[inline]
    fn coeffs<S: Simd>(self, simd: S, row: &[i32], start: usize, dc: bool) -> i32x8<S> {
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
        self.dequantize(simd, v, dc)
    }

    #[inline]
    fn dequantize<S: Simd>(self, simd: S, v: i32x8<S>, dc: bool) -> i32x8<S> {
        let Some(dq) = self.dequant else {
            return v;
        };
        let q2 = if dc {
            let ac = dq.ac;
            u32x8::load_array(simd, [dq.dc, ac, ac, ac, ac, ac, ac, ac])
        } else {
            u32x8::splat(simd, dq.ac)
        };
        dequant_lanes(simd, v, q2, dq.denom_shift, dq.bound)
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

/// One transform shape and type that has the fused 8-bit form of the
/// § 7.15.4 process: the § 7.15.4.1 transform added in place as the § 7.14.3
/// `Clip1(prediction + Residual)`. A block resolves it with one table lookup.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InverseTransformAdd8 {
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
}

/// [`InverseTransformAdd8`] by `[log2W - 2][log2H - 2][PlaneTxType][useDdt]`
/// for sides of 4 to 32 samples.
static FUSED: [[[[Option<InverseTransformAdd8>; 2]; 16]; 4]; 4] = {
    let mut out = [[[[None; 2]; 16]; 4]; 4];
    let mut i = 0;
    while i < 4 * 4 * 16 * 2 {
        let (w, h, tx_type, ddt) = (i / 128, i / 32 % 4, i / 2 % 16, i % 2 == 1);
        out[w][h][tx_type][ddt as usize] =
            InverseTransformAdd8::resolve(tx_type, w as u32 + 2, h as u32 + 2, ddt);
        i += 1;
    }
    out
};

impl InverseTransformAdd8 {
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

    /// Adds the transform of `coeffs` to the 8-bit block at the start of
    /// `dst`, whose rows are `stride` apart.
    ///
    /// With `dequant`, `coeffs` holds the quantized coefficients in raster
    /// order up to the last nonzero one, dequantized as they load; with
    /// `None` it is an already dequantized prefix of the block. Returns
    /// `false` with `dst` untouched when `coeffs` is longer than the block or
    /// the block does not fit `dst`.
    #[inline]
    pub fn add(
        self,
        coeffs: &[i32],
        dequant: Option<FlatDequant>,
        dst: &mut [u8],
        stride: usize,
        scratch: &mut [i32; 32 * 32],
    ) -> bool {
        let (w, h) = (self.w(), self.h());
        if !self.fits(dst, stride) || coeffs.len() > w * h {
            return false;
        }
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
        run(
            self.transform8(dequant),
            (coeffs, w),
            &mut self.add_u8(dst, stride),
            scratch,
        )
    }

    /// Adds the transform of the § 7.15.3 secondary inverse transform of
    /// `quant`, under the flat § 7.14.4 dequantization by the `(dc_quant,
    /// ac_quant, dq_denom)` `quantizers`, to the 8-bit block at the start of
    /// `dst` as [`Self::add`] does, with the block's `Dequant` in `dequant`.
    /// A block at least 16 wide, without an identity row pass, keeps only the
    /// 8 columns the secondary transform writes and runs the row pass
    /// straight from them.
    ///
    /// # Errors
    /// Returns the [`secondary_inverse_transform`](crate::secondary_inverse_transform)
    /// errors for invalid `secondary` parameters or a `dequant` that is not
    /// the block.
    pub fn add_secondary(
        self,
        (quantizers, secondary): ((u32, u32, u32), &SecondaryInverseTransform),
        quant: &[i32],
        dequant: &mut [i32],
        (dst, stride): (&mut [u8], usize),
        scratch: &mut [i32; 32 * 32],
    ) -> crate::Result<bool> {
        #[cfg(target_arch = "x86_64")]
        if self.w_log2 >= 4
            && self.row != InverseTransform2dDim::Identity
            && self.fits(dst, stride)
            && let Some(token) = Level::baseline().as_avx2()
            && let Some(rows) = dequant.first_chunk_mut::<64>()
        {
            let len = secondary_inverse_transform_quant_rows8(quant, quantizers, secondary, rows)?;
            let sink = avx2::Sink::Add { dst, stride };
            avx2::transform(token, &self, None, &rows[..len], true, sink, scratch);
            return Ok(true);
        }
        #[cfg(not(target_arch = "x86_64"))]
        if self.w_log2 >= 4
            && self.fits(dst, stride)
            && let Some(rows) = dequant.first_chunk_mut::<64>()
        {
            let len = secondary_inverse_transform_quant_rows8(quant, quantizers, secondary, rows)?;
            let sink = &mut self.add_u8(dst, stride);
            return Ok(run(self.transform8(None), (&rows[..len], 8), sink, scratch));
        }
        let coded = secondary_inverse_transform_quant(quant, quantizers, secondary, dequant)?;
        Ok(self.add(&dequant[..coded], None, dst, stride, scratch))
    }

    /// The column-pass sink adding to the block at the start of `dst`.
    fn add_u8(self, dst: &mut [u8], stride: usize) -> AddU8<'_> {
        let w = self.w();
        AddU8 {
            dst,
            stride,
            chunks_log2: (w >= 8).then(|| (w / 8).trailing_zeros()),
        }
    }

    /// Whether a `w x h` block with rows `stride` apart fits `dst`.
    fn fits(self, dst: &[u8], stride: usize) -> bool {
        (self.h() - 1)
            .checked_mul(stride)
            .and_then(|span| span.checked_add(self.w()))
            .is_some_and(|span| stride >= self.w() && span <= dst.len())
    }

    const fn transform8(self, dequant: Option<FlatDequant>) -> Transform8 {
        Transform8 {
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

    /// The constant residual of a `DCT_DCT` block whose only nonzero
    /// coefficient is the first of `coeffs` (as [`Self::add`] takes them), or
    /// `None` when the block's residual is not constant.
    #[inline]
    #[must_use]
    pub fn flat_residual(&self, coeffs: &[i32], dequant: Option<FlatDequant>) -> Option<i32> {
        if !self.flat {
            return None;
        }
        let (&dc, rest) = coeffs.split_first()?;
        if rest.len() >= self.w() || rest.iter().any(|&coeff| coeff != 0) {
            return None;
        }
        let dc = dequant.map_or(dc, |dq| {
            dequant_coefficient(dc, dq.dc, 1 << dq.denom_shift, crate::BitDepth::Eight)
        });
        let round = |v: i32, shift: u8| (v + (1 << (shift - 1))) >> shift;
        let dc = if self.rescale {
            dc.wrapping_mul(2896).wrapping_add(1 << 11) >> 12
        } else {
            dc
        };
        let dc = dc.clamp(i16::MIN.into(), i16::MAX.into());
        let row = round(dc * 64, self.row_shift).clamp(i16::MIN.into(), i16::MAX.into());
        Some(round(row * 64, self.col_shift).clamp(-256, 255))
    }
}

/// Runs the transform of `dequant` (a whole-row prefix of the `w * h` block)
/// into the `w * h` `residual`, or returns `false` with nothing written when
/// the shape or a shift has no lane form.
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
    if let Some(token) = Level::baseline().as_avx2() {
        let shape = InverseTransformAdd8::of(
            t.row,
            t.col,
            (t.w.trailing_zeros(), t.h.trailing_zeros()),
            (t.row_shift, t.col_shift),
            t.rescale,
        );
        let Some(shape) = shape.filter(|s| s.w() == t.w && s.h() == t.h) else {
            return false;
        };
        let sink = avx2::Sink::Residual(residual);
        avx2::transform(token, &shape, t.dequant, dequant, false, sink, scratch);
        return true;
    }
    run(t, (dequant, t.w), residual, scratch)
}

/// Whether `t` and `dequant` have a lane form.
fn valid(t: &Transform8, dequant: &[i32]) -> bool {
    let shifts = 1..=20u8;
    shifts.contains(&t.row_shift)
        && shifts.contains(&t.col_shift)
        && dequant.len() <= t.w * t.h
        && matches!(t.w, 4 | 8 | 16 | 32)
        && matches!(t.h, 4 | 8 | 16 | 32)
}

/// The DC coefficient of a block whose only non-zero coefficient it is, with
/// input rows `in_w` apart.
fn dc_only(t: &Transform8, dequant: &[i32], in_w: usize) -> Option<i32> {
    let (&dc, rest) = dequant.split_first()?;
    (rest.len() < in_w && rest.iter().all(|&coeff| coeff == 0)).then(|| {
        t.dequant.map_or(dc, |dq| {
            dequant_coefficient(dc, dq.dc, 1 << dq.denom_shift, crate::BitDepth::Eight)
        })
    })
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
    let rows = Rows {
        input: dequant,
        in_w,
        coded: dequant.len().div_ceil(in_w).min(t.h),
        load: Load {
            rescale: t.rescale,
            dequant: t.dequant,
        },
        shift: u32::from(t.row_shift),
    };
    let pass = ColPass {
        w: t.w,
        h: t.h,
        shift: u32::from(t.col_shift),
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
}

impl<O: Output + ?Sized> Output for IdentityCol<'_, O> {
    #[inline]
    fn put<S: Simd>(&mut self, simd: S, index: usize, v: i32x8<S>) {
        let v = clip_lanes(simd, v * self.scale, self.shift, !O::ADDS);
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
    let dc = row_input(i32x8::splat(simd, dc), t.rescale);
    let (row_shift, col_shift) = (u32::from(t.row_shift), u32::from(t.col_shift));
    let kr = first_kernel_row(row, t.w);
    let kc = first_kernel_row(col, t.h);
    let row0 = |c: usize| {
        let k = kr.as_chunks::<8>().0.get(c).map_or([0; 8], |k| *k);
        clip_row(i32x8::from_slice(simd, &k) * dc, row_shift)
    };
    let dct = |dim| dim == InverseTransform2dDim::Kernel(InverseTransform1dType::Dct);
    if dct(t.row) && dct(t.col) {
        let value: [i32; 8] = clip_col(row0(0) * kc[0], col_shift).into();
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
            residual.put(simd, index, clip_col(row0 * k, col_shift));
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
            residual.put(simd, r * chunks + c, clip_col(row0 * k, col_shift));
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

/// The optional rescale and the row input clamp.
#[inline]
fn row_input<S: Simd>(v: i32x8<S>, rescale: bool) -> i32x8<S> {
    let v = rescaled(v, rescale);
    v.max(i32x8::splat(v.simd, i32::from(i16::MIN)))
        .min(i32x8::splat(v.simd, i32::from(i16::MAX)))
}

/// `Round2(sum, shift)` for `1 <= shift <= 20` and `|sum| < 2^28`.
#[inline]
fn round<S: Simd>(sum: i32x8<S>, shift: u32) -> i32x8<S> {
    (sum + (1 << (shift - 1))) >> shift
}

/// The row `Clip3` of `Round2(sum, shift)`.
#[inline]
fn clip_row<S: Simd>(sum: i32x8<S>, shift: u32) -> i32x8<S> {
    round(sum, shift)
        .max(i32x8::splat(sum.simd, i32::from(i16::MIN)))
        .min(i32x8::splat(sum.simd, i32::from(i16::MAX)))
}

/// The column `Clip3` of `Round2(sum, shift)`.
#[inline]
fn clip_col<S: Simd>(sum: i32x8<S>, shift: u32) -> i32x8<S> {
    clip_lanes(sum.simd, sum, shift, true)
}

/// [`clip_col`] over `V` lanes, or only its `Round2` without `clip`.
#[inline]
fn clip_lanes<S: Simd, V: SimdInt<S, Element = i32>>(simd: S, sum: V, shift: u32, clip: bool) -> V {
    let rounded = (sum + (1 << (shift - 1))) >> shift;
    if clip {
        rounded.max(V::splat(simd, -256)).min(V::splat(simd, 255))
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
    /// Row-pass down-shift.
    shift: u32,
}

impl Rows<'_> {
    /// The clamped § 7.15.2.1 inputs of the eight coefficients of `row` from
    /// `8 * k`; `dc` marks lane 0 as the DC coefficient.
    #[inline]
    fn inputs<S: Simd>(self, simd: S, row: &[i32], k: usize, dc: bool) -> i32x8<S> {
        row_input(self.load.coeffs(simd, row, 8 * k, dc), self.load.rescale)
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
                let v = clip_row(simd.combine_i32x4(y[0], y[1]), rows.shift);
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
        out.put(simd, q, clip_row(sum, rows.shift));
    }
}

/// [`row_pass`] for the § 7.15.2.3 identity transform: each row output is
/// its own clamped input times the identity scale, in the row `Clip3`.
fn row_pass_identity<S: Simd, O: Output + ?Sized>(simd: S, w: usize, rows: Rows<'_>, out: &mut O) {
    let scale = identity_scale(w);
    let scaled = |v: i32x8<S>| clip_row(v * scale, rows.shift);
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
    /// Column-pass down-shift.
    shift: u32,
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
    (ColPass { w, shift, .. }, coded): (ColPass, usize),
    residual: &mut O,
) {
    let rows = if M < 32 { M } else { coded.min(M) };
    let zero = V::splat(simd, 0);
    let outputs = if SPLIT { M / 2 } else { M };
    let clip = |v: V| clip_lanes(simd, v, shift, !O::ADDS);
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
