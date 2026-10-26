// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! SAD, SSE, and Hadamard SATD over strided `u16` blocks.
//!
//! The SIMD forms run one row at a time in `min(width, 16)` lanes (rows are 4,
//! 8, or a multiple of 16 samples wide). Every sum is exact integer arithmetic
//! that cannot overflow for `u16` inputs, so lane order does not change a
//! result: `|d| <= 65535` keeps a SAD lane of a 256x256 block below `2^28`
//! and its whole `u32` sum below `2^32` (`65536 * 65535`), `d * d` fits `u32`
//! before it widens into `u64` lanes, and an 8x8 Hadamard coefficient stays
//! below `64 * 65535`.

use std::simd::num::{SimdInt as _, SimdUint as _};
use std::simd::{Simd, cmp::SimdOrd as _, simd_swizzle};

use super::BlockDims;

/// The first `dims.height()` rows of a strided block, each `dims.width()` wide.
#[cfg(test)]
fn rows(samples: &[u16], stride: usize, dims: BlockDims) -> impl Iterator<Item = &[u16]> {
    debug_assert!(stride >= dims.width());
    samples
        .chunks(stride.max(1))
        .take(dims.height())
        .filter_map(move |row| row.get(..dims.width()))
}

#[cfg(test)]
fn row_pairs<'a>(
    src: &'a [u16],
    src_stride: usize,
    pred: &'a [u16],
    pred_stride: usize,
    dims: BlockDims,
) -> impl Iterator<Item = (&'a [u16], &'a [u16])> {
    rows(src, src_stride, dims).zip(rows(pred, pred_stride, dims))
}

/// Calls `f` on the same row pairs as the scalar row walk, with one bounds check up
/// front: every row after the first is the tail of a `stride`-long exact chunk.
#[inline]
fn for_each_row_pair<'a>(
    src: &'a [u16],
    src_stride: usize,
    pred: &'a [u16],
    pred_stride: usize,
    dims: BlockDims,
    mut f: impl FnMut(&[u16], &[u16]),
) {
    let width = dims.width();
    let held = |len: usize, stride: usize| match len.checked_sub(width) {
        Some(spare) if stride >= width => spare / stride + 1,
        _ => 0,
    };
    let rows = held(src.len(), src_stride)
        .min(held(pred.len(), pred_stride))
        .min(dims.height());
    let Some(last) = rows.checked_sub(1) else {
        return;
    };
    let split = |samples: &'a [u16], stride: usize| {
        samples
            .get(..last * stride + width)
            .and_then(|block| block.split_at_checked(width))
    };
    let (Some((src_first, src_rest)), Some((pred_first, pred_rest))) =
        (split(src, src_stride), split(pred, pred_stride))
    else {
        return;
    };
    f(src_first, pred_first);
    let tail = |row: &[u16]| row.len().saturating_sub(width);
    let src_rest = src_rest.chunks_exact(src_stride.max(1));
    for (a, b) in src_rest.zip(pred_rest.chunks_exact(pred_stride.max(1))) {
        f(&a[tail(a)..], &b[tail(b)..]);
    }
}

#[cfg(test)]
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

#[cfg(test)]
pub(super) fn sse_row_scalar(a: &[u16], b: &[u16]) -> u64 {
    a.iter()
        .zip(b)
        .map(|(&a, &b)| u64::from(a.abs_diff(b)).pow(2))
        .sum()
}

/// [`sse_row_scalar`] in 16 lanes plus a scalar tail.
pub(super) fn sse_row_simd(a: &[u16], b: &[u16]) -> u64 {
    let len = a.len().min(b.len());
    let (a_chunks, a_tail) = a[..len].as_chunks::<16>();
    let (b_chunks, b_tail) = b[..len].as_chunks::<16>();
    let mut acc = Simd::<u64, 16>::splat(0);
    for (a, b) in a_chunks.iter().zip(b_chunks) {
        let (a, b) = (Simd::from_array(*a), Simd::from_array(*b));
        let d = (a.simd_max(b) - a.simd_min(b)).cast::<u32>();
        acc += (d * d).cast::<u64>();
    }
    acc.reduce_sum()
        + a_tail
            .iter()
            .zip(b_tail)
            .map(|(&a, &b)| u64::from(a.abs_diff(b)).pow(2))
            .sum::<u64>()
}

#[cfg(test)]
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
    match dims.width() {
        4 => sad_lanes::<4>(src, src_stride, pred, pred_stride, dims),
        8 => sad_lanes::<8>(src, src_stride, pred, pred_stride, dims),
        _ => sad_lanes::<16>(src, src_stride, pred, pred_stride, dims),
    }
}

/// The sum of `|d|` in `L` `u32` lanes.
fn sad_lanes<const L: usize>(
    src: &[u16],
    src_stride: usize,
    pred: &[u16],
    pred_stride: usize,
    dims: BlockDims,
) -> u32 {
    let mut acc = Simd::<u32, L>::splat(0);
    for_each_row_pair(src, src_stride, pred, pred_stride, dims, |a, b| {
        for (a, b) in a.as_chunks::<L>().0.iter().zip(b.as_chunks::<L>().0) {
            let (a, b) = (Simd::from_array(*a), Simd::from_array(*b));
            acc += (a.simd_max(b) - a.simd_min(b)).cast::<u32>();
        }
    });
    acc.reduce_sum()
}

pub(super) fn sse_simd(
    src: &[u16],
    src_stride: usize,
    pred: &[u16],
    pred_stride: usize,
    dims: BlockDims,
) -> u64 {
    match dims.width() {
        4 => sse_lanes::<4>(src, src_stride, pred, pred_stride, dims),
        8 => sse_lanes::<8>(src, src_stride, pred, pred_stride, dims),
        _ => sse_lanes::<16>(src, src_stride, pred, pred_stride, dims),
    }
}

/// The sum of `d * d` in `L` `u64` lanes.
fn sse_lanes<const L: usize>(
    src: &[u16],
    src_stride: usize,
    pred: &[u16],
    pred_stride: usize,
    dims: BlockDims,
) -> u64 {
    let mut acc = Simd::<u64, L>::splat(0);
    for_each_row_pair(src, src_stride, pred, pred_stride, dims, |a, b| {
        for (a, b) in a.as_chunks::<L>().0.iter().zip(b.as_chunks::<L>().0) {
            let (a, b) = (Simd::from_array(*a), Simd::from_array(*b));
            let d = (a.simd_max(b) - a.simd_min(b)).cast::<u32>();
            acc += (d * d).cast::<u64>();
        }
    });
    acc.reduce_sum()
}

#[cfg(test)]
pub(super) fn residual_scalar(
    src: &[u16],
    src_stride: usize,
    pred: &[u16],
    pred_stride: usize,
    dims: BlockDims,
    out: &mut [i32],
) {
    let rows = row_pairs(src, src_stride, pred, pred_stride, dims);
    for (out, (a, b)) in out.chunks_exact_mut(dims.width()).zip(rows) {
        for (slot, (&a, &b)) in out.iter_mut().zip(a.iter().zip(b)) {
            *slot = i32::from(a) - i32::from(b);
        }
    }
}

pub(super) fn residual_simd(
    src: &[u16],
    src_stride: usize,
    pred: &[u16],
    pred_stride: usize,
    dims: BlockDims,
    out: &mut [i32],
) {
    match dims.width() {
        4 => residual_lanes::<4>(src, src_stride, pred, pred_stride, dims, out),
        8 => residual_lanes::<8>(src, src_stride, pred, pred_stride, dims, out),
        _ => residual_lanes::<16>(src, src_stride, pred, pred_stride, dims, out),
    }
}

fn residual_lanes<const L: usize>(
    src: &[u16],
    src_stride: usize,
    pred: &[u16],
    pred_stride: usize,
    dims: BlockDims,
    out: &mut [i32],
) {
    let mut out = out.chunks_exact_mut(dims.width());
    for_each_row_pair(src, src_stride, pred, pred_stride, dims, |a, b| {
        let Some(out) = out.next() else { return };
        let lanes = out.as_chunks_mut::<L>().0.iter_mut();
        for (slot, (a, b)) in lanes.zip(a.as_chunks::<L>().0.iter().zip(b.as_chunks::<L>().0)) {
            let (a, b) = (Simd::from_array(*a), Simd::from_array(*b));
            *slot = (a.cast::<i32>() - b.cast::<i32>()).to_array();
        }
    });
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

#[cfg(test)]
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
            for (j, slot) in row.iter_mut().enumerate() {
                let sample = |s: &[u16]| i32::from(s.get(j).copied().unwrap_or(0));
                *slot = sample(a) - sample(b);
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
#[cfg(test)]
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
    let mut v = [Simd::<i32, N>::splat(0); N];
    for (i, slot) in v.iter_mut().enumerate() {
        let load = |s: &[u16], stride: usize| {
            let row = s.get(i * stride..).unwrap_or_default();
            let row = row.first_chunk::<N>().copied().unwrap_or_else(|| {
                let mut tail = [0u16; N];
                tail[..row.len()].copy_from_slice(row);
                tail
            });
            Simd::from_array(row).cast::<i32>()
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
        (2u32..=8).prop_map(|log2| 1usize << log2)
    }

    /// Two strided blocks of `w x h` samples below `2^bits`.
    fn blocks() -> impl Strategy<Value = (BlockDims, usize, Vec<u16>, usize, Vec<u16>)> {
        (
            side(),
            side(),
            prop_oneof![0usize..9, 9usize..300],
            prop_oneof![0usize..9, 9usize..300],
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
            let len = dims.width() * dims.height();
            let (mut simd, mut scalar) = (vec![0; len], vec![1; len]);
            SimdKernels.residual(&a, sa, &b, sb, dims, &mut simd);
            ScalarKernels.residual(&a, sa, &b, sb, dims, &mut scalar);
            prop_assert_eq!(simd, scalar);
        }

        /// Rows of any length, both slices of different lengths.
        #[test]
        fn row_sse_simd_matches_scalar(
            a in proptest::collection::vec(any::<u16>(), 0..100),
            b in proptest::collection::vec(any::<u16>(), 0..100),
        ) {
            prop_assert_eq!(SimdKernels.sse_row(&a, &b), ScalarKernels.sse_row(&a, &b));
        }

        /// Short sample slices and a short `out` drop the same rows on both paths.
        #[test]
        fn simd_matches_scalar_on_short_slices(
            (dims, sa, a, sb, b) in blocks(),
            (keep_a, keep_b, keep_out) in any::<(prop::sample::Index, prop::sample::Index, prop::sample::Index)>(),
        ) {
            let a = &a[..keep_a.index(a.len() + 1)];
            let b = &b[..keep_b.index(b.len() + 1)];
            prop_assert_eq!(SimdKernels.sad(a, sa, b, sb, dims), ScalarKernels.sad(a, sa, b, sb, dims));
            prop_assert_eq!(SimdKernels.sse(a, sa, b, sb, dims), ScalarKernels.sse(a, sa, b, sb, dims));
            prop_assert_eq!(SimdKernels.satd(a, sa, b, sb, dims), ScalarKernels.satd(a, sa, b, sb, dims));
            let len = keep_out.index(dims.width() * dims.height() + 1);
            let (mut simd, mut scalar) = (vec![7; len], vec![7; len]);
            SimdKernels.residual(a, sa, b, sb, dims, &mut simd);
            ScalarKernels.residual(a, sa, b, sb, dims, &mut scalar);
            prop_assert_eq!(simd, scalar);
        }
    }

    #[test]
    fn constant_difference_values() {
        for (w, h) in [(4, 4), (8, 8), (16, 4), (128, 128), (256, 256)] {
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
        assert!(BlockDims::new(4, 512).is_err());
    }
}
