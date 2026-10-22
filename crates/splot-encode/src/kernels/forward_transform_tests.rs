// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::unwrap_used)]

use proptest::prelude::*;
use splot_recon::{
    BitDepth, InverseTransform1dType, InverseTransform2d, InverseTransform2dDim,
    inverse_transform_2d, transform_shift,
};

use super::super::{EncoderKernels, Quantizer, ScalarKernels, SimdKernels};
use super::{EXTRA_BITS, ForwardTransform2d, KINDS, MATRICES, matrix_index};

/// The 25 AV2 `TX_SIZES_ALL` shapes as `(log2W, log2H)`.
const SHAPES: [(u32, u32); 25] = [
    (2, 2),
    (3, 3),
    (4, 4),
    (5, 5),
    (6, 6),
    (2, 3),
    (3, 2),
    (3, 4),
    (4, 3),
    (4, 5),
    (5, 4),
    (5, 6),
    (6, 5),
    (2, 4),
    (4, 2),
    (3, 5),
    (5, 3),
    (4, 6),
    (6, 4),
    (2, 5),
    (5, 2),
    (3, 6),
    (6, 3),
    (2, 6),
    (6, 2),
];

fn bit_depth(ten: bool) -> BitDepth {
    if ten { BitDepth::Ten } else { BitDepth::Eight }
}

/// A deterministic xorshift residual source in `-(2^bits - 1)..=2^bits - 1`.
struct Residuals(u64);

impl Residuals {
    fn next(&mut self, bits: u32) -> i32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        let span = (1u64 << (bits + 1)) - 1;
        (self.0 % span) as i32 - ((1 << bits) - 1)
    }

    fn block(&mut self, len: usize, bits: u32) -> Vec<i32> {
        (0..len).map(|_| self.next(bits)).collect()
    }
}

/// Runs the decoder inverse on `coeffs` and returns the largest distance from
/// the (pair-averaged) residual.
fn round_trip_error(
    shape: (u32, u32),
    row: InverseTransform2dDim,
    col: InverseTransform2dDim,
    bd: BitDepth,
    residual: &[i32],
    coeffs: &[i32],
) -> f64 {
    let (row_shift, col_shift) = transform_shift(shape.0, shape.1).unwrap();
    let params = InverseTransform2d {
        log2_width: shape.0,
        log2_height: shape.1,
        lossless: false,
        row_type: row,
        col_type: col,
        row_shift,
        col_shift,
        bit_depth: bd,
    };
    let mut out = vec![0i32; coeffs.len()];
    inverse_transform_2d(&params, coeffs, &mut out).unwrap();
    let (px, py) = (usize::from(shape.0 > 5), usize::from(shape.1 > 5));
    let (w, h) = (1usize << shape.0.min(5), 1usize << shape.1.min(5));
    let mut worst = 0.0f64;
    for y in 0..h {
        for x in 0..w {
            let mut sum = 0.0;
            for dy in 0..=py {
                for dx in 0..=px {
                    sum += f64::from(residual[((y << py) + dy) * (w << px) + (x << px) + dx]);
                }
            }
            let target = sum / f64::from(1u32 << (px + py));
            worst = worst.max((f64::from(out[y * w + x]) - target).abs());
        }
    }
    worst
}

/// The decoder inverse returns the residual within 1, or within 2 of the
/// pair-averaged residual for a 64-sample side (the 10-bit worst case; 8-bit
/// stays within 1).
#[test]
fn round_trip_through_the_decoder_inverse() {
    let mut rng = Residuals(0x9E37_79B9_7F4A_7C15);
    let mut worst_by_shape = [0.0f64; SHAPES.len()];
    for (slot, &shape) in worst_by_shape.iter_mut().zip(&SHAPES) {
        for row in KINDS {
            for col in KINDS {
                for ten in [false, true] {
                    let bd = bit_depth(ten);
                    let tx = ForwardTransform2d::new(shape.0, shape.1, row, col, bd).unwrap();
                    let residual = rng.block(tx.residual_len(), u32::from(bd.bits()));
                    let mut coeffs = vec![0i32; tx.coeff_width() * tx.coeff_height()];
                    SimdKernels
                        .forward_transform_2d(&tx, &residual, &mut coeffs)
                        .unwrap();
                    *slot = slot.max(round_trip_error(shape, row, col, bd, &residual, &coeffs));
                }
            }
        }
    }
    for (&shape, &worst) in SHAPES.iter().zip(&worst_by_shape) {
        let tolerance = if shape.0 > 5 || shape.1 > 5 { 2.0 } else { 1.0 };
        assert!(worst <= tolerance, "{shape:?} round-trip error {worst}");
    }
}

#[test]
fn energy_is_preserved() {
    let mut rng = Residuals(0x0123_4567_89AB_CDEF);
    for &shape in SHAPES.iter().filter(|s| s.0 <= 5 && s.1 <= 5) {
        let (row_shift, col_shift) = transform_shift(shape.0, shape.1).unwrap();
        let odd = shape.0.abs_diff(shape.1) % 2 == 1;
        let unit = f64::from(1u32 << 12)
            * f64::from(1u32 << (shape.0 + shape.1)).sqrt()
            * if odd { 2896.0 / 4096.0 } else { 1.0 }
            / f64::from(1u32 << (row_shift + col_shift));
        for row in KINDS {
            for col in KINDS {
                let tx =
                    ForwardTransform2d::new(shape.0, shape.1, row, col, BitDepth::Ten).unwrap();
                let residual = rng.block(tx.residual_len(), 9);
                let mut coeffs = vec![0i32; residual.len()];
                ScalarKernels
                    .forward_transform_2d(&tx, &residual, &mut coeffs)
                    .unwrap();
                let input: f64 = residual.iter().map(|&v| f64::from(v).powi(2)).sum();
                let output: f64 = coeffs.iter().map(|&c| (f64::from(c) * unit).powi(2)).sum();
                let ratio = output / input;
                assert!(
                    (0.95..=1.05).contains(&ratio),
                    "{shape:?} {row:?} {col:?} energy ratio {ratio}"
                );
            }
        }
    }
}

#[test]
fn flat_residual_is_dc_only() {
    let dct = InverseTransform2dDim::Kernel(InverseTransform1dType::Dct);
    let tx = ForwardTransform2d::new(2, 2, dct, dct, BitDepth::Eight).unwrap();
    let mut coeffs = [0i32; 16];
    ScalarKernels
        .forward_transform_2d(&tx, &[4; 16], &mut coeffs)
        .unwrap();
    assert_eq!(coeffs[0], 128, "the decoder maps a DC of 128 to a flat 4");
    assert!(coeffs[1..].iter().all(|&c| c == 0), "{coeffs:?}");
}

#[test]
fn column_norms_bound_the_intermediate_range() {
    for kind in KINDS {
        for log2n in 2..=5u32 {
            for rescaled in [false, true] {
                let n = 1usize << log2n;
                let m = &MATRICES[matrix_index(kind, log2n, rescaled)];
                let limit = 1.25
                    * f64::from(1u32 << (6 + EXTRA_BITS + log2n))
                    * if rescaled { 4096.0 / 2896.0 } else { 1.0 };
                for k in 0..n {
                    let l1: i64 = (0..n).map(|x| i64::from(m[x * n + k].abs())).sum();
                    assert!(
                        (l1 as f64) <= limit,
                        "{kind:?} n={n} column {k} L1 {l1} > {limit}"
                    );
                }
            }
        }
    }
}

#[test]
fn rejects_bad_shapes_and_buffers() {
    let dct = InverseTransform2dDim::Kernel(InverseTransform1dType::Dct);
    assert!(ForwardTransform2d::new(7, 2, dct, dct, BitDepth::Eight).is_err());
    assert!(ForwardTransform2d::new(2, 1, dct, dct, BitDepth::Eight).is_err());
    let tx = ForwardTransform2d::new(6, 6, dct, dct, BitDepth::Eight).unwrap();
    assert_eq!(tx.residual_len(), 4096);
    let mut coeffs = [0i32; 1024];
    assert!(
        SimdKernels
            .forward_transform_2d(&tx, &[0; 1024], &mut coeffs)
            .is_err()
    );
    assert!(
        ScalarKernels
            .forward_transform_2d(&tx, &[0; 4096], &mut coeffs[..16])
            .is_err()
    );
}

fn kind() -> impl Strategy<Value = InverseTransform2dDim> {
    (0..KINDS.len()).prop_map(|i| KINDS[i])
}

proptest! {
    #[test]
    fn simd_matches_scalar(shape in 0..SHAPES.len(), row in kind(), col in kind(), ten: bool, seed: u64) {
        let (log2w, log2h) = SHAPES[shape];
        let bd = bit_depth(ten);
        let tx = ForwardTransform2d::new(log2w, log2h, row, col, bd).unwrap();
        let residual = Residuals(seed | 1).block(tx.residual_len(), u32::from(bd.bits()));
        let mut scalar = vec![0i32; tx.coeff_width() * tx.coeff_height()];
        let mut simd = scalar.clone();
        ScalarKernels.forward_transform_2d(&tx, &residual, &mut scalar).unwrap();
        SimdKernels.forward_transform_2d(&tx, &residual, &mut simd).unwrap();
        prop_assert_eq!(scalar, simd);
    }
}

/// The RDOQ multiplier relies on a flat residual's coefficient energy being
/// `64 / dqDenom^2` times its sample energy at every size.
#[test]
fn coefficient_energy_gain_is_64_over_dq_denom_squared() {
    let dct = InverseTransform2dDim::Kernel(splot_recon::InverseTransform1dType::Dct);
    for log2_w in 2..=6u32 {
        for log2_h in (2..=6u32).filter(|h| h.abs_diff(log2_w) <= 2) {
            let forward =
                ForwardTransform2d::new(log2_w, log2_h, dct, dct, BitDepth::Eight).unwrap();
            let residual = vec![10; 1 << (log2_w + log2_h)];
            let mut coeffs = vec![0; forward.coeff_width() * forward.coeff_height()];
            ScalarKernels
                .forward_transform_2d(&forward, &residual, &mut coeffs)
                .unwrap();
            let energy = |v: &[i32]| v.iter().map(|&x| f64::from(x).powi(2)).sum::<f64>();
            let dq = f64::from(Quantizer::dq_denom_without_tcq(log2_w, log2_h));
            let gain = energy(&coeffs) / energy(&residual) * dq * dq / 64.0;
            assert!((gain - 1.0).abs() < 0.01, "{log2_w}x{log2_h}: {gain}");
        }
    }
}
