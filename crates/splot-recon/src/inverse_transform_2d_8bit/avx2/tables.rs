// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The § 9.6 kernel tables of the AVX2 transform as `pmaddwd` pairs and the
//! `pshufb` orders that read them; their identities are checked at compile
//! time.

use splot_tables::tables::transform_1d::{
    ADST_KERNEL4, ADST_KERNEL8, ADST_KERNEL16, DCT_KERNEL4, DCT_KERNEL8, DCT_KERNEL16,
    DCT_KERNEL32, DDTX_KERNEL8, DDTX_KERNEL16, FDST_KERNEL4, FDST_KERNEL8, FDST_KERNEL16,
};

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

pub(super) static DCT4: [[u32; 4]; 2] = pair_kernel(&DCT_KERNEL4, false);
pub(super) static ADST4: [[u32; 4]; 2] = pair_kernel(&ADST_KERNEL4, false);
pub(super) static FDST4: [[u32; 4]; 2] = pair_kernel(&FDST_KERNEL4, false);
pub(super) static DCT8: [[u32; 8]; 4] = pair_kernel(&DCT_KERNEL8, false);
pub(super) static ADST8: [[u32; 8]; 4] = pair_kernel(&ADST_KERNEL8, false);
pub(super) static FDST8: [[u32; 8]; 4] = pair_kernel(&FDST_KERNEL8, false);
pub(super) static DDTX8: [[u32; 8]; 4] = pair_kernel(&DDTX_KERNEL8, false);
pub(super) static FDDT8: [[u32; 8]; 4] = pair_kernel(&DDTX_KERNEL8, true);
pub(super) static DCT16: [[u32; 16]; 8] = pair_kernel(&DCT_KERNEL16, false);
pub(super) static ADST16: [[u32; 16]; 8] = pair_kernel(&ADST_KERNEL16, false);
pub(super) static FDST16: [[u32; 16]; 8] = pair_kernel(&FDST_KERNEL16, false);
pub(super) static DDTX16: [[u32; 16]; 8] = pair_kernel(&DDTX_KERNEL16, false);
pub(super) static FDDT16: [[u32; 16]; 8] = pair_kernel(&DDTX_KERNEL16, true);
pub(super) static DCT32: [[u32; 32]; 16] = pair_kernel(&DCT_KERNEL32, false);
pub(super) static DCT8_SPLIT: [[u32; 8]; 4] = split_pair_kernel(&DCT_KERNEL8);
pub(super) static DCT16_SPLIT: [[u32; 16]; 8] = split_pair_kernel(&DCT_KERNEL16);
pub(super) static DCT32_EVEN: [[[u32; 8]; 2]; 4] = dct32_even_pairs();
pub(super) static DCT32_ODD: [[[u32; 8]; 2]; 8] = dct32_odd_pairs();

/// The `DCT32` rows of the two-level row split, by group `g` of inputs
/// `8g..8g + 8`: rows `8g, 8g + 4` (`EE`) and `8g + 2, 8g + 6` (`EO`) over
/// outputs `0..8`, each symmetric (`EE`) or antisymmetric (`EO`) about
/// `7.5` within the first 16 outputs.
#[allow(
    clippy::many_single_char_names,
    reason = "kernel k, rows a to d, group g and output n, as in § 7.15.2.1"
)]
const fn dct32_even_pairs() -> [[[u32; 8]; 2]; 4] {
    let k = &DCT_KERNEL32;
    let mut out = [[[0u32; 8]; 2]; 4];
    let mut g = 0;
    while g < 4 {
        let mut n = 0;
        while n < 8 {
            let (a, b, c, d) = (8 * g, 8 * g + 4, 8 * g + 2, 8 * g + 6);
            assert!(k[a][15 - n] == k[a][n] && k[b][15 - n] == k[b][n]);
            assert!(k[c][15 - n] == -k[c][n] && k[d][15 - n] == -k[d][n]);
            out[g][0][n] = pack(k[a][n], k[b][n]);
            out[g][1][n] = pack(k[c][n], k[d][n]);
            n += 1;
        }
        g += 1;
    }
    out
}

/// The odd `DCT32` row pairs `4s + 1, 4s + 3` over outputs `0..8` and,
/// reversed, `8..16`, each row antisymmetric about `15.5`.
#[allow(
    clippy::many_single_char_names,
    reason = "kernel k, rows a to d, group g and output n, as in § 7.15.2.1"
)]
const fn dct32_odd_pairs() -> [[[u32; 8]; 2]; 8] {
    let k = &DCT_KERNEL32;
    let mut out = [[[0u32; 8]; 2]; 8];
    let mut s = 0;
    while s < 8 {
        let (a, b) = (4 * s + 1, 4 * s + 3);
        let mut n = 0;
        while n < 16 {
            assert!(k[a][31 - n] == -k[a][n] && k[b][31 - n] == -k[b][n]);
            n += 1;
        }
        n = 0;
        while n < 8 {
            out[s][0][n] = pack(k[a][n], k[b][n]);
            out[s][1][n] = pack(k[a][15 - n], k[b][15 - n]);
            n += 1;
        }
        s += 1;
    }
    out
}

/// The `pshufb` that orders the samples `x0..x8` of a lane as the pairs
/// `(x0, x4), (x2, x6), (x1, x3), (x5, x7)` of the two-level split.
pub(super) const TWO_LEVEL_ORDER: [u8; 32] = {
    let words = [0, 4, 2, 6, 1, 3, 5, 7];
    let mut order = [0u8; 32];
    let mut i = 0;
    while i < 32 {
        order[i] = (2 * words[i % 16 / 2] + i % 2) as u8;
        i += 1;
    }
    order
};

/// [`pair_kernel`] over the row pairs `4m + j, 4m + j + 2` (pair `2m + j`)
/// of a split `DCT` column pass, for the first `N / 2` outputs.
const fn split_pair_kernel<const N: usize, const H: usize>(
    kernel: &[[i32; N]; N],
) -> [[u32; N]; H] {
    assert!(2 * H == N);
    let mut out = [[0u32; N]; H];
    let mut q = 0;
    while q < H {
        let a = 4 * (q / 2) + q % 2;
        let b = a + 2;
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

/// The `pshufb` that swaps the middle two samples of every four.
pub(super) const SPLIT_ORDER: [u8; 32] = {
    let mut order = [0u8; 32];
    let mut i = 0;
    while i < 32 {
        let sample = i % 16 / 2;
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

pub(super) const fn pack(low: i32, high: i32) -> u32 {
    assert!(low == low as i16 as i32 && high == high as i16 as i32);
    (low as u16 as u32) | ((high as u16 as u32) << 16)
}

/// `Dct_KernelN` odd-row pairs `4p + 1, 4p + 3` at `[p]` over outputs
/// `0..N / 2`, checking each odd row's antisymmetry.
const fn odd_pairs<const N: usize, const H: usize, const P: usize>(
    kernel: &[[i32; N]; N],
) -> [[u32; H]; P] {
    assert!(2 * H == N && 4 * P == N);
    let mut out = [[0u32; H]; P];
    let mut p = 0;
    while p < P {
        let mut i = 0;
        while i < H {
            let (a, b) = (kernel[4 * p + 1], kernel[4 * p + 3]);
            assert!(a[N - 1 - i] == -a[i] && b[N - 1 - i] == -b[i]);
            out[p][i] = pack(a[i], b[i]);
            i += 1;
        }
        p += 1;
    }
    out
}

/// `Dct_Kernel2N[2i][j] == Dct_KernelN[i][j]`, and the even rows of
/// `Dct_KernelN` symmetric.
const fn halves<const N: usize, const M: usize>(half: &[[i32; N]; N], full: &[[i32; M]; M]) {
    assert!(2 * N == M);
    let mut i = 0;
    while i < N {
        let mut j = 0;
        while j < N {
            assert!(full[2 * i][j] == half[i][j]);
            if i % 2 == 0 {
                assert!(half[i][N - 1 - j] == half[i][j]);
            }
            j += 1;
        }
        i += 1;
    }
}

const _: () = {
    halves(&DCT_KERNEL4, &DCT_KERNEL8);
    halves(&DCT_KERNEL8, &DCT_KERNEL16);
    halves(&DCT_KERNEL16, &DCT_KERNEL32);
    assert!(DCT_KERNEL4[0][0] == 64 && DCT_KERNEL4[0][1] == 64);
    assert!(DCT_KERNEL4[2][0] == 64 && DCT_KERNEL4[2][1] == -64);
};

pub(super) static ODD4: [[u32; 2]; 1] = odd_pairs(&DCT_KERNEL4);
pub(super) static ODD8: [[u32; 4]; 2] = odd_pairs(&DCT_KERNEL8);
pub(super) static ODD16: [[u32; 8]; 4] = odd_pairs(&DCT_KERNEL16);
pub(super) static ODD32: [[u32; 16]; 8] = odd_pairs(&DCT_KERNEL32);

/// The row pairs of an `m`-point `DCT` column in the order its factorization
/// reads them: those of the half-size `DCT` on the even rows, then the odd
/// pairs `4p + 1, 4p + 3`.
pub(super) const fn dct_pairs(m: usize) -> [(u8, u8); 16] {
    let mut out = [(0u8, 1u8); 16];
    let (mut len, mut size) = (1, 2);
    while size < m {
        let mut i = 0;
        while i < len {
            out[i] = (2 * out[i].0, 2 * out[i].1);
            i += 1;
        }
        size *= 2;
        let mut p = 0;
        while p < size / 4 {
            out[len + p] = ((4 * p + 1) as u8, (4 * p + 3) as u8);
            p += 1;
        }
        len += size / 4;
    }
    out
}

pub(super) static DCT_PAIRS: [[(u8, u8); 16]; 4] =
    [dct_pairs(4), dct_pairs(8), dct_pairs(16), dct_pairs(32)];

/// Adjacent row pairs `2q, 2q + 1`.
pub(super) static ADJACENT_PAIRS: [(u8, u8); 16] = {
    let mut out = [(0u8, 0u8); 16];
    let mut q = 0;
    while q < 16 {
        out[q] = (2 * q as u8, 2 * q as u8 + 1);
        q += 1;
    }
    out
};

/// `pshufb` interleaving the `i16` samples `0..4` and `4..8` of a lane, in
/// order or (`reversed`) from the last.
const fn interleave(low_reversed: bool, high_reversed: bool) -> [u8; 32] {
    let mut out = [0u8; 32];
    let mut i = 0;
    while i < 32 {
        let reversed = if i < 16 { low_reversed } else { high_reversed };
        let pair = (i % 16) / 4;
        let sample = if reversed { 3 - pair } else { pair } + 4 * ((i % 4) / 2);
        out[i] = (2 * sample + i % 2) as u8;
        i += 1;
    }
    out
}

pub(super) static INTERLEAVE: [u8; 32] = interleave(false, false);
pub(super) static INTERLEAVE_HIGH_REVERSED: [u8; 32] = interleave(false, true);
pub(super) static INTERLEAVE_REVERSED: [u8; 32] = interleave(true, true);
