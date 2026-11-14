// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Blocks of at most 64 samples with their rows and columns in registers.

use core::arch::aarch64::*;

use fearless_simd::Neon;

use super::{
    COL8, DCT_TABLES, DctTables, FlatDequant, Pass, add8, lanes, load_prefix, narrow, reverse,
    scale, scaled,
};

/// An 8x8 block with its rows in registers: no scratch and no loops over
/// a runtime size.
#[inline]
#[target_feature(enable = "neon")]
pub(super) fn block8(
    t: Neon,
    (row, col): (Pass, Pass),
    (coeffs, load): (&[i32], (Option<FlatDequant>, bool)),
    (row_shift, col_shift): (int32x4_t, int32x4_t),
    (dst, stride): (&mut [u8], usize),
) {
    let zero = vdupq_n_s16(0);
    let mut inputs = [zero; 8];
    let coded = load_prefix::<false>(t, coeffs, load, true, |i, v| inputs[i % 8] = v);
    macro_rules! rows {
        (|$x:ident| $kernel:expr) => {
            core::array::from_fn(|r| {
                if r < coded {
                    let $x = inputs[r];
                    let [lo, hi]: [int32x4_t; 2] = $kernel;
                    narrow(lo, hi, row_shift)
                } else {
                    zero
                }
            })
        };
    }
    let y: [int16x8_t; 8] = match row {
        Pass::Identity(s) => rows!(|x| [vmull_n_s16(vget_low_s16(x), s), vmull_high_n_s16(x, s)]),
        Pass::Eight(k) => rows!(|x| eight!(t, k, x)),
        _ => rows!(|x| dct8!(t, &DCT_TABLES, [x])),
    };
    let put = |dst: &mut [u8], o: usize, [lo, hi]: [int32x4_t; 2]| {
        add8(dst, o * stride, narrow(lo, hi, col_shift));
    };
    let zero32 = vdupq_n_s32(0);
    match col {
        Pass::Identity(s) => {
            for (r, &y) in y.iter().enumerate().take(coded) {
                add8(dst, r * stride, scale(y, s, col_shift));
            }
        }
        Pass::Eight(k) => {
            let mut acc = [[zero32; 2]; 8];
            for (r, &y) in y.iter().enumerate() {
                let kv = lanes(t, &k[r][0]);
                col_mac!(acc, y, kv; 0 0 1 1 2 2 3 3 4 4 5 5 6 6 7 7);
            }
            for (o, &acc) in acc.iter().enumerate() {
                put(dst, o, acc);
            }
        }
        _ => {
            let (mut even, mut odd) = ([[zero32; 2]; 4], [[zero32; 2]; 4]);
            for (p, pair) in y.as_chunks::<2>().0.iter().enumerate() {
                let kv = lanes(t, &COL8[p][0]);
                col_mac!(even, pair[0], kv; 0 0 1 1 2 2 3 3);
                col_mac!(odd, pair[1], kv; 0 4 1 5 2 6 3 7);
            }
            for (o, (e, d)) in even.iter().zip(&odd).enumerate() {
                put(dst, o, [vaddq_s32(e[0], d[0]), vaddq_s32(e[1], d[1])]);
                put(dst, 7 - o, [vsubq_s32(e[0], d[0]), vsubq_s32(e[1], d[1])]);
            }
        }
    }
}

/// A 4x4 block: rows `0, 1` and `2, 3` in two registers.
#[inline]
#[target_feature(enable = "neon")]
pub(super) fn block4x4(
    t: Neon,
    (row, col): (Pass, Pass),
    x: [int16x8_t; 2],
    shifts: (int32x4_t, int32x4_t),
) -> [int16x8_t; 2] {
    let y = x.map(|x| {
        let [a, b] = match row {
            Pass::Four(k) => four!(t, k, x),
            Pass::Identity(s) => scaled::<1, 2>([x], s),
            _ => [vdupq_n_s32(0); 2],
        };
        narrow(a, b, shifts.0)
    });
    match col {
        Pass::Four(k) => {
            let kv = [0, 1, 2, 3].map(|r| lanes(t, &k[r][0]));
            let mut acc = [vdupq_n_s32(0); 4];
            col_mac4!(acc, acc, y[0], kv[0], kv[1]; 0 0 0 1 1 1 2 2 2 3 3 3);
            col_mac4!(acc, acc, y[1], kv[2], kv[3]; 0 0 0 1 1 1 2 2 2 3 3 3);
            [
                narrow(acc[0], acc[1], shifts.1),
                narrow(acc[2], acc[3], shifts.1),
            ]
        }
        Pass::Identity(s) => y.map(|y| scale(y, s, shifts.1)),
        _ => [vdupq_n_s16(0); 2],
    }
}

/// An 8x4 block: one row per register.
#[inline]
#[target_feature(enable = "neon")]
pub(super) fn block8x4(
    t: Neon,
    (row, col): (Pass, Pass),
    x: [int16x8_t; 4],
    shifts: (int32x4_t, int32x4_t),
) -> [int16x8_t; 4] {
    let y = x.map(|x| {
        let [a, b] = match row {
            Pass::Eight(k) => eight!(t, k, x),
            Pass::Identity(s) => scaled::<1, 2>([x], s),
            _ => dct8!(t, &DCT_TABLES, [x]),
        };
        narrow(a, b, shifts.0)
    });
    match col {
        Pass::Four(k) => {
            let mut acc = [[vdupq_n_s32(0); 2]; 4];
            for (r, &y) in y.iter().enumerate() {
                let kv = lanes(t, &k[r][0]);
                col_mac!(acc, y, kv; 0 0 1 1 2 2 3 3);
            }
            acc.map(|[lo, hi]| narrow(lo, hi, shifts.1))
        }
        Pass::Identity(s) => y.map(|y| scale(y, s, shifts.1)),
        _ => [vdupq_n_s16(0); 4],
    }
}

/// A 4x8 block: row pairs `2p, 2p + 1` in four registers.
#[inline]
#[target_feature(enable = "neon")]
pub(super) fn block4x8(
    t: Neon,
    (row, col): (Pass, Pass),
    x: [int16x8_t; 4],
    shifts: (int32x4_t, int32x4_t),
) -> [int16x8_t; 4] {
    let y = x.map(|x| {
        let [a, b] = match row {
            Pass::Four(k) => four!(t, k, x),
            Pass::Identity(s) => scaled::<1, 2>([x], s),
            _ => [vdupq_n_s32(0); 2],
        };
        narrow(a, b, shifts.0)
    });
    let zero = vdupq_n_s32(0);
    match col {
        Pass::Eight(k) => {
            let (mut upper, mut lower) = ([zero; 8], [zero; 8]);
            for (p, &y) in y.iter().enumerate() {
                let (k0, k1) = (lanes(t, &k[2 * p][0]), lanes(t, &k[2 * p + 1][0]));
                col_mac4!(upper, lower, y, k0, k1; 0 0 0 1 1 1 2 2 2 3 3 3 4 4 4 5 5 5 6 6 6 7 7 7);
            }
            let sum = |i: usize| vaddq_s32(upper[i], lower[i]);
            core::array::from_fn(|p| narrow(sum(2 * p), sum(2 * p + 1), shifts.1))
        }
        Pass::Identity(s) => y.map(|y| scale(y, s, shifts.1)),
        _ => {
            let (mut even, mut odd) = ([zero; 4], [zero; 4]);
            for (p, &y) in y.iter().enumerate() {
                let kv = lanes(t, &COL8[p][0]);
                col_mac4!(even, odd, y, kv, kv; 0 0 4 1 1 5 2 2 6 3 3 7);
            }
            let sum = |i: usize| vaddq_s32(even[i], odd[i]);
            let diff = |i: usize| vsubq_s32(even[i], odd[i]);
            [
                narrow(sum(0), sum(1), shifts.1),
                narrow(sum(2), sum(3), shifts.1),
                narrow(diff(3), diff(2), shifts.1),
                narrow(diff(1), diff(0), shifts.1),
            ]
        }
    }
}
