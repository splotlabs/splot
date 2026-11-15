// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! The row pass: each row's outputs in lanes, one `smlal` by lane per
//! product, `DCT` rows split into even and odd inputs at every length.

use core::arch::aarch64::*;

use fearless_simd::Neon;

use super::{DCT_TABLES, DctTables, Half, Pass, get16, lanes, narrow, reverse, tables};

/// `acc += k[j][0][0..4] * x[l]` for each `(j, l)`.
macro_rules! mac4 {
    ($t:ident, $acc:ident, $k:expr, $x:expr; $($j:literal $l:literal),*) => {
        $(
            $acc = vmlal_laneq_s16::<$l>($acc, vget_low_s16(lanes($t, &$k[$j][0])), $x);
        )*
    };
}

/// `(lo, hi) += k[j0 + l][g] * x[l]` for each lane `l`.
macro_rules! mac8 {
    ($t:ident, $lo:ident, $hi:ident, $k:expr, $j0:literal, $g:literal, $x:expr) => {
        mac8!(@ $t, $lo, $hi, $k, $j0, $g, $x; 0 1 2 3 4 5 6 7)
    };
    (@ $t:ident, $lo:ident, $hi:ident, $k:expr, $j0:literal, $g:literal, $x:expr; $($l:literal)*) => {
        $(
            let kv = lanes($t, &$k[$j0 + $l][$g]);
            $lo = vmlal_laneq_s16::<$l>($lo, vget_low_s16(kv), $x);
            $hi = vmlal_high_laneq_s16::<$l>($hi, kv, $x);
        )*
    };
}

/// Two 4-wide rows in the low and high lanes of `x` by the 4-point kernel
/// `k`.
macro_rules! four {
    ($t:ident, $k:expr, $x:expr) => {{
        let (mut a, mut b, k, x) = (vdupq_n_s32(0), vdupq_n_s32(0), $k, $x);
        mac4!($t, a, k, x; 0 0, 1 1, 2 2, 3 3);
        mac4!($t, b, k, x; 0 4, 1 5, 2 6, 3 7);
        [a, b]
    }};
}

/// One 8-wide row `x` by the 8-point kernel `k`.
macro_rules! eight {
    ($t:ident, $k:expr, $x:expr) => {{
        let (mut lo, mut hi, k, x) = (vdupq_n_s32(0), vdupq_n_s32(0), $k, $x);
        mac8!($t, lo, hi, k, 0, 0, x);
        [lo, hi]
    }};
}

/// One 16-wide row `x` by the 16-point kernel `k`.
macro_rules! sixteen {
    ($t:ident, $k:expr, $x:expr) => {{
        let ([mut a, mut b, mut c, mut d], k, [x0, x1]) = ([vdupq_n_s32(0); 4], $k, $x);
        mac8!($t, a, b, k, 0, 0, x0);
        mac8!($t, c, d, k, 0, 1, x0);
        mac8!($t, a, b, k, 8, 0, x1);
        mac8!($t, c, d, k, 8, 1, x1);
        [a, b, c, d]
    }};
}

/// The 8-point `DCT` of `x`: the even lanes by the 4-point `DCT`, the odd
/// lanes by the odd rows.
macro_rules! dct8 {
    ($t:ident, $k:expr, $x:expr) => {{
        let (mut even, mut odd, [x]) = (vdupq_n_s32(0), vdupq_n_s32(0), $x);
        let k: &DctTables = $k;
        mac4!($t, even, k.0, x; 0 0, 1 2, 2 4, 3 6);
        mac4!($t, odd, k.1, x; 0 1, 1 3, 2 5, 3 7);
        [vaddq_s32(even, odd), reverse(vsubq_s32(even, odd))]
    }};
}

/// The 16-point `DCT` of `x`.
macro_rules! dct16 {
    ($t:ident, $k:expr, $x:expr) => {{
        let ([x0, x1], k): ([int16x8_t; 2], &DctTables) = ($x, $k);
        let [e0, e1] = dct8!($t, k, [vuzp1q_s16(x0, x1)]);
        let [o0, o1] = eight!($t, k.2, vuzp2q_s16(x0, x1));
        [
            vaddq_s32(e0, o0),
            vaddq_s32(e1, o1),
            reverse(vsubq_s32(e1, o1)),
            reverse(vsubq_s32(e0, o0)),
        ]
    }};
}

/// The 32-point `DCT` of `x`.
macro_rules! dct32 {
    ($t:ident, $k:expr, $x:expr) => {{
        let ([x0, x1, x2, x3], k): ([int16x8_t; 4], &DctTables) = ($x, $k);
        let even = dct16!($t, k, [vuzp1q_s16(x0, x1), vuzp1q_s16(x2, x3)]);
        let odd = sixteen!($t, k.3, [vuzp2q_s16(x0, x1), vuzp2q_s16(x2, x3)]);
        let mut out = [vdupq_n_s32(0); 8];
        for i in 0..4 {
            out[i] = vaddq_s32(even[i], odd[i]);
            out[7 - i] = reverse(vsubq_s32(even[i], odd[i]));
        }
        out
    }};
}

/// The products of `x` and the identity scale `s`.
#[inline]
#[target_feature(enable = "neon")]
pub(super) fn scaled<const N: usize, const M: usize>(x: [int16x8_t; N], s: i16) -> [int32x4_t; M] {
    let mut sums = [vdupq_n_s32(0); M];
    for (k, x) in x.into_iter().enumerate() {
        sums[2 * k] = vmull_n_s16(vget_low_s16(x), s);
        sums[2 * k + 1] = vmull_high_n_s16(x, s);
    }
    sums
}

/// Whether any lane of the vectors of `x` from `from` is nonzero.
#[inline]
#[target_feature(enable = "neon")]
fn nonzero_from<const N: usize>(x: &[int16x8_t; N], from: usize) -> bool {
    let any = x
        .iter()
        .skip(from)
        .fold(vdupq_n_s16(0), |a, &v| vorrq_s16(a, v));
    from < N && vmaxvq_u16(vreinterpretq_u16_s16(any)) != 0
}

/// The row pass of `$kernel` over the `$n` input vectors `$x` of each of
/// the first `coded` rows of `$rows`, each output vector `k` of row `r`
/// handed to `$put(r, k, v)`. Inputs past a row's last nonzero vector are
/// zero: each expansion of `$kernel` sees them as literal zero vectors, so
/// their products fold away.
macro_rules! row_loop {
    ($t:ident, $geometry:expr, $rows:expr, $put:expr, |$x:ident: $n:literal| $kernel:expr) => {{
        let (coded, w, shift): (usize, usize, int32x4_t) = $geometry;
        let rows: &Half = $rows;
        let step = if w == 4 { 2 } else { 1 };
        for r in (0..coded).step_by(step) {
            let x: [int16x8_t; $n] = core::array::from_fn(|k| get16($t, rows, r * w + 8 * k));
            let zero = vdupq_n_s16(0);
            let sums: [int32x4_t; 2 * $n] = if nonzero_from(&x, 2) {
                let $x = x;
                $kernel
            } else if nonzero_from(&x, 1) {
                let $x: [int16x8_t; $n] = core::array::from_fn(|k| if k < 2 { x[k] } else { zero });
                $kernel
            } else {
                let $x: [int16x8_t; $n] = core::array::from_fn(|k| if k < 1 { x[k] } else { zero });
                $kernel
            };
            for k in 0..$n {
                $put(r, k, narrow(sums[2 * k], sums[2 * k + 1], shift));
            }
        }
    }};
}

/// The row pass of `pass` over the first `coded` rows of `rows`, each output
/// vector `k` of row `r` handed to `put(r, k, v)`.
#[inline]
#[target_feature(enable = "neon")]
pub(super) fn row_pass(
    t: Neon,
    pass: Pass,
    geometry: (usize, usize, int32x4_t),
    rows: &Half,
    mut put: impl FnMut(usize, usize, int16x8_t),
) {
    match (pass, geometry.1) {
        (Pass::Identity(s), 4 | 8) => row_loop!(t, geometry, rows, put, |x: 1| scaled(x, s)),
        (Pass::Identity(s), 16) => row_loop!(t, geometry, rows, put, |x: 2| scaled(x, s)),
        (Pass::Identity(s), _) => row_loop!(t, geometry, rows, put, |x: 4| scaled(x, s)),
        (Pass::Four(k), _) => row_loop!(t, geometry, rows, put, |x: 1| four!(t, k, x[0])),
        (Pass::Eight(k), _) => row_loop!(t, geometry, rows, put, |x: 1| eight!(t, k, x[0])),
        (Pass::Sixteen(k), _) => row_loop!(t, geometry, rows, put, |x: 2| sixteen!(t, k, x)),
        (Pass::Dct(8), _) => row_loop!(t, geometry, rows, put, |x: 1| dct8!(t, &DCT_TABLES, x)),
        (Pass::Dct(16), _) => {
            row_loop!(t, geometry, rows, put, |x: 2| dct16!(t, &DCT_TABLES, x));
        }
        (Pass::Dct(_), _) => row_loop!(t, geometry, rows, put, |x: 4| dct32!(t, tables(), x)),
    }
}
