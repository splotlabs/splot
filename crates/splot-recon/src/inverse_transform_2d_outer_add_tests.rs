// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::unwrap_used)]

use super::*;
use crate::dequant_process::{DequantBlockParams, dequantize_block};
use crate::inverse_transform_2d_8bit::{FlatDequant, InverseTransformAdd8};

/// A small deterministic generator for coefficient and sample patterns.
struct Lcg(u64);

impl Lcg {
    fn next(&mut self, range: u32) -> i32 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        ((self.0 >> 33) % u64::from(range)) as i32
    }
}

/// The fused 8-bit transform + add must reconstruct exactly what the
/// separate dequantization, residual transform and `Clip1` add produce, for
/// every transform type and non-64 shape, with quantized and dequantized
/// inputs, sparse and dense coefficients, and a strided destination.
#[test]
fn fused_add_matches_residual_path() {
    let mut rng = Lcg(7);
    for (log2_w, log2_h, tx_type, use_ddt) in (0..4 * 4 * 16 * 2).map(|i| {
        (
            2 + i / 128,
            2 + i / 32 % 4,
            (i / 2 % 16) as usize,
            i % 2 == 1,
        )
    }) {
        let Ok(params) = InverseTransform2dOuter::resolve(
            tx_type,
            log2_w,
            log2_h,
            use_ddt,
            false,
            BitDepth::Eight,
            None,
        ) else {
            continue;
        };
        let (w, h) = (1usize << log2_w, 1usize << log2_h);
        for trial in 0..6 {
            let len = match trial {
                0 => 1,
                1 => 1 + rng.next(w as u32) as usize,
                _ => 1 + rng.next((w * h) as u32) as usize,
            };
            let spread = if trial % 2 == 0 { 9 } else { 400 };
            let mut quant: Vec<i32> = (0..len)
                .map(|_| {
                    if rng.next(3) == 0 {
                        rng.next(2 * spread) - spread as i32
                    } else {
                        0
                    }
                })
                .collect();
            if let Some(last) = quant.last_mut() {
                *last = 1 + rng.next(spread);
            }
            let dq = DequantBlockParams {
                dc_quant: 4 + rng.next(900) as u32,
                ac_quant: 4 + rng.next(900) as u32,
                tx_width: w,
                tx_height: h,
                dq_denom: 1 << rng.next(2),
                bit_depth: BitDepth::Eight,
                qm: None,
            };
            let mut dequant = vec![0; w * h];
            let coded = dequantize_block(&dq, &quant, &mut dequant).unwrap();
            let mut residual = vec![0; w * h];
            inverse_transform_2d_outer(&params, &dequant[..coded], &mut residual).unwrap();
            let stride = w + 3;
            let pred: Vec<u8> = (0..stride * h).map(|_| rng.next(256) as u8).collect();
            let mut expected = pred.clone();
            for (row, res) in residual.chunks_exact(w).enumerate() {
                for (out, &r) in expected[row * stride..].iter_mut().zip(res) {
                    *out = (i32::from(*out) + r).clamp(0, 255) as u8;
                }
            }
            let mut scratch = [0; 32 * 32];
            let table = InverseTransformAdd8::new(tx_type, log2_w, log2_h, use_ddt).unwrap();
            let flat = FlatDequant::new(dq.dc_quant, dq.ac_quant, dq.dq_denom);
            let mut dst = pred.clone();
            assert!(table.add(&quant, flat, &mut dst, stride, &mut scratch));
            assert_eq!(dst, expected, "{w}x{h} type {tx_type} ddt {use_ddt} table");
            if let Some(value) = table.flat_residual(&quant, flat) {
                assert!(tx_type == 0 && residual.iter().all(|&r| r == value));
            }
            let mut dst = pred.clone();
            assert!(table.add(&dequant[..coded], None, &mut dst, stride, &mut scratch));
            assert_eq!(
                dst, expected,
                "{w}x{h} type {tx_type} trial {trial} dequantized"
            );
        }
    }
}
