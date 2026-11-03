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

use std::simd::cmp::{SimdOrd as _, SimdPartialEq as _};
use std::simd::num::SimdInt as _;
use std::simd::{Simd, ToBytes as _, simd_swizzle};

use splot_tables::tables::transform_1d::{
    ADST_KERNEL4, ADST_KERNEL8, ADST_KERNEL16, DCT_KERNEL4, DCT_KERNEL8, DCT_KERNEL16,
    DCT_KERNEL32, DDTX_KERNEL8, DDTX_KERNEL16, FDST_KERNEL4, FDST_KERNEL8, FDST_KERNEL16,
};

use crate::inverse_transform::InverseTransform1dType;

type Sum = Simd<i32, 8>;
type Pairs = Simd<i16, 16>;

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
    pub row: InverseTransform1dType,
    pub col: InverseTransform1dType,
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
    let (Some(row), Some(col)) = (select(t.w, t.row), select(t.h, t.col)) else {
        return false;
    };
    let split = t.h >= 8 && (t.col == InverseTransform1dType::Dct || t.h == 32);
    let col = if split { select_split(t.h) } else { Some(col) };
    let Some(col) = col else {
        return false;
    };
    let (row_shift, rows) = (u32::from(t.row_shift), (t.h, split));
    let mask = match row {
        PairKernel::N4(k) => row_pass_4(k, dequant, t.rescale, row_shift, rows, scratch),
        PairKernel::N8(k) => row_pass::<8, 4>(k, dequant, t.rescale, row_shift, rows, scratch),
        PairKernel::N16(k) => row_pass::<16, 8>(k, dequant, t.rescale, row_shift, rows, scratch),
        PairKernel::N32(k) => row_pass::<32, 16>(k, dequant, t.rescale, row_shift, rows, scratch),
    };
    let (shift, w) = (u32::from(t.col_shift), t.w);
    match (col, split) {
        (PairKernel::N4(k), _) => col_pass::<4, 2, false>(k, scratch, w, mask, shift, residual),
        (PairKernel::N8(k), false) => col_pass::<8, 4, false>(k, scratch, w, mask, shift, residual),
        (PairKernel::N8(k), true) => col_pass::<8, 4, true>(k, scratch, w, mask, shift, residual),
        (PairKernel::N16(k), false) => {
            col_pass::<16, 8, false>(k, scratch, w, mask, shift, residual);
        }
        (PairKernel::N16(k), true) => col_pass::<16, 8, true>(k, scratch, w, mask, shift, residual),
        (PairKernel::N32(k), _) => col_pass::<32, 16, true>(k, scratch, w, mask, shift, residual),
    }
    true
}

/// `pmaddwd`: the sum of each adjacent pair of `i16` products.
#[inline]
fn madd(x: Pairs, w: Pairs) -> Sum {
    let product = x.cast::<i32>() * w.cast::<i32>();
    simd_swizzle!(product, [0, 2, 4, 6, 8, 10, 12, 14])
        + simd_swizzle!(product, [1, 3, 5, 7, 9, 11, 13, 15])
}

#[inline]
fn pairs(packed: Simd<u32, 8>) -> Pairs {
    Pairs::from_ne_bytes(packed.to_ne_bytes())
}

#[inline]
fn splat_pair(packed: u32) -> Pairs {
    pairs(Simd::splat(packed))
}

/// `Round2(sum, shift)` for `1 <= shift <= 20` and `|sum| < 2^28`.
#[inline]
fn round(sum: Sum, shift: u32) -> Sum {
    (sum + Simd::splat(1 << (shift - 1))) >> Simd::splat(shift as i32)
}

/// Rows `a` and `b` of the row `Clip3`, packed `a` low and `b` high.
#[inline]
fn pack_rows(a: Sum, b: Sum, shift: u32) -> Simd<i32, 8> {
    let clip = |sum| {
        round(sum, shift)
            .simd_max(Simd::splat(i32::from(i16::MIN)))
            .simd_min(Simd::splat(i32::from(i16::MAX)))
    };
    (clip(a) & Simd::splat(0xffff)) | (clip(b) << Simd::splat(16))
}

/// The row input clamp (after the optional rescale) of up to 32 row samples
/// as packed column pairs, with one past the last non-zero pair.
#[inline]
fn row_input_pairs(row: &[i32], rescale: bool) -> ([u32; 16], usize) {
    let mut out = [0u32; 16];
    let mut nonzero = 0u32;
    let clamp = |v: Simd<i32, 8>| {
        let v = if rescale {
            (v * Simd::splat(2896) + Simd::splat(1 << 11)) >> Simd::splat(12)
        } else {
            v
        };
        v.simd_max(Simd::splat(i32::from(i16::MIN)))
            .simd_min(Simd::splat(i32::from(i16::MAX)))
            .cast::<i16>()
    };
    let (chunks, tail) = row.as_chunks::<8>();
    for (index, (chunk, slot)) in chunks.iter().zip(out.chunks_exact_mut(4)).enumerate() {
        let packed = Simd::<u32, 4>::from_ne_bytes(clamp(Simd::from_array(*chunk)).to_ne_bytes());
        nonzero |= (packed.simd_ne(Simd::splat(0)).to_bitmask() as u32) << (4 * index);
        slot.copy_from_slice(packed.as_array()); // splot-copy-ok: packed row pairs
    }
    if let Some(&[a, b, c, d]) = tail.first_chunk::<4>() {
        let wide = Simd::from_array([a, b, c, d, 0, 0, 0, 0]);
        let packed = Simd::<u32, 4>::from_ne_bytes(clamp(wide).to_ne_bytes());
        nonzero |= packed.simd_ne(Simd::splat(0)).to_bitmask() as u32 & 0b11;
        out[..2].copy_from_slice(&packed.as_array()[..2]); // splot-copy-ok: packed row pairs
    }
    (out, (u32::BITS - nonzero.leading_zeros()) as usize)
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
fn row_pass<const N: usize, const H: usize>(
    kernel: &[[u32; N]; H],
    dequant: &[i32],
    rescale: bool,
    shift: u32,
    (h, split): (usize, bool),
    scratch: &mut [i32; 32 * 32],
) -> u32 {
    let rows = dequant.as_chunks::<N>().0;
    let input = |row: usize| {
        rows.get(row)
            .map_or(([0; 16], 0), |row| row_input_pairs(row, rescale))
    };
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
        let mut sum_a = [Sum::splat(0); 4];
        let mut sum_b = [Sum::splat(0); 4];
        for ((weights, &a), &b) in kernel[..used].iter().zip(&a).zip(&b) {
            let (a, b) = (splat_pair(a), splat_pair(b));
            for ((weights, sum_a), sum_b) in weights
                .as_chunks::<8>()
                .0
                .iter()
                .zip(&mut sum_a)
                .zip(&mut sum_b)
            {
                let weights = pairs(Simd::from_array(*weights));
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
            *out = pack_rows(a, b, shift).to_array();
        }
    }
    mask
}

/// [`row_pass`] for a 4-wide block: each sum holds row `a` in its low half and
/// row `b` in its high half.
fn row_pass_4(
    kernel: &[[u32; 4]; 2],
    dequant: &[i32],
    rescale: bool,
    shift: u32,
    (h, split): (usize, bool),
    scratch: &mut [i32; 32 * 32],
) -> u32 {
    let rows = dequant.as_chunks::<4>().0;
    let weights =
        kernel.map(|w| pairs(simd_swizzle!(Simd::from_array(w), [0, 1, 2, 3, 0, 1, 2, 3])));
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
        let (pair, used) = row_input_pairs(&[a0, a1, a2, a3, b0, b1, b2, b3], rescale);
        if used == 0 {
            continue;
        }
        mask |= 1 << q;
        let mut sum = Sum::splat(0);
        for (p, weights) in weights.iter().enumerate() {
            let (a, b) = (pair[p], pair[2 + p]);
            sum += madd(pairs(Simd::from_array([a, a, a, a, b, b, b, b])), *weights);
        }
        let packed = pack_rows(
            simd_swizzle!(sum, [0, 1, 2, 3, 0, 1, 2, 3]),
            simd_swizzle!(sum, [4, 5, 6, 7, 4, 5, 6, 7]),
            shift,
        );
        *out = simd_swizzle!(packed, [0, 1, 2, 3]).to_array();
    }
    mask
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
fn col_pass<const M: usize, const HM: usize, const SPLIT: bool>(
    kernel: &[[u32; M]; HM],
    scratch: &[i32; 32 * 32],
    w: usize,
    mask: u32,
    shift: u32,
    residual: &mut [i32],
) {
    let emit = |sum: Sum| {
        round(sum, shift)
            .simd_max(Simd::splat(-256))
            .simd_min(Simd::splat(255))
            .to_array()
    };
    let list = |mask: u32| {
        let mut list = [0usize; 16];
        let mut count = 0;
        let mut bits = mask & ((1 << HM) - 1);
        while bits != 0 {
            list[count] = bits.trailing_zeros() as usize;
            bits &= bits - 1;
            count += 1;
        }
        (list, count)
    };
    let (even, odd) = if SPLIT {
        (list(mask & 0x5555), list(mask & 0xaaaa))
    } else {
        (list(mask), ([0; 16], 0))
    };
    let (even, odd) = (&even.0[..even.1], &odd.0[..odd.1]);
    let outputs = if SPLIT { M / 2 } else { M };
    let out = residual.as_chunks_mut::<8>().0;
    if w == 4 {
        let pair_sum = |list: &[usize], r: usize| {
            let mut sum = Sum::splat(0);
            for &q in list {
                let Some(z) = scratch.get(q * 4..q * 4 + 4) else {
                    continue;
                };
                let z = Simd::<i32, 4>::from_slice(z);
                let z = pairs(simd_swizzle!(z, [0, 1, 2, 3, 0, 1, 2, 3]).cast());
                let (a, b) = (kernel[q][r], kernel[q][r + 1]);
                sum += madd(z, pairs(Simd::from_array([a, a, a, a, b, b, b, b])));
            }
            sum
        };
        for r in (0..outputs).step_by(2) {
            let (e, o) = (pair_sum(even, r), pair_sum(odd, r));
            if let Some(out) = out.get_mut(r / 2) {
                *out = emit(e + o);
            }
            if SPLIT && let Some(out) = out.get_mut((M - 2 - r) / 2) {
                *out = emit(simd_swizzle!(e - o, [4, 5, 6, 7, 0, 1, 2, 3]));
            }
        }
        return;
    }
    let chunks = w / 8;
    let sums = |list: &[usize], c: usize, r: usize| {
        let mut sums = [Sum::splat(0); 4];
        for &q in list {
            let Some(z) = scratch.as_chunks::<8>().0.get(q * chunks + c) else {
                continue;
            };
            let z = pairs(Simd::from_array(*z).cast());
            for (offset, sum) in sums.iter_mut().enumerate() {
                *sum += madd(z, splat_pair(kernel[q][r + offset]));
            }
        }
        sums
    };
    for c in 0..chunks {
        for r in (0..outputs).step_by(4) {
            let (e, o) = (sums(even, c, r), sums(odd, c, r));
            for (offset, (&e, &o)) in e.iter().zip(&o).enumerate() {
                if let Some(out) = out.get_mut((r + offset) * chunks + c) {
                    *out = emit(e + o);
                }
                if SPLIT && let Some(out) = out.get_mut((M - 1 - r - offset) * chunks + c) {
                    *out = emit(e - o);
                }
            }
        }
    }
}
