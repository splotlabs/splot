// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! SAD, SSE, and Hadamard SATD over strided `u16` blocks.
//!
//! The SIMD forms run one row at a time in `min(width, 16)` lanes (rows are 4,
//! 8, or a multiple of 16 samples wide). Every sum is exact integer arithmetic
//! that cannot overflow for `u16` inputs, so lane order does not change a
//! result: `|d| <= 65535` keeps a SAD lane below `2^27`, `d * d` fits `u32`
//! before it widens into `u64` lanes, and an 8x8 Hadamard coefficient stays
//! below `64 * 65535`.

use std::simd::num::{SimdInt as _, SimdUint as _};
use std::simd::{Simd, cmp::SimdOrd as _, simd_swizzle};

use super::BlockDims;

/// The first `dims.height()` rows of a strided block, each `dims.width()` wide.
fn rows(samples: &[u16], stride: usize, dims: BlockDims) -> impl Iterator<Item = &[u16]> {
    debug_assert!(stride >= dims.width());
    samples
        .chunks(stride.max(1))
        .take(dims.height())
        .filter_map(move |row| row.get(..dims.width()))
}

fn row_pairs<'a>(
    src: &'a [u16],
    src_stride: usize,
    pred: &'a [u16],
    pred_stride: usize,
    dims: BlockDims,
) -> impl Iterator<Item = (&'a [u16], &'a [u16])> {
    rows(src, src_stride, dims).zip(rows(pred, pred_stride, dims))
}

pub(super) fn sad_scalar(
    src: &[u16],
    src_stride: usize,
    pred: &[u16],
    pred_stride: usize,
    dims: BlockDims,
) -> u32 {
    row_pairs(src, src_stride, pred, pred_stride, dims)
        .flat_map(|(a, b)| a.iter().zip(b))
        .map(|(&a, &b)| u32::from(a.abs_diff(b)))
        .sum()
}

pub(super) fn sse_scalar(
    src: &[u16],
    src_stride: usize,
    pred: &[u16],
    pred_stride: usize,
    dims: BlockDims,
) -> u64 {
    row_pairs(src, src_stride, pred, pred_stride, dims)
        .flat_map(|(a, b)| a.iter().zip(b))
        .map(|(&a, &b)| {
            let d = u64::from(a.abs_diff(b));
            d * d
        })
        .sum()
}

pub(super) fn sad_simd(
    src: &[u16],
    src_stride: usize,
    pred: &[u16],
    pred_stride: usize,
    dims: BlockDims,
) -> u32 {
    diff_sum_simd(src, src_stride, pred, pred_stride, dims, false) as u32
}

pub(super) fn sse_simd(
    src: &[u16],
    src_stride: usize,
    pred: &[u16],
    pred_stride: usize,
    dims: BlockDims,
) -> u64 {
    diff_sum_simd(src, src_stride, pred, pred_stride, dims, true)
}

/// The sum of `|d|` or of `d * d` over the block, in `min(width, 16)` lanes.
fn diff_sum_simd(
    src: &[u16],
    src_stride: usize,
    pred: &[u16],
    pred_stride: usize,
    dims: BlockDims,
    square: bool,
) -> u64 {
    match dims.width() {
        4 => diff_sum_lanes::<4>(src, src_stride, pred, pred_stride, dims, square),
        8 => diff_sum_lanes::<8>(src, src_stride, pred, pred_stride, dims, square),
        _ => diff_sum_lanes::<16>(src, src_stride, pred, pred_stride, dims, square),
    }
}

fn diff_sum_lanes<const L: usize>(
    src: &[u16],
    src_stride: usize,
    pred: &[u16],
    pred_stride: usize,
    dims: BlockDims,
    square: bool,
) -> u64 {
    let mut acc = Simd::<u64, L>::splat(0);
    for (a, b) in row_pairs(src, src_stride, pred, pred_stride, dims) {
        for (a, b) in a.as_chunks::<L>().0.iter().zip(b.as_chunks::<L>().0) {
            let (a, b) = (Simd::from_array(*a), Simd::from_array(*b));
            let d = (a.simd_max(b) - a.simd_min(b)).cast::<u32>();
            acc += if square { d * d } else { d }.cast::<u64>();
        }
    }
    acc.reduce_sum()
}

/// The SATD tile side for `dims`: 8 when both sides allow it, else 4.
fn satd_tile(dims: BlockDims) -> usize {
    if dims.width() >= 8 && dims.height() >= 8 {
        8
    } else {
        4
    }
}

/// The top-left sample offset of every SATD tile in a strided block.
fn tile_origins(dims: BlockDims, tile: usize) -> impl Iterator<Item = (usize, usize)> {
    (0..dims.height())
        .step_by(tile)
        .flat_map(move |y| (0..dims.width()).step_by(tile).map(move |x| (y, x)))
}

pub(super) fn satd_scalar(
    src: &[u16],
    src_stride: usize,
    pred: &[u16],
    pred_stride: usize,
    dims: BlockDims,
) -> u64 {
    let tile = satd_tile(dims);
    let mut total = 0u64;
    for (y, x) in tile_origins(dims, tile) {
        let mut d = [0i32; 64];
        for (i, row) in d.chunks_exact_mut(tile).take(tile).enumerate() {
            let a = src.get((y + i) * src_stride + x..).unwrap_or_default();
            let b = pred.get((y + i) * pred_stride + x..).unwrap_or_default();
            for ((slot, &a), &b) in row.iter_mut().zip(a).zip(b) {
                *slot = i32::from(a) - i32::from(b);
            }
        }
        let d = &mut d[..tile * tile];
        for row in d.chunks_exact_mut(tile) {
            hadamard_scalar(row, 1);
        }
        for column in 0..tile {
            hadamard_scalar(&mut d[column..], tile);
        }
        total += d.iter().map(|c| u64::from(c.unsigned_abs())).sum::<u64>();
    }
    total
}

/// The in-place unnormalized Walsh-Hadamard butterfly over the samples of `v`
/// at `0, step, 2 * step, ...` (`v.len() / step` of them, a power of two).
fn hadamard_scalar(v: &mut [i32], step: usize) {
    let n = v.len().div_ceil(step);
    let mut half = 1;
    while half < n {
        for base in (0..n).step_by(2 * half) {
            for k in base..base + half {
                let (a, b) = (v[k * step], v[(k + half) * step]);
                v[k * step] = a + b;
                v[(k + half) * step] = a - b;
            }
        }
        half *= 2;
    }
}

pub(super) fn satd_simd(
    src: &[u16],
    src_stride: usize,
    pred: &[u16],
    pred_stride: usize,
    dims: BlockDims,
) -> u64 {
    let tile = satd_tile(dims);
    let mut total = 0u64;
    for (y, x) in tile_origins(dims, tile) {
        let a = src.get(y * src_stride + x..).unwrap_or_default();
        let b = pred.get(y * pred_stride + x..).unwrap_or_default();
        total += if tile == 8 {
            satd_tile_lanes::<8>(a, src_stride, b, pred_stride, hadamard_rows8)
        } else {
            satd_tile_lanes::<4>(a, src_stride, b, pred_stride, hadamard_rows4)
        };
    }
    total
}

/// One `N x N` SATD tile: a column Hadamard across the row vectors, then a
/// row Hadamard inside each vector. The row transform permutes its outputs,
/// which leaves the sum of magnitudes unchanged.
fn satd_tile_lanes<const N: usize>(
    src: &[u16],
    src_stride: usize,
    pred: &[u16],
    pred_stride: usize,
    row_hadamard: fn(Simd<i32, N>) -> Simd<i32, N>,
) -> u64 {
    let zero = [0u16; N];
    let mut v = [Simd::<i32, N>::splat(0); N];
    for (i, slot) in v.iter_mut().enumerate() {
        let load = |s: &[u16], stride: usize| {
            let row = s
                .get(i * stride..)
                .and_then(|r| r.first_chunk::<N>())
                .unwrap_or(&zero);
            Simd::from_array(*row).cast::<i32>()
        };
        *slot = load(src, src_stride) - load(pred, pred_stride);
    }
    let mut half = 1;
    while half < N {
        for base in (0..N).step_by(2 * half) {
            for k in base..base + half {
                let (x, y) = (v[k], v[k + half]);
                v[k] = x + y;
                v[k + half] = x - y;
            }
        }
        half *= 2;
    }
    v.iter()
        .map(|&row| u64::from(row_hadamard(row).abs().cast::<u32>().reduce_sum()))
        .sum()
}

fn hadamard_rows4(v: Simd<i32, 4>) -> Simd<i32, 4> {
    let v = simd_swizzle!(v, [0, 0, 2, 2])
        + simd_swizzle!(v, [1, 1, 3, 3]) * Simd::from_array([1, -1, 1, -1]);
    simd_swizzle!(v, [0, 1, 0, 1])
        + simd_swizzle!(v, [2, 3, 2, 3]) * Simd::from_array([1, 1, -1, -1])
}

fn hadamard_rows8(v: Simd<i32, 8>) -> Simd<i32, 8> {
    let v = simd_swizzle!(v, [0, 0, 2, 2, 4, 4, 6, 6])
        + simd_swizzle!(v, [1, 1, 3, 3, 5, 5, 7, 7])
            * Simd::from_array([1, -1, 1, -1, 1, -1, 1, -1]);
    let v = simd_swizzle!(v, [0, 1, 0, 1, 4, 5, 4, 5])
        + simd_swizzle!(v, [2, 3, 2, 3, 6, 7, 6, 7])
            * Simd::from_array([1, 1, -1, -1, 1, 1, -1, -1]);
    simd_swizzle!(v, [0, 1, 2, 3, 0, 1, 2, 3])
        + simd_swizzle!(v, [4, 5, 6, 7, 4, 5, 6, 7])
            * Simd::from_array([1, 1, 1, 1, -1, -1, -1, -1])
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use proptest::prelude::*;

    use super::super::{BlockDims, EncoderKernels, ScalarKernels, SimdKernels, kernels};

    fn side() -> impl Strategy<Value = usize> {
        (2u32..=7).prop_map(|log2| 1usize << log2)
    }

    /// Two strided blocks of `w x h` samples below `2^bits`.
    fn blocks() -> impl Strategy<Value = (BlockDims, usize, Vec<u16>, usize, Vec<u16>)> {
        (
            side(),
            side(),
            0usize..9,
            0usize..9,
            prop_oneof![Just(8u32), Just(10), Just(16)],
        )
            .prop_flat_map(|(w, h, pad_a, pad_b, bits)| {
                let max = ((1u32 << bits) - 1) as u16;
                let (sa, sb) = (w + pad_a, w + pad_b);
                (
                    Just(BlockDims::new(w, h).unwrap()),
                    Just(sa),
                    proptest::collection::vec(0..=max, (h - 1) * sa + w),
                    Just(sb),
                    proptest::collection::vec(0..=max, (h - 1) * sb + w),
                )
            })
    }

    proptest! {
        #[test]
        fn simd_matches_scalar((dims, sa, a, sb, b) in blocks()) {
            prop_assert_eq!(SimdKernels.sad(&a, sa, &b, sb, dims), ScalarKernels.sad(&a, sa, &b, sb, dims));
            prop_assert_eq!(SimdKernels.sse(&a, sa, &b, sb, dims), ScalarKernels.sse(&a, sa, &b, sb, dims));
            prop_assert_eq!(SimdKernels.satd(&a, sa, &b, sb, dims), ScalarKernels.satd(&a, sa, &b, sb, dims));
        }
    }

    #[test]
    fn constant_difference_values() {
        for (w, h) in [(4, 4), (8, 8), (16, 4), (128, 128)] {
            let dims = BlockDims::new(w, h).unwrap();
            let a = vec![300u16; w * h];
            let b = vec![297u16; w * h];
            let n = (w * h) as u64;
            for k in [
                &ScalarKernels as &dyn EncoderKernels,
                &SimdKernels,
                kernels(),
            ] {
                assert_eq!(u64::from(k.sad(&a, w, &b, w, dims)), 3 * n);
                assert_eq!(k.sse(&a, w, &b, w, dims), 9 * n);
                assert_eq!(
                    k.satd(&a, w, &b, w, dims),
                    3 * n,
                    "flat SATD equals SAD at {w}x{h}"
                );
            }
        }
    }

    #[test]
    fn satd_of_one_hadamard_basis_is_one_coefficient() {
        let dims = BlockDims::new(4, 4).unwrap();
        let a: Vec<u16> = (0..16)
            .map(|i| if (i % 4) % 2 == 0 { 110 } else { 90 })
            .collect();
        let b = vec![100u16; 16];
        assert_eq!(ScalarKernels.satd(&a, 4, &b, 4, dims), 160);
        assert_eq!(SimdKernels.satd(&a, 4, &b, 4, dims), 160);
    }

    #[test]
    fn rejects_unsupported_sizes() {
        assert!(BlockDims::new(2, 4).is_err());
        assert!(BlockDims::new(12, 4).is_err());
        assert!(BlockDims::new(4, 256).is_err());
    }
}
