// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! Forward cross-chroma transform: the transpose of the decoder's § 7.14.6
//! rotation of the U and V coefficients, and the U/V energy sums that choose
//! the rotation.

use std::simd::cmp::{SimdOrd as _, SimdPartialOrd as _};
use std::simd::num::SimdInt as _;
use std::simd::{Select as _, Simd};

use splot_recon::math::round2_signed_i32;

const LANES: usize = 8;
/// `CCTX_PREC_BITS` (§ 3).
const PREC_BITS: u32 = 8;
/// Inputs are clipped here, so a rotation of two inputs fits in `i32`.
const INPUT_BOUND: i32 = 1 << 22;

/// `[Σ u², Σ v², Σ u·v]` over the shorter of `u` and `v`.
#[cfg(test)]
pub(crate) fn stats_scalar(u: &[i32], v: &[i32]) -> [i64; 3] {
    u.iter().zip(v).fold([0; 3], |[uu, vv, uv], (&a, &b)| {
        let (a, b) = (i64::from(a), i64::from(b));
        [uu + a * a, vv + b * b, uv + a * b]
    })
}

/// [`stats_scalar`] in four `i64` lanes.
pub(crate) fn stats_simd(u: &[i32], v: &[i32]) -> [i64; 3] {
    let len = u.len().min(v.len());
    let (mut uu, mut vv, mut uv) = (Simd::<i64, 4>::splat(0), Simd::splat(0), Simd::splat(0));
    let (u_chunks, v_chunks) = (u[..len].chunks_exact(4), v[..len].chunks_exact(4));
    let (u_tail, v_tail) = (u_chunks.remainder(), v_chunks.remainder());
    for (a, b) in u_chunks.zip(v_chunks) {
        let a: Simd<i64, 4> = Simd::<i32, 4>::from_slice(a).cast();
        let b: Simd<i64, 4> = Simd::<i32, 4>::from_slice(b).cast();
        uu += a * a;
        vv += b * b;
        uv += a * b;
    }
    let [tu, tv, tuv] = u_tail
        .iter()
        .zip(v_tail)
        .fold([0; 3], |[x, y, z], (&a, &b)| {
            let (a, b) = (i64::from(a), i64::from(b));
            [x + a * a, y + b * b, z + a * b]
        });
    [
        uu.reduce_sum() + tu,
        vv.reduce_sum() + tv,
        uv.reduce_sum() + tuv,
    ]
}

/// One rotated pair: `(cos·u + sin·v, cos·v − sin·u) >> CCTX_PREC_BITS`.
fn rotate_pair(u: i32, v: i32, [cos, sin]: [i32; 2]) -> (i32, i32) {
    let (u, v) = (
        u.clamp(-INPUT_BOUND, INPUT_BOUND),
        v.clamp(-INPUT_BOUND, INPUT_BOUND),
    );
    (
        round2_signed_i32(cos * u + sin * v, PREC_BITS),
        round2_signed_i32(cos * v - sin * u, PREC_BITS),
    )
}

/// Rotates each `(u, v)` by [`rotate_pair`]: the coefficients whose
/// § 7.14.6 inverse with `[cos, sin]` returns `(u, v)`.
#[cfg(test)]
pub(crate) fn rotate_scalar(u: &mut [i32], v: &mut [i32], rotation: [i32; 2]) {
    for (cu, cv) in u.iter_mut().zip(v.iter_mut()) {
        (*cu, *cv) = rotate_pair(*cu, *cv, rotation);
    }
}

/// [`rotate_scalar`] in eight lanes.
pub(crate) fn rotate_simd(u: &mut [i32], v: &mut [i32], rotation: [i32; 2]) {
    let len = u.len().min(v.len());
    let (u, v) = (&mut u[..len], &mut v[..len]);
    let bound = Simd::<i32, LANES>::splat(INPUT_BOUND);
    let (cos, sin) = (Simd::splat(rotation[0]), Simd::splat(rotation[1]));
    let round = |sum: Simd<i32, LANES>| {
        let magnitude =
            (sum.abs() + Simd::splat(1 << (PREC_BITS - 1))) >> Simd::splat(PREC_BITS as i32);
        sum.simd_lt(Simd::splat(0)).select(-magnitude, magnitude)
    };
    let mut u_chunks = u.chunks_exact_mut(LANES);
    let mut v_chunks = v.chunks_exact_mut(LANES);
    for (cu, cv) in (&mut u_chunks).zip(&mut v_chunks) {
        let su = Simd::from_slice(cu).simd_clamp(-bound, bound);
        let sv = Simd::from_slice(cv).simd_clamp(-bound, bound);
        round(cos * su + sin * sv).copy_to_slice(cu);
        round(cos * sv - sin * su).copy_to_slice(cv);
    }
    for (cu, cv) in u_chunks
        .into_remainder()
        .iter_mut()
        .zip(v_chunks.into_remainder())
    {
        (*cu, *cv) = rotate_pair(*cu, *cv, rotation);
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;
    use splot_decode::encoder_syntax::CCTX_ROTATIONS;

    use super::*;

    proptest! {
        /// U and V of asymmetric ranges, past the input bound too, and odd lengths.
        #[test]
        fn simd_matches_scalar(
            u in prop::collection::vec(-6_000_000i32..900_000, 0..70),
            v in prop::collection::vec(-40_000i32..5_000_000, 0..70),
            rotation in 0..6usize,
        ) {
            prop_assert_eq!(stats_scalar(&u, &v), stats_simd(&u, &v));
            let len = u.len().min(v.len());
            let (mut su, mut sv) = (u[..len].to_vec(), v[..len].to_vec());
            let (mut vu, mut vv) = (su.clone(), sv.clone());
            rotate_scalar(&mut su, &mut sv, CCTX_ROTATIONS[rotation]);
            rotate_simd(&mut vu, &mut vv, CCTX_ROTATIONS[rotation]);
            prop_assert_eq!((su, sv), (vu, vv));
        }

        /// The decoder's inverse of a rotation returns `(u, v)` within the
        /// rounding of two steps and the 0.2% gain of `Cctx_Mtx`.
        #[test]
        fn inverse_undoes_rotation(
            u in -70_000i32..90_000,
            v in -90_000i32..60_000,
            rotation in 0..6usize,
        ) {
            let [c, s] = CCTX_ROTATIONS[rotation];
            let (mut p, mut q) = ([u], [v]);
            rotate_simd(&mut p, &mut q, [c, s]);
            let back_u = round2_signed_i32(p[0] * c - q[0] * s, PREC_BITS);
            let back_v = round2_signed_i32(p[0] * s + q[0] * c, PREC_BITS);
            let slack = 2 + (u.abs() + v.abs()) / 256;
            prop_assert!((back_u - u).abs() <= slack && (back_v - v).abs() <= slack);
        }
    }
}
