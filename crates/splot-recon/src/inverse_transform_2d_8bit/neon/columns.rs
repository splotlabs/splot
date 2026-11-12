// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The column pass: the block's columns in lanes, `DCT` columns split into
//! even and odd rows, twice from 16 rows.

use core::arch::aarch64::*;

use fearless_simd::Neon;

use super::{
    COL8, COL16, COL32, EVEN16, EVEN32, Half, Lanes, ODD16C, ODD32C, Pass, add4x2, add8, get16,
    lanes, narrow,
};

/// The row outputs of a block and its destination.
pub(super) struct Columns<'a> {
    /// The `i16` row outputs, rows `w` apart.
    pub(super) rows: &'a Half,
    pub(super) w: usize,
    /// Rows of `rows` to read, even.
    pub(super) coded: usize,
    /// `-colShift`.
    pub(super) shift: int32x4_t,
    pub(super) dst: &'a mut [u8],
    pub(super) stride: usize,
}

impl Columns<'_> {
    /// Row `r`, columns `8 * c..8 * c + 8` (rows `r` and `r + 1` of a
    /// 4-wide block).
    #[inline]
    #[target_feature(enable = "neon")]
    fn row(&self, t: Neon, r: usize, c: usize) -> int16x8_t {
        get16(t, self.rows, r * self.w + 8 * c)
    }

    /// Adds output row `o` of column chunk `c` from its sums.
    #[inline]
    #[target_feature(enable = "neon")]
    fn put(&mut self, o: usize, c: usize, [lo, hi]: [int32x4_t; 2]) {
        add8(
            self.dst,
            o * self.stride + 8 * c,
            narrow(lo, hi, self.shift),
        );
    }

    /// Adds output rows `o` and `o + 1` of a 4-wide block from their sums.
    #[inline]
    #[target_feature(enable = "neon")]
    fn put4(&mut self, o: usize, a: int32x4_t, b: int32x4_t) {
        add4x2(
            self.dst,
            o * self.stride,
            self.stride,
            narrow(a, b, self.shift),
        );
    }
}

/// `acc[o] += y * kv[l]` for each pair `o l`, both halves of `y`.
macro_rules! col_mac {
    ($acc:ident, $y:expr, $kv:expr; $($o:literal $l:literal)*) => {
        $(
            $acc[$o][0] = vmlal_laneq_s16::<$l>($acc[$o][0], vget_low_s16($y), $kv);
            $acc[$o][1] = vmlal_high_laneq_s16::<$l>($acc[$o][1], $y, $kv);
        )*
    };
}

/// `a[o] += low(y) * k0[l0]` and `b[o] += high(y) * k1[l1]` for each `o l0 l1`.
macro_rules! col_mac4 {
    ($a:ident, $b:ident, $y:expr, $k0:expr, $k1:expr; $($o:literal $l0:literal $l1:literal)*) => {
        $(
            $a[$o] = vmlal_laneq_s16::<$l0>($a[$o], vget_low_s16($y), $k0);
            $b[$o] = vmlal_high_laneq_s16::<$l1>($b[$o], $y, $k1);
        )*
    };
}

/// The § 7.15.2.3 identity column pass of `scale`.
#[inline]
#[target_feature(enable = "neon")]
fn col_identity(t: Neon, scale: i16, cols: &mut Columns<'_>) {
    if cols.w == 4 {
        for r in (0..cols.coded).step_by(2) {
            let y = cols.row(t, r, 0);
            cols.put4(
                r,
                vmull_n_s16(vget_low_s16(y), scale),
                vmull_high_n_s16(y, scale),
            );
        }
        return;
    }
    for r in 0..cols.coded {
        for c in 0..cols.w / 8 {
            let y = cols.row(t, r, c);
            cols.put(
                r,
                c,
                [
                    vmull_n_s16(vget_low_s16(y), scale),
                    vmull_high_n_s16(y, scale),
                ],
            );
        }
    }
}

/// The column pass of an `M`-point kernel `k` other than a split `DCT`.
#[inline]
#[target_feature(enable = "neon")]
fn col_matrix<const M: usize, const G: usize>(
    t: Neon,
    k: &[[Lanes; G]; M],
    cols: &mut Columns<'_>,
) {
    let zero = vdupq_n_s32(0);
    let kernel = |r: usize, g: usize| lanes(t, &k[r % M][g % G]);
    if cols.w == 4 {
        for g in 0..G {
            let (mut upper, mut lower) = ([zero; 8], [zero; 8]);
            for r in (0..cols.coded).step_by(2) {
                let (y, k0, k1) = (cols.row(t, r, 0), kernel(r, g), kernel(r + 1, g));
                if M == 4 {
                    col_mac4!(upper, lower, y, k0, k1; 0 0 0 1 1 1 2 2 2 3 3 3);
                } else {
                    col_mac4!(upper, lower, y, k0, k1; 0 0 0 1 1 1 2 2 2 3 3 3 4 4 4 5 5 5 6 6 6 7 7 7);
                }
            }
            for o in (0..M.min(8)).step_by(2) {
                let sum = |i: usize| vaddq_s32(upper[i], lower[i]);
                cols.put4(8 * g + o, sum(o), sum(o + 1));
            }
        }
        return;
    }
    for c in 0..cols.w / 8 {
        for g in 0..G {
            let mut acc = [[zero; 2]; 8];
            for r in 0..cols.coded {
                let (y, kv) = (cols.row(t, r, c), kernel(r, g));
                if M == 4 {
                    col_mac!(acc, y, kv; 0 0 1 1 2 2 3 3);
                } else {
                    col_mac!(acc, y, kv; 0 0 1 1 2 2 3 3 4 4 5 5 6 6 7 7);
                }
            }
            for (o, &acc) in acc.iter().enumerate().take(M.min(8)) {
                cols.put(8 * g + o, c, acc);
            }
        }
    }
}

/// The column pass of the `M`-point `DCT` by its row pairs `k`: the even
/// rows sum `E` and the odd rows `O` over the first `M / 2` outputs, and
/// output `M - 1 - o` is `E - O` where output `o` is `E + O`.
#[inline]
#[target_feature(enable = "neon")]
fn col_dct<const M: usize, const H: usize, const G: usize>(
    t: Neon,
    k: &[[Lanes; G]; H],
    cols: &mut Columns<'_>,
) {
    let zero = vdupq_n_s32(0);
    let kernel = |r: usize, g: usize| lanes(t, &k[(r / 2) % H][g % G]);
    if cols.w == 4 {
        for g in 0..G {
            let (mut even, mut odd) = ([zero; 4], [zero; 4]);
            for r in (0..cols.coded).step_by(2) {
                let (y, kv) = (cols.row(t, r, 0), kernel(r, g));
                col_mac4!(even, odd, y, kv, kv; 0 0 4 1 1 5 2 2 6 3 3 7);
            }
            let sum = |i: usize| vaddq_s32(even[i], odd[i]);
            let diff = |i: usize| vsubq_s32(even[i], odd[i]);
            for o in [0, 2] {
                cols.put4(4 * g + o, sum(o), sum(o + 1));
                cols.put4(M - 2 - 4 * g - o, diff(o + 1), diff(o));
            }
        }
        return;
    }
    for c in 0..cols.w / 8 {
        for g in 0..G {
            let (mut even, mut odd) = ([[zero; 2]; 4], [[zero; 2]; 4]);
            for r in (0..cols.coded).step_by(2) {
                let kv = kernel(r, g);
                col_mac!(even, cols.row(t, r, c), kv; 0 0 1 1 2 2 3 3);
                col_mac!(odd, cols.row(t, r + 1, c), kv; 0 4 1 5 2 6 3 7);
            }
            for (o, (e, d)) in even.iter().zip(&odd).enumerate() {
                cols.put(4 * g + o, c, [vaddq_s32(e[0], d[0]), vaddq_s32(e[1], d[1])]);
                cols.put(
                    M - 1 - 4 * g - o,
                    c,
                    [vsubq_s32(e[0], d[0]), vsubq_s32(e[1], d[1])],
                );
            }
        }
    }
}

/// The column pass of the `M`-point `DCT`, `M >= 16`, over a block at
/// least 8 wide, split twice: rows `0 mod 4` sum `EE` and rows `2 mod 4`
/// sum `EO` over the first `M / 4` outputs, giving the even-row sums `E`
/// over the first `M / 2` (`EE + EO`, then `EE - EO` reversed), which
/// combine with the odd-row sums `O` as in [`col_dct`]. `even` and `odd`
/// are the [`quads`] of the kernel.
#[allow(
    clippy::needless_range_loop,
    reason = "the group index also places the outputs"
)]
#[inline]
#[target_feature(enable = "neon")]
fn col_dct2<const M: usize, const Q: usize, const GE: usize, const GO: usize>(
    t: Neon,
    (even, odd): (&[[Lanes; GE]; Q], &[[Lanes; GO]; Q]),
    cols: &mut Columns<'_>,
) {
    let zero = vdupq_n_s32(0);
    let quads = cols.coded.div_ceil(4);
    for c in 0..cols.w / 8 {
        let mut e = [[zero; 2]; 16];
        for g in 0..GE {
            let (mut ee, mut eo) = ([[zero; 2]; 4], [[zero; 2]; 4]);
            for q in 0..quads {
                let kv = lanes(t, &even[q % Q][g]);
                col_mac!(ee, cols.row(t, 4 * q, c), kv; 0 0 1 1 2 2 3 3);
                col_mac!(eo, cols.row(t, 4 * q + 2, c), kv; 0 4 1 5 2 6 3 7);
            }
            for o in 0..4 {
                let (a, b) = (ee[o], eo[o]);
                e[(4 * g + o) % 16] = [vaddq_s32(a[0], b[0]), vaddq_s32(a[1], b[1])];
                e[(M / 2 - 1 - 4 * g - o) % 16] = [vsubq_s32(a[0], b[0]), vsubq_s32(a[1], b[1])];
            }
        }
        for g in 0..GO {
            let mut o4 = [[zero; 2]; 4];
            let mut o4b = [[zero; 2]; 4];
            for q in 0..quads {
                let kv = lanes(t, &odd[q % Q][g]);
                col_mac!(o4, cols.row(t, 4 * q + 1, c), kv; 0 0 1 1 2 2 3 3);
                col_mac!(o4b, cols.row(t, 4 * q + 3, c), kv; 0 4 1 5 2 6 3 7);
            }
            for o in 0..4 {
                let i = 4 * g + o;
                let ev = e[i % 16];
                let d = [0, 1].map(|half| vaddq_s32(o4[o][half], o4b[o][half]));
                cols.put(i, c, [vaddq_s32(ev[0], d[0]), vaddq_s32(ev[1], d[1])]);
                cols.put(
                    M - 1 - i,
                    c,
                    [vsubq_s32(ev[0], d[0]), vsubq_s32(ev[1], d[1])],
                );
            }
        }
    }
}

/// The column pass of `pass`.
#[inline]
#[target_feature(enable = "neon")]
pub(super) fn col_pass(t: Neon, pass: Pass, cols: &mut Columns<'_>) {
    match pass {
        Pass::Identity(scale) => col_identity(t, scale, cols),
        Pass::Four(k) => col_matrix(t, k, cols),
        Pass::Eight(k) => col_matrix(t, k, cols),
        Pass::Sixteen(k) => col_matrix(t, k, cols),
        Pass::Dct(8) => col_dct::<8, 4, 1>(t, &COL8, cols),
        Pass::Dct(16) if cols.w == 4 => col_dct::<16, 8, 2>(t, &COL16, cols),
        Pass::Dct(16) => col_dct2::<16, 4, 1, 2>(t, (&EVEN16, &ODD16C), cols),
        Pass::Dct(_) if cols.w == 4 => col_dct::<32, 16, 4>(t, &COL32, cols),
        Pass::Dct(_) => col_dct2::<32, 8, 2, 4>(t, (&EVEN32, &ODD32C), cols),
    }
}
