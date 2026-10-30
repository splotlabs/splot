// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The open-loop motion search kernels over 8-bit planes: the 8x8 SADs of a
//! block grid at four horizontal positions, the 2x2 sums that build larger
//! blocks from them, the per-block best of four positions, and the 2x2
//! mean that halves a plane.
//!
//! The SIMD forms keep one SAD lane per position. An 8x8 SAD of `u8` samples
//! is at most `64 * 255`, so a `u16` row accumulator cannot overflow; sums
//! of SADs wrap in `u32` on both paths.

use std::simd::Simd;
use std::simd::cmp::{SimdOrd as _, SimdPartialEq as _};
use std::simd::num::SimdUint as _;

/// The positions one SAD call scores, one sample apart horizontally.
pub(crate) const WAYS: usize = 4;

/// A block's best vector and its SAD.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Motion {
    /// SAD of the block at `mv`.
    pub(crate) sad: u32,
    /// Full-sample vector `[x, y]` at the plane's resolution.
    pub(crate) mv: [i16; 2],
}

impl Motion {
    /// No vector yet: every SAD beats it.
    pub(crate) const NONE: Self = Self {
        sad: u32::MAX,
        mv: [0, 0],
    };
}

/// Whether the 8x8 blocks of a `cols` x `rows` grid fit `src` and, with
/// [`WAYS`] - 1 more columns, `refs`.
fn grid_fits(
    (src, src_stride): (&[u8], usize),
    (refs, ref_stride): (&[u8], usize),
    cols: usize,
    rows: usize,
) -> bool {
    let width = cols * 8;
    let needed = |stride: usize, extra: usize| (rows * 8 - 1) * stride + width + extra;
    cols > 0
        && rows > 0
        && src_stride >= width
        && ref_stride >= width + WAYS - 1
        && src.len() >= needed(src_stride, 0)
        && refs.len() >= needed(ref_stride, WAYS - 1)
}

/// `out[b][lane]`: the SAD of 8x8 block `b` of a `cols`-wide grid of `src`
/// against `refs` moved right by `lane` samples. A grid that does not fit
/// its planes scores `u32::MAX` everywhere.
#[cfg(test)]
pub(super) fn sad8x8_x4_scalar(
    (src, src_stride): (&[u8], usize),
    (refs, ref_stride): (&[u8], usize),
    cols: usize,
    out: &mut [[u32; WAYS]],
) {
    let rows = out.len() / cols.max(1);
    if !grid_fits((src, src_stride), (refs, ref_stride), cols, rows) {
        out.fill([u32::MAX; WAYS]);
        return;
    }
    for (index, sads) in out.iter_mut().enumerate().take(cols * rows) {
        let (x, y) = (index % cols * 8, index / cols * 8);
        *sads = core::array::from_fn(|lane| {
            (0..64)
                .map(|i| {
                    let (dy, dx) = (y + i / 8, x + i % 8);
                    u32::from(src[dy * src_stride + dx].abs_diff(refs[dy * ref_stride + dx + lane]))
                })
                .sum()
        });
    }
}

/// [`sad8x8_x4_scalar`] with each source row loaded once for the four
/// positions, in eight `u16` lanes per position.
pub(super) fn sad8x8_x4_simd(
    (src, src_stride): (&[u8], usize),
    (refs, ref_stride): (&[u8], usize),
    cols: usize,
    out: &mut [[u32; WAYS]],
) {
    let rows = out.len() / cols.max(1);
    if !grid_fits((src, src_stride), (refs, ref_stride), cols, rows) {
        out.fill([u32::MAX; WAYS]);
        return;
    }
    for (index, sads) in out.iter_mut().enumerate().take(cols * rows) {
        let (x, y) = (index % cols * 8, index / cols * 8);
        let mut acc = [Simd::<u16, 8>::splat(0); WAYS];
        for row in y..y + 8 {
            let source = Simd::<u8, 8>::from_slice(&src[row * src_stride + x..]);
            let candidates = &refs[row * ref_stride + x..];
            for (lane, acc) in acc.iter_mut().enumerate() {
                let candidate = Simd::<u8, 8>::from_slice(&candidates[lane..]);
                *acc += (source.simd_max(candidate) - source.simd_min(candidate)).cast::<u16>();
            }
        }
        *sads = acc.map(|acc| u32::from(acc.reduce_sum()));
    }
}

/// `out[i]`: the wrapping sum of each 2x2 group of a `cols`-wide grid of
/// `input`, for the `(cols / 2) x (rows / 2)` groups that `out` holds.
#[cfg(test)]
pub(super) fn sad_quads_scalar(input: &[[u32; WAYS]], cols: usize, out: &mut [[u32; WAYS]]) {
    let (half_cols, half_rows) = (cols / 2, input.len() / cols.max(1) / 2);
    for (index, sum) in out.iter_mut().enumerate().take(half_cols * half_rows) {
        let at = index / half_cols * 2 * cols + index % half_cols * 2;
        *sum = core::array::from_fn(|lane| {
            [at, at + 1, at + cols, at + cols + 1]
                .iter()
                .fold(0u32, |sum, &i| sum.wrapping_add(input[i][lane]))
        });
    }
}

/// [`sad_quads_scalar`] in [`WAYS`] `u32` lanes.
pub(super) fn sad_quads_simd(input: &[[u32; WAYS]], cols: usize, out: &mut [[u32; WAYS]]) {
    let (half_cols, half_rows) = (cols / 2, input.len() / cols.max(1) / 2);
    let load = |i: usize| Simd::<u32, WAYS>::from_array(input[i]);
    for (index, sum) in out.iter_mut().enumerate().take(half_cols * half_rows) {
        let at = index / half_cols * 2 * cols + index % half_cols * 2;
        *sum = (load(at) + load(at + 1) + load(at + cols) + load(at + cols + 1)).to_array();
    }
}

/// Moves each `best` to the first lane of its `sads` that scores below it;
/// lane `l` is the vector `mv + [l, 0]`.
#[cfg(test)]
pub(super) fn keep_best_scalar(sads: &[[u32; WAYS]], mv: [i16; 2], best: &mut [Motion]) {
    for (sads, best) in sads.iter().zip(best) {
        for (lane, &sad) in sads.iter().enumerate() {
            if sad < best.sad {
                *best = Motion {
                    sad,
                    mv: [mv[0].wrapping_add(lane as i16), mv[1]],
                };
            }
        }
    }
}

/// [`keep_best_scalar`] with a lane minimum per block.
pub(super) fn keep_best_simd(sads: &[[u32; WAYS]], mv: [i16; 2], best: &mut [Motion]) {
    for (sads, best) in sads.iter().zip(best) {
        let sads = Simd::<u32, WAYS>::from_array(*sads);
        let min = sads.reduce_min();
        if min < best.sad
            && let Some(lane) = sads.simd_eq(Simd::splat(min)).first_set()
        {
            *best = Motion {
                sad: min,
                mv: [mv[0].wrapping_add(lane as i16), mv[1]],
            };
        }
    }
}

/// `out[i]`: the mean, rounded down, of samples `2i` and `2i + 1` of `top` and of
/// `bottom`, for the `i` all three slices hold.
#[cfg(test)]
pub(super) fn downscale_row_scalar(top: &[u8], bottom: &[u8], out: &mut [u8]) {
    let len = out.len().min(top.len() / 2).min(bottom.len() / 2);
    for (i, out) in out.iter_mut().enumerate().take(len) {
        let sum: u16 = [top[2 * i], top[2 * i + 1], bottom[2 * i], bottom[2 * i + 1]]
            .iter()
            .map(|&s| u16::from(s))
            .sum();
        *out = (sum >> 2) as u8;
    }
}

/// [`downscale_row_scalar`] in 16 lanes plus a scalar tail.
pub(super) fn downscale_row_simd(top: &[u8], bottom: &[u8], out: &mut [u8]) {
    let len = out.len().min(top.len() / 2).min(bottom.len() / 2);
    let pairs = |row: &[u8], i: usize| {
        let (even, odd) = Simd::<u8, 16>::from_slice(&row[2 * i..])
            .deinterleave(Simd::from_slice(&row[2 * i + 16..]));
        even.cast::<u16>() + odd.cast::<u16>()
    };
    let mut i = 0;
    while i + 16 <= len {
        let sum = pairs(top, i) + pairs(bottom, i);
        (sum >> Simd::splat(2))
            .cast::<u8>()
            .copy_to_slice(&mut out[i..i + 16]);
        i += 16;
    }
    for i in i..len {
        let sum = u16::from(top[2 * i]) + u16::from(top[2 * i + 1]);
        let sum = sum + u16::from(bottom[2 * i]) + u16::from(bottom[2 * i + 1]);
        out[i] = (sum >> 2) as u8;
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    fn samples(len: usize) -> impl Strategy<Value = Vec<u8>> {
        prop::collection::vec(any::<u8>(), len)
    }

    proptest! {
        /// Different source and reference samples and strides, every grid
        /// shape up to 3x3, and slices one sample short of the grid.
        #[test]
        fn sads_match_scalar(
            (cols, rows, src_stride, ref_stride) in (1usize..4, 1usize..4, 0usize..5, 0usize..9),
            seed in samples(4096),
            short in any::<bool>(),
        ) {
            let src_stride = cols * 8 + src_stride;
            let ref_stride = cols * 8 + WAYS - 1 + ref_stride;
            let src: Vec<u8> = seed.iter().take(rows * 8 * src_stride).copied().collect();
            let mut refs: Vec<u8> =
                seed.iter().rev().map(|s| s.wrapping_mul(7) ^ 0x5a).take(rows * 8 * ref_stride).collect();
            if short {
                refs.truncate((rows * 8 - 1) * ref_stride + cols * 8 + WAYS - 2);
            }
            let (mut scalar, mut simd) = (vec![[1; WAYS]; cols * rows], vec![[2; WAYS]; cols * rows]);
            sad8x8_x4_scalar((&src, src_stride), (&refs, ref_stride), cols, &mut scalar);
            sad8x8_x4_simd((&src, src_stride), (&refs, ref_stride), cols, &mut simd);
            prop_assert_eq!(&scalar, &simd);
            prop_assert_eq!(short, scalar[0][0] == u32::MAX);
        }

        /// Odd grid sides, short outputs, and sums that wrap.
        #[test]
        fn quads_and_best_match_scalar(
            cols in 1usize..7,
            values in prop::collection::vec(
                prop_oneof![3 => 0u32..20_000, 1 => any::<u32>()].prop_map(|v| [v, v / 3, v ^ 0x55, v.rotate_left(3)]),
                0..49,
            ),
            prior in prop::collection::vec(prop_oneof![Just(u32::MAX), 0u32..30_000], 0..30),
            mv in any::<[i16; 2]>(),
        ) {
            let mut scalar = vec![[7; WAYS]; values.len() / 3];
            let mut simd = scalar.clone();
            sad_quads_scalar(&values, cols, &mut scalar);
            sad_quads_simd(&values, cols, &mut simd);
            prop_assert_eq!(scalar, simd);
            let mut scalar: Vec<Motion> = prior.iter().map(|&sad| Motion { sad, mv: [3, -4] }).collect();
            let mut simd = scalar.clone();
            keep_best_scalar(&values, mv, &mut scalar);
            keep_best_simd(&values, mv, &mut simd);
            prop_assert_eq!(scalar, simd);
        }

        /// Rows of any length, `top` and `bottom` of different lengths.
        #[test]
        fn downscale_matches_scalar(
            top in prop::collection::vec(any::<u8>(), 0..80),
            bottom in prop::collection::vec(any::<u8>(), 0..80),
            out_len in 0usize..41,
        ) {
            let (mut scalar, mut simd) = (vec![9; out_len], vec![9; out_len]);
            downscale_row_scalar(&top, &bottom, &mut scalar);
            downscale_row_simd(&top, &bottom, &mut simd);
            prop_assert_eq!(scalar, simd);
        }
    }

    #[test]
    fn lanes_are_horizontal_positions() {
        let stride = 16 + WAYS - 1;
        let refs: Vec<u8> = (0..8 * stride).map(|i| (i * 37 % 251) as u8).collect();
        let src: Vec<u8> = (0..8 * 16)
            .map(|i| refs[i / 16 * stride + i % 16 + 2])
            .collect();
        let mut sads = [[0; WAYS]; 2];
        sad8x8_x4_simd((&src, 16), (&refs, stride), 2, &mut sads);
        assert!(
            sads.iter().all(|s| s[2] == 0 && s[0] > 0 && s[3] > 0),
            "{sads:?}"
        );
        let mut best = [Motion::NONE, Motion { sad: 0, mv: [9, 9] }];
        keep_best_simd(&sads, [-1, 5], &mut best);
        assert_eq!(
            best,
            [Motion { sad: 0, mv: [1, 5] }, Motion { sad: 0, mv: [9, 9] }]
        );
        let mut half = [0; 2];
        downscale_row_simd(&[0, 1, 255, 255], &[1, 1, 255, 254], &mut half);
        assert_eq!(half, [0, 254]);
    }
}
