// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! AV2 § 7.15.4.1 kernel-by-kernel 2D matrix transform at 8-bit depth, in
//! 16-bit multiply-add pairs.
//!
//! At `BitDepth == 8` every § 7.15.2.1 input lies in the row clamp
//! `[-2^15, 2^15 - 1]`, every row output in the row `Clip3` bound of the same
//! range, and every kernel entry below `2^7`, so each product fits an `i16`
//! multiply with an `i32` sum and the transform runs on `pmaddwd` pairs of
//! adjacent kernel rows. The `i32` sums are the scalar sums re-associated,
//! which is exact under wrapping `i32` addition, and `|sum| < 32 * 2^7 * 2^15`
//! never wraps.
//!
//! The row pass works on two rows at once and stores them as one `u32` pair
//! per column (row `2q` low, row `2q + 1` high), which is already the operand
//! of the column pass's multiply-add over rows `2q` and `2q + 1`.

use fearless_simd::{
    Bytes as _, Level, Simd, SimdBase as _, SimdIntWiden as _, SimdMask as _, SimdNarrow as _,
    i16x16, i32x4, i32x8, u8x32, u32x8,
};

#[cfg(target_arch = "x86_64")]
use core::arch::x86_64::{__m256i, _mm256_packs_epi32, _mm256_setr_epi8, _mm256_shuffle_epi8};
#[cfg(target_arch = "x86_64")]
use fearless_simd::{Avx2, SimdInto as _};
use splot_tables::tables::transform_1d::{
    ADST_KERNEL4, ADST_KERNEL8, ADST_KERNEL16, DCT_KERNEL4, DCT_KERNEL8, DCT_KERNEL16,
    DCT_KERNEL32, DDTX_KERNEL8, DDTX_KERNEL16, FDST_KERNEL4, FDST_KERNEL8, FDST_KERNEL16,
};

use crate::inverse_transform::InverseTransform1dType;
use crate::inverse_transform_2d::InverseTransform2dDim;

/// Entry `[q][i]` packs `kernel[2q][i]` (low half) and `kernel[2q + 1][i]`
/// (high half), with the output index reversed for `FDDT`.
const fn pair_kernel<const N: usize, const H: usize>(
    kernel: &[[i32; N]; N],
    reversed: bool,
) -> [[u32; N]; H] {
    assert!(2 * H == N);
    let mut out = [[0u32; N]; H];
    let mut q = 0;
    while q < H {
        let mut i = 0;
        while i < N {
            let column = if reversed { N - 1 - i } else { i };
            let (low, high) = (kernel[2 * q][column], kernel[2 * q + 1][column]);
            assert!(low == low as i16 as i32 && high == high as i16 as i32);
            out[q][i] = (low as u16 as u32) | ((high as u16 as u32) << 16);
            i += 1;
        }
        q += 1;
    }
    out
}

static DCT4: [[u32; 4]; 2] = pair_kernel(&DCT_KERNEL4, false);
static ADST4: [[u32; 4]; 2] = pair_kernel(&ADST_KERNEL4, false);
static FDST4: [[u32; 4]; 2] = pair_kernel(&FDST_KERNEL4, false);
static DCT8: [[u32; 8]; 4] = pair_kernel(&DCT_KERNEL8, false);
static ADST8: [[u32; 8]; 4] = pair_kernel(&ADST_KERNEL8, false);
static FDST8: [[u32; 8]; 4] = pair_kernel(&FDST_KERNEL8, false);
static DDTX8: [[u32; 8]; 4] = pair_kernel(&DDTX_KERNEL8, false);
static FDDT8: [[u32; 8]; 4] = pair_kernel(&DDTX_KERNEL8, true);
static DCT16: [[u32; 16]; 8] = pair_kernel(&DCT_KERNEL16, false);
static ADST16: [[u32; 16]; 8] = pair_kernel(&ADST_KERNEL16, false);
static FDST16: [[u32; 16]; 8] = pair_kernel(&FDST_KERNEL16, false);
static DDTX16: [[u32; 16]; 8] = pair_kernel(&DDTX_KERNEL16, false);
static FDDT16: [[u32; 16]; 8] = pair_kernel(&DDTX_KERNEL16, true);
static DCT32: [[u32; 32]; 16] = pair_kernel(&DCT_KERNEL32, false);
static DCT8_SPLIT: [[u32; 8]; 4] = split_pair_kernel(&DCT_KERNEL8);
static DCT16_SPLIT: [[u32; 16]; 8] = split_pair_kernel(&DCT_KERNEL16);
static DCT32_SPLIT: [[u32; 32]; 16] = split_pair_kernel(&DCT_KERNEL32);

/// [`pair_kernel`] over the [`pair_rows`] of a split `DCT` column pass, for
/// the first `N / 2` outputs.
const fn split_pair_kernel<const N: usize, const H: usize>(
    kernel: &[[i32; N]; N],
) -> [[u32; N]; H] {
    assert!(2 * H == N);
    let mut out = [[0u32; N]; H];
    let mut q = 0;
    while q < H {
        let (a, b) = pair_rows(q, true);
        let mut i = 0;
        while i < N / 2 {
            let (low, high) = (kernel[a][i], kernel[b][i]);
            assert!(low == low as i16 as i32 && high == high as i16 as i32);
            assert!(kernel[a][N - 1 - i] == low * (1 - 2 * (a % 2) as i32));
            assert!(kernel[b][N - 1 - i] == high * (1 - 2 * (b % 2) as i32));
            out[q][i] = (low as u16 as u32) | ((high as u16 as u32) << 16);
            i += 1;
        }
        q += 1;
    }
    out
}

fn select_split(len: usize) -> Option<PairKernel> {
    Some(match len {
        8 => PairKernel::N8(&DCT8_SPLIT),
        16 => PairKernel::N16(&DCT16_SPLIT),
        32 => PairKernel::N32(&DCT32_SPLIT),
        _ => return None,
    })
}

/// The paired § 7.15.2.1 kernel of one 1D length, with the same dispatch as
/// the scalar kernel selection.
#[derive(Clone, Copy)]
enum PairKernel {
    N4(&'static [[u32; 4]; 2]),
    N8(&'static [[u32; 8]; 4]),
    N16(&'static [[u32; 16]; 8]),
    N32(&'static [[u32; 32]; 16]),
}

fn select(len: usize, tx_type: InverseTransform1dType) -> Option<PairKernel> {
    use InverseTransform1dType::{Adst, Dct, Ddtx, Fddt, Fdst};
    Some(match (len, tx_type) {
        (4, Dct) => PairKernel::N4(&DCT4),
        (4, Adst) => PairKernel::N4(&ADST4),
        (4, Fdst | Ddtx | Fddt) => PairKernel::N4(&FDST4),
        (8, Dct) => PairKernel::N8(&DCT8),
        (8, Adst) => PairKernel::N8(&ADST8),
        (8, Fdst) => PairKernel::N8(&FDST8),
        (8, Ddtx) => PairKernel::N8(&DDTX8),
        (8, Fddt) => PairKernel::N8(&FDDT8),
        (16, Dct) => PairKernel::N16(&DCT16),
        (16, Adst) => PairKernel::N16(&ADST16),
        (16, Fdst) => PairKernel::N16(&FDST16),
        (16, Ddtx) => PairKernel::N16(&DDTX16),
        (16, Fddt) => PairKernel::N16(&FDDT16),
        (32, _) => PairKernel::N32(&DCT32),
        _ => return None,
    })
}

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
}

/// Runs the transform of `dequant` (a whole-row prefix of the `w * h` block)
/// into the `w * h` `residual`, or returns `false` with nothing written when
/// the shape or a shift has no paired form.
pub(crate) fn inverse_transform_2d_8bit(
    t: Transform8,
    dequant: &[i32],
    residual: &mut [i32],
    scratch: &mut [i32; 32 * 32],
) -> bool {
    let shifts = 1..=20u8;
    if !shifts.contains(&t.row_shift)
        || !shifts.contains(&t.col_shift)
        || dequant.len() > t.w * t.h
        || residual.len() != t.w * t.h
    {
        return false;
    }
    let kernel = |dim, len| match dim {
        InverseTransform2dDim::Kernel(tx_type) => select(len, tx_type).map(Some),
        InverseTransform2dDim::Identity => Some(None),
    };
    let (Some(row), Some(col)) = (kernel(t.row, t.w), kernel(t.col, t.h)) else {
        return false;
    };
    let dct = |dim| dim == InverseTransform2dDim::Kernel(InverseTransform1dType::Dct);
    let split = t.h >= 8 && col.is_some() && (dct(t.col) || t.h == 32);
    let col = match col {
        Some(_) if split => select_split(t.h).map(Some),
        col => Some(col),
    };
    let Some(col) = col else {
        return false;
    };
    if let Some((&dc, rest)) = dequant.split_first()
        && rest.len() + 1 == t.w
        && rest.iter().all(|&coeff| coeff == 0)
    {
        let flat = dct(t.row) && dct(t.col);
        baseline_simd!(simd => dc_only(simd, t, dc, flat, residual));
        return true;
    }
    let plan = Plan {
        t,
        row,
        row_dct: dct(t.row),
        col,
        split,
    };
    baseline_simd!(simd => transform(simd, plan, dequant, residual, scratch));
    true
}

/// The resolved kernels of one [`Transform8`].
#[derive(Clone, Copy)]
struct Plan {
    t: Transform8,
    row: Option<PairKernel>,
    row_dct: bool,
    col: Option<PairKernel>,
    split: bool,
}

#[inline]
fn transform<S: Simd>(
    simd: S,
    plan: Plan,
    dequant: &[i32],
    residual: &mut [i32],
    scratch: &mut [i32; 32 * 32],
) {
    let Plan {
        t,
        row,
        row_dct,
        col,
        split,
    } = plan;
    let (row_shift, rows) = (u32::from(t.row_shift), (t.h, split));
    let (rescale, input) = (t.rescale, dequant);
    let mask = match (row, row_dct) {
        (None, _) => row_pass_identity(simd, t.w, input, rescale, row_shift, rows, scratch),
        (Some(PairKernel::N4(k)), _) => {
            row_pass_4(simd, k, input, rescale, row_shift, rows, scratch)
        }
        (Some(PairKernel::N8(k)), _) => {
            row_pass::<_, 8, 4, false>(simd, k, input, rescale, row_shift, rows, scratch)
        }
        (Some(PairKernel::N16(_)), true) => {
            row_pass::<_, 16, 8, true>(simd, &DCT16_SPLIT, input, rescale, row_shift, rows, scratch)
        }
        (Some(PairKernel::N16(k)), false) => {
            row_pass::<_, 16, 8, false>(simd, k, input, rescale, row_shift, rows, scratch)
        }
        (Some(PairKernel::N32(_)), _) => row_pass::<_, 32, 16, true>(
            simd,
            &DCT32_SPLIT,
            input,
            rescale,
            row_shift,
            rows,
            scratch,
        ),
    };
    let (shift, w) = (u32::from(t.col_shift), t.w);
    let Some(col) = col else {
        col_pass_identity(simd, (w, t.h), scratch, mask, shift, residual);
        return;
    };
    let pass = ColPass { w, mask, shift };
    match (col, split) {
        (PairKernel::N4(k), _) => col_pass::<_, 4, 2, false>(simd, k, scratch, pass, residual),
        (PairKernel::N8(k), false) => col_pass::<_, 8, 4, false>(simd, k, scratch, pass, residual),
        (PairKernel::N8(k), true) => col_pass::<_, 8, 4, true>(simd, k, scratch, pass, residual),
        (PairKernel::N16(k), false) => {
            col_pass::<_, 16, 8, false>(simd, k, scratch, pass, residual);
        }
        (PairKernel::N16(k), true) => col_pass::<_, 16, 8, true>(simd, k, scratch, pass, residual),
        (PairKernel::N32(k), _) => col_pass::<_, 32, 16, true>(simd, k, scratch, pass, residual),
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

/// Row 0 of the paired § 7.15.2.1 kernel of `dim` at length `len` (the
/// identity scale at index 0 for the § 7.15.2.3 identity), as packed
/// multiply-add pairs `(k, 0)`.
fn first_kernel_row(dim: InverseTransform2dDim, len: usize) -> [u32; 32] {
    let mut row = [0u32; 32];
    let kernel: &[u32] = match dim {
        InverseTransform2dDim::Identity => {
            row[0] = identity_scale(len) as u32;
            return row;
        }
        InverseTransform2dDim::Kernel(tx_type) => match select(len, tx_type) {
            Some(PairKernel::N4(k)) => &k[0],
            Some(PairKernel::N8(k)) => &k[0],
            Some(PairKernel::N16(k)) => &k[0],
            Some(PairKernel::N32(k)) => &k[0],
            None => &[],
        },
    };
    for (slot, &pair) in row.iter_mut().zip(kernel) {
        *slot = pair & 0xffff;
    }
    row
}

/// The transform of a block whose only non-zero coefficient is `dc`: the
/// row pass leaves only row 0, `Clip3(Round2(dc * kr[c], rowShift))` for
/// row-kernel row 0 `kr`, and every column sum has the one term
/// `kc[r] * row0[c]` for column-kernel row 0 `kc`. With `flat` (`DCT` both
/// ways) every kernel row 0 is constant and so is the residual.
fn dc_only<S: Simd>(simd: S, t: Transform8, dc: i32, flat: bool, residual: &mut [i32]) {
    let dc = row_input(i32x8::splat(simd, dc), t.rescale).bitcast::<i16x16<S>>();
    let (row_shift, col_shift) = (u32::from(t.row_shift), u32::from(t.col_shift));
    let row = first_kernel_row(t.row, t.w);
    let col = first_kernel_row(t.col, t.h);
    let row0 = |c: usize| {
        let k = row.as_chunks::<8>().0.get(c).map_or([0; 8], |k| *k);
        clip_row(madd(dc, pairs(simd, &k)), row_shift).bitcast::<i16x16<S>>()
    };
    if flat {
        let value: [i32; 8] = clip_col(madd(row0(0), splat_pair(simd, col[0])), col_shift).into();
        residual.fill(value[0]);
        return;
    }
    if t.w == 4 {
        let row0 = row0(0).bitcast::<i32x8<S>>();
        let (row0, _) = simd.split_i32x8(row0);
        let row0 = simd.combine_i32x4(row0, row0).bitcast();
        for (out, k) in residual
            .as_chunks_mut::<8>()
            .0
            .iter_mut()
            .zip(col.as_chunks::<2>().0)
        {
            let [a, b] = *k;
            clip_col(
                madd(row0, pairs(simd, &[a, a, a, a, b, b, b, b])),
                col_shift,
            )
            .store_slice(out);
        }
        return;
    }
    let chunks = t.w / 8;
    let mut row0s = [i16x16::splat(simd, 0); 4];
    for (c, slot) in row0s.iter_mut().enumerate().take(chunks) {
        *slot = row0(c);
    }
    for (out, &k) in residual.chunks_exact_mut(t.w).zip(&col) {
        let k = splat_pair(simd, k);
        for (out, &row0) in out.as_chunks_mut::<8>().0.iter_mut().zip(&row0s) {
            clip_col(madd(row0, k), col_shift).store_slice(out);
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

/// [`row_pass`] for the § 7.15.2.3 identity transform: each row output is
/// its own clamped input times the identity scale, in the row `Clip3`.
fn row_pass_identity<S: Simd>(
    simd: S,
    w: usize,
    dequant: &[i32],
    rescale: bool,
    shift: u32,
    (h, split): (usize, bool),
    scratch: &mut [i32; 32 * 32],
) -> u32 {
    let scale = identity_scale(w);
    let scaled = |v: i32x8<S>| row_input(v, rescale) * scale;
    let coded = dequant.len() / w;
    let mut mask = 0u32;
    let mut padded = [0i32; 8];
    for (q, out) in scratch.chunks_exact_mut(w).take(h / 2).enumerate() {
        let (row_a, row_b) = pair_rows(q, split);
        if row_a >= coded {
            break;
        }
        let row = |row: usize| dequant.get(row * w..(row + 1) * w).unwrap_or(&[]);
        let (a, b) = (row(row_a), row(row_b));
        if a.iter().chain(b).all(|&coeff| coeff == 0) {
            continue;
        }
        mask |= 1 << q;
        let lanes = |row: &[i32], c: usize| {
            row.get(c..c + 8)
                .map_or(i32x8::splat(simd, 0), |row| i32x8::from_slice(simd, row))
        };
        if w == 4 {
            let pick = |row: &[i32]| row.first_chunk::<4>().copied().unwrap_or([0; 4]);
            padded[..4].copy_from_slice(&pick(a)); // splot-copy-ok: one identity row pair
            padded[4..].copy_from_slice(&pick(b)); // splot-copy-ok: one identity row pair
            let sum: [i32; 8] = clip_row(scaled(i32x8::from_slice(simd, &padded)), shift).into();
            for (i, out) in out.iter_mut().take(4).enumerate() {
                *out = (sum[i] & 0xffff) | (sum[4 + i] << 16);
            }
            continue;
        }
        for (c, out) in out.as_chunks_mut::<8>().0.iter_mut().enumerate() {
            let (a, b) = (scaled(lanes(a, 8 * c)), scaled(lanes(b, 8 * c)));
            pack_rows(a, b, shift).store_slice(out);
        }
    }
    mask
}

/// [`col_pass`] for the § 7.15.2.3 identity transform over the adjacent row
/// pairs of `scratch`: each output is its own row-pass sample times the
/// identity scale, in the column `Clip3`.
fn col_pass_identity<S: Simd>(
    simd: S,
    (w, h): (usize, usize),
    scratch: &[i32; 32 * 32],
    mask: u32,
    shift: u32,
    residual: &mut [i32],
) {
    let scale = identity_scale(h);
    let emit = |packed: i32x8<S>, high: bool| {
        let v = if high {
            packed >> 16
        } else {
            (packed << 16) >> 16
        };
        clip_col(v * scale, shift)
    };
    for (q, (pair, out)) in scratch
        .chunks_exact(w)
        .zip(residual.chunks_exact_mut(2 * w))
        .take(h / 2)
        .enumerate()
    {
        let (row_a, row_b) = out.split_at_mut(w);
        if mask & (1 << q) == 0 {
            out.fill(0);
            continue;
        }
        if w == 4 {
            let Some(&[z0, z1, z2, z3]) = pair.first_chunk::<4>() else {
                continue;
            };
            let packed = i32x8::from_slice(simd, &[z0, z1, z2, z3, z0, z1, z2, z3]);
            let (a, b): ([i32; 8], [i32; 8]) =
                (emit(packed, false).into(), emit(packed, true).into());
            row_a.copy_from_slice(&a[..4]); // splot-copy-ok: one identity output row
            row_b.copy_from_slice(&b[..4]); // splot-copy-ok: one identity output row
            continue;
        }
        for ((packed, a), b) in pair
            .as_chunks::<8>()
            .0
            .iter()
            .zip(row_a.as_chunks_mut::<8>().0)
            .zip(row_b.as_chunks_mut::<8>().0)
        {
            let packed = i32x8::from_slice(simd, packed);
            emit(packed, false).store_slice(a);
            emit(packed, true).store_slice(b);
        }
    }
}

/// `pmaddwd`: the sum of each adjacent pair of `i16` products.
#[inline]
fn madd<S: Simd>(x: i16x16<S>, w: i16x16<S>) -> i32x8<S> {
    x.pairwise_dot_product(w)
}

#[inline]
fn pairs<S: Simd>(simd: S, packed: &[u32; 8]) -> i16x16<S> {
    u32x8::from_slice(simd, packed).bitcast()
}

#[inline]
fn splat_pair<S: Simd>(simd: S, packed: u32) -> i16x16<S> {
    u32x8::splat(simd, packed).bitcast()
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
    round(sum, shift)
        .max(i32x8::splat(sum.simd, -256))
        .min(i32x8::splat(sum.simd, 255))
}

/// Rows `a` and `b` of the row `Clip3`, packed `a` low and `b` high.
#[inline]
fn pack_rows<S: Simd>(a: i32x8<S>, b: i32x8<S>, shift: u32) -> i32x8<S> {
    #[cfg(target_arch = "x86_64")]
    if let Some(token) = Level::baseline().as_avx2() {
        let (a, b) = (round(a, shift), round(b, shift));
        let packed = pack_rows_avx2(
            token,
            i32x8::from_slice(token, a.as_slice()).into(),
            i32x8::from_slice(token, b.as_slice()).into(),
        );
        let packed: i32x8<Avx2> = packed.simd_into(token);
        return i32x8::from_slice(a.simd, packed.as_slice());
    }
    (clip_row(a, shift) & 0xffff) | (clip_row(b, shift) << 16)
}

#[cfg(target_arch = "x86_64")]
fearless_simd::kernel!(
    /// `packssdw` saturates both rows to the row `Clip3` within each 128-bit
    /// lane, and `pshufb` interleaves them column by column.
    #[inline]
    fn pack_rows_avx2(token: Avx2, a: __m256i, b: __m256i) -> __m256i {
        let order = _mm256_setr_epi8(
            0, 1, 8, 9, 2, 3, 10, 11, 4, 5, 12, 13, 6, 7, 14, 15, 0, 1, 8, 9, 2, 3, 10, 11, 4, 5,
            12, 13, 6, 7, 14, 15,
        );
        _mm256_shuffle_epi8(_mm256_packs_epi32(a, b), order)
    }
);

/// The `pshufb` that swaps the middle two samples of every four.
const SPLIT_ORDER: [u8; 32] = {
    let mut order = [0u8; 32];
    let mut i = 0;
    while i < 32 {
        let sample = i / 2;
        let swapped = match sample % 4 {
            1 => sample + 1,
            2 => sample - 1,
            _ => sample,
        };
        order[i] = (2 * swapped + i % 2) as u8;
        i += 1;
    }
    order
};

/// The row input clamp (after the optional rescale, as the `packssdw`
/// saturation) of up to 32 row samples as packed pairs, with one past the last non-zero pair: adjacent samples,
/// or with `split` samples `4m + j` and `4m + j + 2` as pair `2m + j`.
#[inline]
fn row_input_pairs<S: Simd>(
    simd: S,
    row: &[i32],
    rescale: bool,
    split: bool,
) -> ([u32; 16], usize) {
    let mut out = [0u32; 16];
    let mut nonzero = 0u64;
    let lanes = |i: usize| {
        if let Some(chunk) = row.get(8 * i..8 * i + 8) {
            return rescaled(i32x8::from_slice(simd, chunk), rescale);
        }
        let mut padded = [0i32; 8];
        if let Some(tail) = row.get(8 * i..8 * i + 4) {
            padded[..4].copy_from_slice(tail); // splot-copy-ok: pad a 4-sample row
        }
        rescaled(i32x8::from_slice(simd, &padded), rescale)
    };
    for (index, slot) in out
        .as_chunks_mut::<8>()
        .0
        .iter_mut()
        .take(row.len().div_ceil(16))
        .enumerate()
    {
        let mut packed = lanes(2 * index)
            .saturating_narrow(lanes(2 * index + 1))
            .bitcast::<u8x32<S>>();
        if split {
            packed = packed.swizzle_dyn_within_blocks(u8x32::from_slice(simd, &SPLIT_ORDER));
        }
        let packed = packed.bitcast::<u32x8<S>>();
        nonzero |= packed.simd_ne(u32x8::splat(simd, 0)).to_bitmask() << (8 * index);
        packed.store_slice(slot);
    }
    (out, (u64::BITS - nonzero.leading_zeros()) as usize)
}

/// The two rows the row pass packs as column pair `q`: adjacent rows, or for
/// a split `DCT` column pass rows `4m + j` and `4m + j + 2` (`q = 2m + j`), so
/// that every pair holds two even or two odd rows.
#[inline]
const fn pair_rows(q: usize, split: bool) -> (usize, usize) {
    if split {
        let row = 4 * (q / 2) + q % 2;
        (row, row + 2)
    } else {
        (2 * q, 2 * q + 1)
    }
}

/// The row pass of an `N`-wide, `h`-tall block, `H = N / 2`, for `N >= 8`:
/// writes column pair `q` (rows [`pair_rows`]) at `scratch[q * N..][..N]` and
/// returns the mask of written pairs.
#[allow(
    clippy::inline_always,
    reason = "measured § 7.15.4.1 row-pass hot path"
)]
#[inline(always)]
fn row_pass<S: Simd, const N: usize, const H: usize, const SPLIT: bool>(
    simd: S,
    kernel: &[[u32; N]; H],
    dequant: &[i32],
    rescale: bool,
    shift: u32,
    (h, split): (usize, bool),
    scratch: &mut [i32; 32 * 32],
) -> u32 {
    let rows = dequant.as_chunks::<N>().0;
    let input = |row: usize| {
        rows.get(row).map_or(([0; 16], 0), |row| {
            row_input_pairs(simd, row, rescale, SPLIT)
        })
    };
    let zero = i32x8::splat(simd, 0);
    let mut mask = 0u32;
    for (q, out) in scratch.chunks_exact_mut(N).take(h / 2).enumerate() {
        let (row_a, row_b) = pair_rows(q, split);
        if row_a >= rows.len() {
            break;
        }
        let ((a, used_a), (b, used_b)) = (input(row_a), input(row_b));
        let used = used_a.max(used_b).min(H);
        if used == 0 {
            continue;
        }
        mask |= 1 << q;
        if SPLIT {
            row_pair_split(simd, kernel, (&a, &b), used, shift, out);
            continue;
        }
        let mut sum_a = [zero; 4];
        let mut sum_b = [zero; 4];
        for ((weights, &a), &b) in kernel[..used].iter().zip(&a).zip(&b) {
            let (a, b) = (splat_pair(simd, a), splat_pair(simd, b));
            for ((weights, sum_a), sum_b) in weights
                .as_chunks::<8>()
                .0
                .iter()
                .zip(&mut sum_a)
                .zip(&mut sum_b)
            {
                let weights = pairs(simd, weights);
                *sum_a += madd(a, weights);
                *sum_b += madd(b, weights);
            }
        }
        for ((out, &a), &b) in out
            .as_chunks_mut::<8>()
            .0
            .iter_mut()
            .zip(&sum_a)
            .zip(&sum_b)
        {
            pack_rows(a, b, shift).store_slice(out);
        }
    }
    mask
}

/// One row pair of a split `DCT` [`row_pass`]: the even-sample pairs sum `E`
/// and the odd-sample pairs `O` over the first `N / 2` outputs, and output
/// `N - 1 - i` is `E - O` where output `i` is `E + O`.
#[allow(
    clippy::inline_always,
    reason = "measured § 7.15.4.1 row-pass hot path"
)]
#[inline(always)]
fn row_pair_split<S: Simd, const N: usize, const H: usize>(
    simd: S,
    kernel: &[[u32; N]; H],
    (a, b): (&[u32; 16], &[u32; 16]),
    used: usize,
    shift: u32,
    out: &mut [i32],
) {
    let chunks = N / 16;
    let mut sums = [[[i32x8::splat(simd, 0); 2]; 2]; 2];
    let used = used.next_multiple_of(2).min(H);
    for (p, weights) in kernel[..used].as_chunks::<2>().0.iter().enumerate() {
        for (parity, weights) in weights.iter().enumerate() {
            let inputs = [a[2 * p + parity], b[2 * p + parity]].map(|x| splat_pair(simd, x));
            for (row, &input) in inputs.iter().enumerate() {
                for (sum, weights) in sums[row][parity]
                    .iter_mut()
                    .zip(weights.as_chunks::<8>().0)
                    .take(chunks)
                {
                    *sum += madd(input, pairs(simd, weights));
                }
            }
        }
    }
    let out = out.as_chunks_mut::<8>().0;
    let [[a_even, a_odd], [b_even, b_odd]] = sums;
    for chunk in 0..chunks {
        let (a_e, a_o, b_e, b_o) = (a_even[chunk], a_odd[chunk], b_even[chunk], b_odd[chunk]);
        if let Some(out) = out.get_mut(chunk) {
            pack_rows(a_e + a_o, b_e + b_o, shift).store_slice(out);
        }
        if let Some(out) = out.get_mut(N / 8 - 1 - chunk) {
            pack_rows((a_e - a_o).reverse(), (b_e - b_o).reverse(), shift).store_slice(out);
        }
    }
}

/// [`row_pass`] for a 4-wide block: each sum holds row `a` in its low half and
/// row `b` in its high half.
fn row_pass_4<S: Simd>(
    simd: S,
    kernel: &[[u32; 4]; 2],
    dequant: &[i32],
    rescale: bool,
    shift: u32,
    (h, split): (usize, bool),
    scratch: &mut [i32; 32 * 32],
) -> u32 {
    let rows = dequant.as_chunks::<4>().0;
    let weights = kernel.map(|[w0, w1, w2, w3]| pairs(simd, &[w0, w1, w2, w3, w0, w1, w2, w3]));
    let mut mask = 0u32;
    for (q, out) in scratch
        .as_chunks_mut::<4>()
        .0
        .iter_mut()
        .take(h / 2)
        .enumerate()
    {
        let (row_a, row_b) = pair_rows(q, split);
        let Some(&[a0, a1, a2, a3]) = rows.get(row_a) else {
            break;
        };
        let [b0, b1, b2, b3] = rows.get(row_b).copied().unwrap_or([0; 4]);
        let (pair, used) = row_input_pairs(simd, &[a0, a1, a2, a3, b0, b1, b2, b3], rescale, false);
        if used == 0 {
            continue;
        }
        mask |= 1 << q;
        let mut sum = i32x8::splat(simd, 0);
        for (p, weights) in weights.iter().enumerate() {
            let (a, b) = (pair[p], pair[2 + p]);
            sum += madd(pairs(simd, &[a, a, a, a, b, b, b, b]), *weights);
        }
        let sum: [i32; 8] = clip_row(sum, shift).into();
        for (i, out) in out.iter_mut().enumerate() {
            *out = (sum[i] & 0xffff) | (sum[4 + i] << 16);
        }
    }
    mask
}

/// The column-pair geometry of one [`col_pass`].
#[derive(Clone, Copy)]
struct ColPass {
    /// Block width.
    w: usize,
    /// The written column pairs of the row pass.
    mask: u32,
    /// Column-pass down-shift.
    shift: u32,
}

/// The column pass of an `M`-tall block, `HM = M / 2`, over the column-pair
/// `scratch` of a `w`-wide block, reading only the pairs in `mask`.
///
/// `SPLIT` takes the `DCT` symmetry `kernel[r][M - 1 - i] == (-1)^r *
/// kernel[r][i]`: the even-row pairs sum `E` and the odd-row pairs `O` over
/// the first `M / 2` outputs, and output `M - 1 - i` is `E - O` where output
/// `i` is `E + O`.
#[allow(
    clippy::inline_always,
    reason = "measured § 7.15.4.1 column-pass hot path"
)]
#[inline(always)]
fn col_pass<S: Simd, const M: usize, const HM: usize, const SPLIT: bool>(
    simd: S,
    kernel: &[[u32; M]; HM],
    scratch: &[i32; 32 * 32],
    ColPass { w, mask, shift }: ColPass,
    residual: &mut [i32],
) {
    let mask = mask & ((1 << HM) - 1);
    let kernel = &kernel[..(u32::BITS - mask.leading_zeros()) as usize];
    let (pair_kernels, last) = kernel.as_chunks::<2>();
    let outputs = if SPLIT { M / 2 } else { M };
    let out = residual.as_chunks_mut::<8>().0;
    let zero = i32x8::splat(simd, 0);
    let written = |q: usize| mask & (1 << q) != 0;
    if w == 4 {
        let step = |q: usize, k: &[u32; M], r: usize, sum: &mut i32x8<S>| {
            let (Some(z), true) = (scratch.get(q * 4..q * 4 + 4), written(q)) else {
                return;
            };
            let z = i32x4::from_slice(simd, z);
            let (a, b) = (k[r], k[r + 1]);
            *sum += madd(
                simd.combine_i32x4(z, z).bitcast(),
                pairs(simd, &[a, a, a, a, b, b, b, b]),
            );
        };
        for r in (0..outputs).step_by(2) {
            let (mut e, mut o) = (zero, zero);
            for (p, [k0, k1]) in pair_kernels.iter().enumerate() {
                step(2 * p, k0, r, &mut e);
                step(2 * p + 1, k1, r, if SPLIT { &mut o } else { &mut e });
            }
            if let [k0] = last {
                step(kernel.len() - 1, k0, r, &mut e);
            }
            if let Some(out) = out.get_mut(r / 2) {
                clip_col(e + o, shift).store_slice(out);
            }
            if SPLIT && let Some(out) = out.get_mut((M - 2 - r) / 2) {
                let d: [i32; 8] = clip_col(e - o, shift).into();
                out[..4].copy_from_slice(&d[4..]); // splot-copy-ok: one mirrored output row
                out[4..].copy_from_slice(&d[..4]); // splot-copy-ok: one mirrored output row
            }
        }
        return;
    }
    let chunks = w / 8;
    let columns = scratch.as_chunks::<8>().0;
    let step = |q: usize, k: &[u32; M], c: usize, r: usize, sums: &mut [i32x8<S>; 4]| {
        let (Some(z), Some(k), true) = (columns.get(q * chunks + c), k.get(r..r + 4), written(q))
        else {
            return;
        };
        let z = i32x8::from_slice(simd, z).bitcast();
        for (sum, &k) in sums.iter_mut().zip(k) {
            *sum += madd(z, splat_pair(simd, k));
        }
    };
    for c in 0..chunks {
        for r in (0..outputs).step_by(4) {
            let (mut e, mut o) = ([zero; 4], [zero; 4]);
            for (p, [k0, k1]) in pair_kernels.iter().enumerate() {
                step(2 * p, k0, c, r, &mut e);
                step(2 * p + 1, k1, c, r, if SPLIT { &mut o } else { &mut e });
            }
            if let [k0] = last {
                step(kernel.len() - 1, k0, c, r, &mut e);
            }
            for (offset, (&e, &o)) in e.iter().zip(&o).enumerate() {
                if let Some(out) = out.get_mut((r + offset) * chunks + c) {
                    clip_col(e + o, shift).store_slice(out);
                }
                if SPLIT && let Some(out) = out.get_mut((M - 1 - r - offset) * chunks + c) {
                    clip_col(e - o, shift).store_slice(out);
                }
            }
        }
    }
}
