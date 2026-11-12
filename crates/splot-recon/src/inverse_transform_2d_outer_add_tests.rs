// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::unwrap_used)]

use super::*;
use crate::ReconSample;
use crate::dequant_process::{DequantBlockParams, dequantize_block};
use crate::inverse_transform_2d_8bit::{FlatDequant, InverseTransformAdd};

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

/// The fused transform + add into `u8` (8-bit) and `u16` (8- and 10-bit)
/// samples must reconstruct exactly what the separate dequantization,
/// residual transform and `Clip1` add produce, for every transform type and
/// non-64 shape, with quantized and dequantized inputs, sparse and dense
/// coefficients, extreme samples and a strided destination; `add_onto`
/// writes the prediction first and rejects one above the bit depth.
#[test]
fn fused_add_matches_residual_path() {
    let mut rng = Lcg(7);
    let depths = [
        (BitDepth::Eight, false),
        (BitDepth::Eight, true),
        (BitDepth::Ten, true),
    ];
    for ((log2_w, log2_h, tx_type, use_ddt), (bit_depth, wide)) in (0..4 * 4 * 16 * 2)
        .flat_map(|i| depths.map(|depth| (i, depth)))
        .map(|(i, depth)| {
            let shape = (
                2 + i / 128,
                2 + i / 32 % 4,
                (i / 2 % 16) as usize,
                i % 2 == 1,
            );
            (shape, depth)
        })
    {
        let max = i32::from(bit_depth.max_sample());
        let Ok(params) = InverseTransform2dOuter::resolve(
            tx_type, log2_w, log2_h, use_ddt, false, bit_depth, None,
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
            let extreme = [i32::MIN, i32::MAX, -65_536, 65_535, 1 << 20, -(1 << 23)];
            let mut quant: Vec<i32> = (0..len)
                .map(|_| {
                    if trial == 5 && rng.next(2) == 0 {
                        extreme[rng.next(extreme.len() as u32) as usize]
                    } else if rng.next(3) == 0 {
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
                bit_depth,
                qm: None,
            };
            let mut dequant = vec![0; w * h];
            let coded = dequantize_block(&dq, &quant, &mut dequant).unwrap();
            let mut residual = vec![0; w * h];
            inverse_transform_2d_outer(&params, &dequant[..coded], &mut residual).unwrap();
            let stride = w + 3;
            let pred: Vec<u16> = (0..stride * h)
                .map(|_| match rng.next(5) {
                    0 => 0,
                    1 => max as u16,
                    _ => rng.next(max as u32 + 1) as u16,
                })
                .collect();
            let mut expected = pred.clone();
            for (row, res) in residual.chunks_exact(w).enumerate() {
                for (out, &r) in expected[row * stride..].iter_mut().zip(res) {
                    *out = (i32::from(*out) + r).clamp(0, max) as u16;
                }
            }
            let table = InverseTransformAdd::new(tx_type, log2_w, log2_h, use_ddt).unwrap();
            let flat = FlatDequant::new(dq.dc_quant, dq.ac_quant, dq.dq_denom);
            if let Some(value) = table.flat_residual(&quant, flat, bit_depth) {
                assert!(tx_type == 0 && residual.iter().all(|&r| r == value));
            }
            let label = format!("{w}x{h} type {tx_type} ddt {use_ddt} trial {trial} {bit_depth:?}");
            let block: Vec<u16> = (0..w * h).map(|i| pred[i / w * stride + i % w]).collect();
            let inputs = [(&quant[..], flat), (&dequant[..coded], None)];
            if wide {
                check_fused(
                    table,
                    inputs,
                    (bit_depth, stride),
                    (&pred, &block, &expected),
                    &label,
                );
            } else {
                let narrow = |s: &[u16]| s.iter().map(|&v| v as u8).collect::<Vec<u8>>();
                let samples = (
                    &narrow(&pred)[..],
                    &narrow(&block)[..],
                    &narrow(&expected)[..],
                );
                check_fused(table, inputs, (bit_depth, stride), samples, &label);
            }
        }
    }
}

/// Checks `add` and `add_onto` of `table` on both `inputs` against `expected`.
fn check_fused<T: ReconSample + PartialEq + core::fmt::Debug>(
    table: InverseTransformAdd,
    inputs: [(&[i32], Option<FlatDequant>); 2],
    (bit_depth, stride): (BitDepth, usize),
    (pred, block, expected): (&[T], &[T], &[T]),
    label: &str,
) {
    let mut scratch = [i32::MAX; 32 * 32];
    for (coeffs, flat) in inputs {
        let mut dst = pred.to_vec(); // splot-copy-ok: test destination copy
        assert!(table.add(coeffs, (flat, bit_depth), &mut dst, stride, &mut scratch));
        assert_eq!(dst[..], *expected, "{label} add");
        let mut dst = vec![T::default(); pred.len()];
        let fused = (flat, bit_depth);
        assert!(table.add_onto(block, coeffs, fused, &mut dst, stride, &mut scratch));
        let w = block.len() / (pred.len() / stride);
        for (row, want) in expected.chunks(stride).enumerate() {
            assert_eq!(
                dst[row * stride..][..w],
                want[..w],
                "{label} add_onto row {row}"
            );
        }
    }
    if bit_depth == BitDepth::Ten {
        let mut high = block.to_vec(); // splot-copy-ok: test destination copy
        if let Some(sample) = high.last_mut() {
            *sample = T::try_from_u16(1024).unwrap();
        }
        let mut dst = pred.to_vec(); // splot-copy-ok: test destination copy
        let (coeffs, flat) = inputs[0];
        let fused = (flat, bit_depth);
        assert!(!table.add_onto(&high, coeffs, fused, &mut dst, stride, &mut scratch));
        assert_eq!(dst[..], *pred, "{label} rejected prediction leaves dst");
    }
}

/// The secondary-transform add, which for a block at least 16 wide stages
/// only the 8 columns the secondary transform writes, must match the
/// dequantized-prefix add for every kernel, type, transpose and input count,
/// into `u8` at 8 bits and `u16` at 10 bits, where x86 leaves the AVX2 path.
#[test]
fn secondary_add_matches_dequantized_prefix() {
    check_secondary_add::<u8>(BitDepth::Eight);
    check_secondary_add::<u16>(BitDepth::Ten);
}

fn check_secondary_add<T: ReconSample + PartialEq + core::fmt::Debug>(bit_depth: BitDepth) {
    use crate::coefficient_scan::TransformClass;
    use crate::{SecondaryInverseTransform, dequantize_with_secondary};
    let mut rng = Lcg(11);
    let max = u32::from(bit_depth.max_sample());
    for (log2_w, log2_h, tx_type) in [(4, 2, 0), (4, 3, 0), (4, 4, 0), (4, 5, 0), (5, 3, 0)]
        .into_iter()
        .chain([
            (5, 4, 0),
            (5, 5, 0),
            (4, 4, 5),
            (4, 3, 5),
            (5, 2, 0),
            (3, 3, 0),
            (3, 4, 5),
        ])
    {
        let (w, h) = (1usize << log2_w, 1usize << log2_h);
        let large = h >= 8;
        for trial in 0..24 {
            let secondary = SecondaryInverseTransform {
                w,
                h,
                n: 1 + rng.next(if large { 32 } else { 8 }) as usize,
                kernel: rng.next(if large { 11 } else { 14 }) as usize,
                sec_tx_type: 1 + rng.next(3) as usize,
                primary_scan_class: TransformClass::TwoD,
                transpose: trial % 2 == 1,
                bit_depth,
            };
            let len = 1 + rng.next((w * h) as u32) as usize;
            let spread = if trial % 3 == 0 { 2000 } else { 40 };
            let quant: Vec<i32> = (0..len)
                .map(|_| rng.next(2 * spread) - spread as i32)
                .collect();
            let dq = DequantBlockParams {
                dc_quant: 4 + rng.next(900) as u32,
                ac_quant: 4 + rng.next(900) as u32,
                tx_width: w,
                tx_height: h,
                dq_denom: 1 << rng.next(2),
                bit_depth,
                qm: None,
            };
            let table = InverseTransformAdd::new(tx_type, log2_w, log2_h, false).unwrap();
            let stride = w + 5;
            let pred: Vec<T> = (0..stride * h)
                .map(|_| T::try_from_u16(rng.next(max + 1) as u16).unwrap())
                .collect();
            let mut dequant = vec![0; w * h];
            let mut scratch = [i32::MAX; 32 * 32];
            let coded =
                dequantize_with_secondary(&dq, &quant, Some(&secondary), &mut dequant).unwrap();
            let mut expected = pred.clone();
            let flat = (None, bit_depth);
            assert!(table.add(&dequant[..coded], flat, &mut expected, stride, &mut scratch));
            let mut dst = pred.clone();
            let added = table
                .add_secondary(
                    ((dq.dc_quant, dq.ac_quant, dq.dq_denom), &secondary),
                    &quant,
                    &mut dequant,
                    (&mut dst, stride),
                    &mut scratch,
                )
                .unwrap();
            assert!(added);
            assert_eq!(
                dst, expected,
                "{bit_depth:?} {w}x{h} type {tx_type} trial {trial}"
            );
        }
    }
}
