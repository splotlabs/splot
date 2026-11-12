// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The AVX2 4x4 and 8x8 register cells of [`super`]: both passes of a
//! cell in registers, the vertical pass reading its rows from the
//! horizontal groups.

use super::{Horizontal, average, start32, store_bytes, store_rows, tap_bytes};
use crate::subpel_mc::block::Block;
use core::arch::x86_64::*;
use fearless_simd::{Avx2, SimdBase, u8x32};

/// The 16 predictors of each group of a cell; a 4x4 cell has one.
type Groups = [__m256i; 4];

/// `pshufb` pattern widening columns `0..4` of the two 8-byte rows of
/// each lane to words.
const NARROW_ROWS: [u8; 32] = {
    let mut out = [0x80; 32];
    let mut lane = 0;
    while lane < 32 {
        if lane % 2 == 0 {
            let word = (lane % 16) / 2;
            out[lane] = ((word / 4) * 8 + word % 4) as u8;
        }
        lane += 1;
    }
    out
};

/// `Some` of `$f::<$w, H, V>` for the kernel key `5 * H + V` of a
/// reference of a `$w`-square cell, and `None` for any other key.
macro_rules! by_key {
    ($key:expr, 8, $f:ident $args:tt) => {
        by_key!(@ $key, 8, $f $args, [
            0 0 0, 1 0 1, 2 0 2, 3 0 3, 4 0 4, 5 1 0, 6 1 1, 7 1 2, 8 1 3, 9 1 4,
            10 2 0, 11 2 1, 12 2 2, 13 2 3, 14 2 4, 15 3 0, 16 3 1, 17 3 2, 18 3 3,
            19 3 4, 20 4 0, 21 4 1, 22 4 2, 23 4 3, 24 4 4
        ])
    };
    ($key:expr, 4, $f:ident $args:tt) => {
        by_key!(@ $key, 4, $f $args, [
            0 0 0, 1 0 1, 2 0 2, 3 0 3, 4 0 4, 5 1 0, 6 1 1, 7 1 2, 8 1 3, 9 1 4,
            10 2 0, 11 2 1, 12 2 2, 13 2 3, 14 2 4
        ])
    };
    (@ $key:expr, $w:literal, $f:ident $args:tt, [$($c:literal $h:literal $v:literal),*]) => {
        match $key {
            $($c => Some($f::<$w, $h, $v> $args),)*
            _ => None,
        }
    };
}

/// An output stride whose eight rows stay below `2^32`.
#[inline]
fn output_stride(stride: usize) -> Option<u32> {
    u32::try_from(stride)
        .ok()
        .filter(|&stride| stride <= u32::MAX / 8)
}

/// Whether the `u32` window starts of the kernels address `block`.
pub(super) fn addressable<T>(block: &Block<'_, T>) -> bool {
    u32::try_from(block.source.len()).is_ok()
}

/// The kernel key `5 * H + V` of a block's tap-pair counts, zero for a
/// skipped pass.
fn key(block: &Block<'_, u8>) -> i32 {
    (5 * block.horizontal.map_or(0, |taps| taps.count)
        + block.vertical.map_or(0, |taps| taps.count)) as i32
}

fearless_simd::kernel!(
    /// Stores the COMPOUND_AVERAGE of the blocks of a `size`-square
    /// cell, or returns `false` without writing.
    #[inline]
    pub(super) fn compound(
        t: Avx2,
        blocks: [&Block<'_, u8>; 2],
        size: usize,
        forward: i32,
        output: &mut [u8],
        stride: usize,
    ) -> bool {
        let Some(stride) =
            output_stride(stride).filter(|_| blocks.iter().all(|block| addressable(block)))
        else {
            return false;
        };
        let groups = |block: &Block<'_, u8>| {
            if size == 4 {
                by_key!(key(block), 4, groups(t, block))
            } else {
                by_key!(key(block), 8, groups(t, block))
            }
        };
        let (Some(first), Some(second)) = (groups(blocks[0]), groups(blocks[1])) else {
            return false;
        };
        if size == 4 {
            store_rows::<4, u32>(t, output, 0, stride, average(first[0], second[0], forward));
            return true;
        }
        for (group, (first, second)) in (0..).zip(first.into_iter().zip(second)) {
            let blended = average(first, second, forward);
            store_rows::<8, u32>(t, output, 2 * group * stride, stride, blended);
        }
        true
    }
);

/// The compound [`predict`] of one block.
#[inline(never)]
#[target_feature(enable = "avx2")]
fn groups<const W: usize, const H: usize, const V: usize>(
    t: Avx2,
    block: &Block<'_, u8>,
) -> Groups {
    predict::<W, H, V, false>(t, block)
}

fearless_simd::kernel!(
    #[inline]
    pub(super) fn put(
        t: Avx2,
        block: &Block<'_, u8>,
        size: usize,
        output: &mut [u8],
        stride: usize,
    ) -> bool {
        if size == 4 {
            by_key!(key(block), 4, put_cell(t, block, output, stride)).is_some()
        } else {
            by_key!(key(block), 8, put_cell(t, block, output, stride)).is_some()
        }
    }
);

/// Stores the [`predict`] put of one cell.
#[inline(never)]
#[target_feature(enable = "avx2")]
fn put_cell<const W: usize, const H: usize, const V: usize>(
    t: Avx2,
    block: &Block<'_, u8>,
    output: &mut [u8],
    stride: usize,
) {
    let groups = predict::<W, H, V, true>(t, block);
    if W == 4 {
        store_bytes::<4>(t, output, 0, stride, groups[0]);
        return;
    }
    for (group, &values) in groups.iter().enumerate() {
        store_bytes::<8>(t, output, 2 * group * stride, stride, values);
    }
}

/// The compound predictors of one `W`-square cell whose passes have `H`
/// and `V` tap pairs, none for a skipped pass.
#[inline]
#[target_feature(enable = "avx2")]
fn predict<const W: usize, const H: usize, const V: usize, const PUT: bool>(
    t: Avx2,
    block: &Block<'_, u8>,
) -> Groups {
    let per = 16 / W;
    let rows = if V > 0 { W + 2 * V - 1 } else { W };
    let zero = _mm256_setzero_si256();
    let start = |row: usize| start32(block, row.min(rows - 1));
    let filter = match block.horizontal {
        Some(taps) if H > 0 => Some(Horizontal::new::<W, H>(t, taps)),
        _ => None,
    };
    let narrow: __m256i = u8x32::load_array(t, NARROW_ROWS).into();
    let mut lines = [zero; 8];
    for (group, line) in lines.iter_mut().enumerate().take(rows.div_ceil(per)) {
        let row = group * per;
        let starts = [start(row), start(row + 1), start(row + 2), start(row + 3)];
        let Some(bytes) = tap_bytes::<W>(t, block.source, starts) else {
            return [zero; 4];
        };
        *line = match &filter {
            Some(filter) if PUT && V == 0 => _mm256_srai_epi16::<6>(_mm256_add_epi16(
                filter.sums::<H>(bytes),
                _mm256_set1_epi16(34),
            )),
            Some(filter) => {
                _mm256_mulhrs_epi16(filter.sums::<H>(bytes), _mm256_set1_epi16(1 << 13))
            }
            None if W == 4 => _mm256_shuffle_epi8(bytes, narrow),
            None => _mm256_unpacklo_epi8(bytes, zero),
        };
    }
    let mut out = [zero; 4];
    let Some(taps) = block.vertical.filter(|_| V > 0) else {
        for (out, &line) in out.iter_mut().zip(&lines) {
            *out = if H > 0 || PUT {
                line
            } else {
                _mm256_slli_epi16::<4>(line)
            };
        }
        return out;
    };
    let line = |row: usize| -> __m256i {
        let (index, offset) = (row / per, row % per);
        let first = lines[index.min(7)];
        if offset == 0 {
            return first;
        }
        let second = lines[(index + 1).min(7)];
        let middle = _mm256_permute2x128_si256::<0x21>(first, second);
        match (W, offset) {
            (8, _) | (_, 2) => middle,
            (_, 1) => _mm256_alignr_epi8::<8>(middle, first),
            _ => _mm256_alignr_epi8::<8>(second, middle),
        }
    };
    let weights: [__m256i; 4] =
        core::array::from_fn(|pair| _mm256_set1_epi32(taps.words[pair.min(V.max(1) - 1)]));
    let bias = _mm256_set1_epi32(match (H > 0, PUT) {
        (true, true) => 1 << 9,
        (true, false) | (false, true) => 1 << 5,
        (false, false) => 1 << 1,
    });
    for (group, out) in out.iter_mut().enumerate().take(W * W / 16) {
        let (mut low, mut high) = (bias, bias);
        for (pair, &weights) in weights.iter().enumerate().take(V) {
            let (a, b) = (
                line(group * per + 2 * pair),
                line(group * per + 2 * pair + 1),
            );
            low = _mm256_add_epi32(low, _mm256_madd_epi16(_mm256_unpacklo_epi16(a, b), weights));
            high = _mm256_add_epi32(
                high,
                _mm256_madd_epi16(_mm256_unpackhi_epi16(a, b), weights),
            );
        }
        *out = match (H > 0, PUT) {
            (true, true) => {
                _mm256_packs_epi32(_mm256_srai_epi32::<10>(low), _mm256_srai_epi32::<10>(high))
            }
            (true, false) | (false, true) => {
                _mm256_packs_epi32(_mm256_srai_epi32::<6>(low), _mm256_srai_epi32::<6>(high))
            }
            (false, false) => {
                _mm256_packs_epi32(_mm256_srai_epi32::<2>(low), _mm256_srai_epi32::<2>(high))
            }
        };
    }
    out
}
