// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::unwrap_used)]

use crate::test_support::refs_for;
use proptest::prelude::*;
use splot_core::headers::frame::{
    CoreSeqView, EncoderStreamParams, EncoderToolFlags, FrameFilterParams, KeyFrameTxParams,
    build_encoder_sequence_header, build_key_frame_header,
};
use splot_core::headers::sequence::{BitDepthIdc, ChromaFormatIdc, SuperblockSize};
use splot_decode::encoder_syntax::{
    IntraTileSyntax, TransformBlock, dct_recon_params, is_low_frequency,
};
use splot_recon::{BitDepth, PlaneId, dequant_coefficient, max_quantizer_index, quantizer_value};

use super::super::{EncoderKernels, ForwardTransform2d, Quantizer, ScalarKernels, SimdKernels};
use super::{
    ApproxCoeffRates, BASE_SYMBOLS, BR_SYMBOLS, CoeffRates, LevelContext, RdoqBlock, Walk,
    base_levels, rdoq,
};

fn block() -> impl Strategy<Value = RdoqBlock> {
    (
        any::<bool>(),
        0u32..=1000,
        -64i32..=64,
        2u32..=5,
        2u32..=5,
        0usize..3,
    )
        .prop_map(|(ten, qindex, delta, log2_width, log2_height, plane)| {
            let bit_depth = if ten { BitDepth::Ten } else { BitDepth::Eight };
            let qindex = qindex % (max_quantizer_index(bit_depth) + 1);
            RdoqBlock {
                dc_q2: quantizer_value(qindex, delta, bit_depth),
                ac_q2: quantizer_value(qindex, 0, bit_depth),
                dq_denom: Quantizer::dq_denom_without_tcq(log2_width, log2_height),
                bit_depth,
                log2_width,
                log2_height,
                plane,
            }
        })
}

fn coefficient() -> impl Strategy<Value = i32> {
    prop_oneof![
        6 => -64i32..=64,
        6 => -4096i32..=4096,
        1 => -(1i32 << 20)..=(1 << 20),
    ]
}

/// A block with its coefficients and a `lambda_q8`.
fn case() -> impl Strategy<Value = (RdoqBlock, Vec<i32>, u64)> {
    block().prop_flat_map(|b| {
        let n = 1usize << (b.log2_width + b.log2_height);
        (
            Just(b),
            proptest::collection::vec(coefficient(), n),
            prop_oneof![Just(0u64), 0u64..1 << 12, 0u64..1 << 40],
        )
    })
}

fn run(
    k: &dyn EncoderKernels,
    b: &RdoqBlock,
    coeffs: &[i32],
    lambda: u64,
) -> (Vec<i32>, Vec<i32>, usize) {
    let mut levels = vec![0; coeffs.len()];
    let mut dequant = vec![0; coeffs.len()];
    let eob = rdoq(
        k,
        b,
        coeffs,
        &mut ApproxCoeffRates,
        lambda,
        &mut levels,
        &mut dequant,
    )
    .unwrap();
    (levels, dequant, eob)
}

fn quantize(b: &RdoqBlock, rounding_q8: u32, coeffs: &[i32]) -> (Vec<i32>, Vec<i32>) {
    let q = Quantizer::new(b.dc_q2, b.ac_q2, b.dq_denom, b.bit_depth, rounding_q8).unwrap();
    let mut levels = vec![0; coeffs.len()];
    let mut dequant = vec![0; coeffs.len()];
    SimdKernels
        .quantize(&q, coeffs, &mut levels, &mut dequant)
        .unwrap();
    (levels, dequant)
}

/// The rate of `levels` in the writer's coding order under `rates`.
fn block_rate(b: &RdoqBlock, levels: &[i32], rates: &mut impl CoeffRates) -> u64 {
    let mut walk = Walk::new(b).unwrap();
    let scan = walk.scan;
    let eob = scan
        .iter()
        .rposition(|&pos| levels[usize::from(pos)] != 0)
        .map_or(0, |last| last + 1);
    let mut rate = u64::from(rates.eob(b, eob));
    for c in (0..eob).rev() {
        let level = levels[usize::from(scan[c])].unsigned_abs();
        let costs = walk.costs(rates, c, c + 1 == eob, level, None);
        rate += u64::from(costs.rate(level, walk.hr_level_avg).0);
        walk.commit(c, level, &costs).unwrap();
    }
    rate
}

/// Checks that every context [`rdoq`] passes matches the scan position.
struct CheckedRates {
    block: RdoqBlock,
    scan: &'static [u16],
}

impl CheckedRates {
    fn check(&self, ctx: &LevelContext<'_>) {
        let width = 1 << self.block.log2_width;
        assert_eq!(usize::from(self.scan[ctx.c]), ctx.row * width + ctx.col);
        assert_eq!(
            ctx.is_lf,
            is_low_frequency(ctx.row, ctx.col, self.block.plane)
        );
        assert!(ctx.base_class <= 4 && ctx.br_class <= 6);
    }
}

impl CoeffRates for CheckedRates {
    fn eob(&mut self, block: &RdoqBlock, eob: usize) -> u32 {
        ApproxCoeffRates.eob(block, eob)
    }

    fn base(&mut self, ctx: &LevelContext<'_>) -> [u32; BASE_SYMBOLS] {
        self.check(ctx);
        ApproxCoeffRates.base(ctx)
    }

    fn br(&mut self, ctx: &LevelContext<'_>) -> [u32; BR_SYMBOLS] {
        self.check(ctx);
        ApproxCoeffRates.br(ctx)
    }

    fn dc_sign(&mut self) -> u32 {
        ApproxCoeffRates.dc_sign()
    }
}

/// Reads the live facade rows a CDF-backed model uses, with the approximate costs.
struct FacadeRates {
    syntax: IntraTileSyntax,
    tx: TransformBlock,
}

impl CoeffRates for FacadeRates {
    fn eob(&mut self, block: &RdoqBlock, eob: usize) -> u32 {
        assert!(self.syntax.txb_skip_cdf(&self.tx, false).unwrap().len() >= 2);
        assert!(self.syntax.eob_pt_cdf(0, self.tx.tx_size).unwrap().len() >= 5);
        ApproxCoeffRates.eob(block, eob)
    }

    fn base(&mut self, ctx: &LevelContext<'_>) -> [u32; BASE_SYMBOLS] {
        let tx_size = self.tx.tx_size;
        let symbols = base_levels(ctx.is_lf) as usize + 2 - usize::from(ctx.is_last);
        let row = if ctx.is_last {
            let h = 4 << tx_size;
            self.syntax
                .coeff_base_eob_cdf(0, tx_size, ctx.c, (tx_size as u32 + 2, h), ctx.is_lf)
        } else {
            self.syntax
                .coeff_base_cdf(ctx.levels, 0, tx_size, (ctx.row, ctx.col, ctx.c), ctx.is_lf)
        };
        assert!(row.unwrap().len() >= symbols);
        ApproxCoeffRates.base(ctx)
    }

    fn br(&mut self, ctx: &LevelContext<'_>) -> [u32; BR_SYMBOLS] {
        let row = self
            .syntax
            .coeff_br_cdf(ctx.levels, 0, (ctx.row, ctx.col), ctx.is_lf);
        assert!(row.unwrap().len() >= BR_SYMBOLS);
        ApproxCoeffRates.br(ctx)
    }

    fn dc_sign(&mut self) -> u32 {
        assert!(self.syntax.dc_sign_cdf(&self.tx).unwrap().len() >= 2);
        ApproxCoeffRates.dc_sign()
    }
}

/// The trait needs no new facade API: a model on today's rows plugs in.
#[test]
fn facade_rows_back_the_rate_model() {
    let seq = build_encoder_sequence_header(&EncoderStreamParams {
        width: 64,
        height: 64,
        bit_depth: BitDepthIdc::Eight,
        chroma: ChromaFormatIdc::Yuv420,
        sb_size: SuperblockSize::Block64x64,
        tools: EncoderToolFlags::default(),
    })
    .unwrap();
    let core = build_key_frame_header(
        &CoreSeqView::from_sequence(&seq).unwrap(),
        90,
        0,
        &FrameFilterParams::default(),
        KeyFrameTxParams::default(),
    )
    .unwrap();
    let mut rng = Residuals(7);
    for tx_size in 0..4 {
        let log2 = tx_size as u32 + 2;
        let tx = TransformBlock {
            tx_type: splot_decode::encoder_syntax::TxType::DctDct,
            plane: 0,
            x: 0,
            y: 0,
            tx_size,
            fills_block: true,
        };
        let mut rates = FacadeRates {
            syntax: IntraTileSyntax::new(&seq, &core, &refs_for(&seq)).unwrap(),
            tx,
        };
        let b = RdoqBlock {
            dc_q2: 160,
            ac_q2: 160,
            dq_denom: Quantizer::dq_denom_without_tcq(log2, log2),
            bit_depth: BitDepth::Eight,
            log2_width: log2,
            log2_height: log2,
            plane: 0,
        };
        let coeffs: Vec<i32> = rng
            .block(1 << log2, 1 << log2)
            .iter()
            .map(|v| v * 40)
            .collect();
        let (mut levels, mut dequant) = (vec![0; coeffs.len()], vec![0; coeffs.len()]);
        let eob = rdoq(
            &SimdKernels,
            &b,
            &coeffs,
            &mut rates,
            2000,
            &mut levels,
            &mut dequant,
        );
        assert!(eob.unwrap() > 0);
    }
}

proptest! {
    #[test]
    fn simd_matches_scalar((b, coeffs, lambda) in case()) {
        prop_assert_eq!(run(&SimdKernels, &b, &coeffs, lambda), run(&ScalarKernels, &b, &coeffs, lambda));
    }

    #[test]
    fn levels_are_writable_and_exactly_reconstructed((b, coeffs, lambda) in case()) {
        let (levels, dequant, eob) = run(&SimdKernels, &b, &coeffs, lambda);
        let (nearest, _) = quantize(&b, 128, &coeffs);
        let scan = Walk::new(&b).unwrap().scan;
        let last = scan.iter().rposition(|&pos| levels[usize::from(pos)] != 0).map_or(0, |l| l + 1);
        prop_assert_eq!(eob, last);
        for (i, ((&level, &dq), &l)) in levels.iter().zip(&dequant).zip(&nearest).enumerate() {
            let q2 = if i == 0 { b.dc_q2 } else { b.ac_q2 };
            prop_assert!(level.unsigned_abs() < 1 << 20);
            prop_assert!(level == 0 || level.signum() == l.signum());
            prop_assert!(level.unsigned_abs() <= l.unsigned_abs());
            prop_assert!(l.unsigned_abs() - level.unsigned_abs() <= 1 || level == 0);
            prop_assert_eq!(dq, dequant_coefficient(level, q2, b.dq_denom, b.bit_depth));
        }
        let scan_len = scan.len();
        let mut checked = CheckedRates { block: b, scan };
        let mut levels2 = vec![0; scan_len];
        let mut dequant2 = vec![0; scan_len];
        rdoq(&SimdKernels, &b, &coeffs, &mut checked, lambda, &mut levels2, &mut dequant2).unwrap();
    }

    #[test]
    fn lambda_zero_is_round_to_nearest((b, coeffs, _) in case()) {
        let (levels, dequant, _) = run(&ScalarKernels, &b, &coeffs, 0);
        prop_assert_eq!((levels, dequant), quantize(&b, 128, &coeffs));
    }
}

#[test]
fn huge_lambda_codes_nothing() {
    let b = RdoqBlock {
        dc_q2: 64,
        ac_q2: 64,
        dq_denom: 1,
        bit_depth: BitDepth::Eight,
        log2_width: 2,
        log2_height: 2,
        plane: 0,
    };
    let coeffs = [40, -24, 16, 9, 0, 3, -2, 1, 0, 0, 0, 0, 0, 0, 0, 1];
    assert!(run(&SimdKernels, &b, &coeffs, 0).2 > 0);
    assert_eq!(
        run(&SimdKernels, &b, &coeffs, u64::MAX >> 8),
        (vec![0; 16], vec![0; 16], 0)
    );
}

#[test]
fn rejects_invalid_blocks() {
    let mut b = RdoqBlock {
        dc_q2: 64,
        ac_q2: 64,
        dq_denom: 1,
        bit_depth: BitDepth::Eight,
        log2_width: 6,
        log2_height: 2,
        plane: 0,
    };
    let mut out = [0; 16];
    let mut dq = [0; 16];
    assert!(
        rdoq(
            &SimdKernels,
            &b,
            &[0; 16],
            &mut ApproxCoeffRates,
            1,
            &mut out,
            &mut dq
        )
        .is_err()
    );
    b.log2_width = 2;
    assert!(
        rdoq(
            &SimdKernels,
            &b,
            &[0; 15],
            &mut ApproxCoeffRates,
            1,
            &mut out,
            &mut dq
        )
        .is_err()
    );
    assert!(
        rdoq(
            &SimdKernels,
            &b,
            &[0; 16],
            &mut ApproxCoeffRates,
            1,
            &mut out[..8],
            &mut dq
        )
        .is_err()
    );
}

/// A smooth gradient plus noise, like an intra prediction residual.
struct Residuals(u64);

impl Residuals {
    fn next(&mut self, range: i32) -> i32 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        ((self.0 >> 33) % (2 * range as u64 + 1)) as i32 - range
    }

    fn block(&mut self, width: usize, height: usize) -> Vec<i32> {
        let (gx, gy, offset, noise) = (
            self.next(6),
            self.next(6),
            self.next(20),
            self.next(8).abs() + 1,
        );
        (0..height)
            .flat_map(|y| (0..width).map(move |x| (x as i32, y as i32)))
            .map(|(x, y)| {
                gx * (2 * x - width as i32) / 4 + gy * (2 * y - height as i32) / 4 + offset
            })
            .map(|v| v + self.next(noise))
            .collect()
    }
}

/// Summed `(D, R, J)` of one method.
#[derive(Default)]
struct Totals {
    distortion: u128,
    rate: u64,
    cost: u128,
}

impl Totals {
    fn add(
        &mut self,
        b: &RdoqBlock,
        coeffs: &[i32],
        (levels, dequant): (&[i32], &[i32]),
        lambda: u64,
    ) {
        let d: u128 = coeffs
            .iter()
            .zip(dequant)
            .map(|(&c, &dq)| u128::from((i64::from(c) - i64::from(dq)).unsigned_abs().pow(2)))
            .sum();
        let r = block_rate(b, levels, &mut ApproxCoeffRates);
        self.distortion += d;
        self.rate += r;
        self.cost += (d << 16) + u128::from(lambda) * u128::from(r);
    }
}

/// D + lambda * R under [`ApproxCoeffRates`] against the encoder's deadzone
/// (96/256) and round-to-nearest quantizers. `lambda` is `0.156 * step^2`, with
/// `step` the AC reconstruction step in transform units. Prints the totals.
#[test]
fn rdoq_lowers_rd_cost_on_natural_residuals() {
    let bd = BitDepth::Eight;
    let mut rng = Residuals(0x9E37_79B9_7F4A_7C15);
    for qindex in [40, 100, 160, 220] {
        let mut totals: [Totals; 3] = Default::default();
        for (log2_w, log2_h) in [(2, 2), (3, 3), (4, 4), (5, 5), (3, 4), (4, 2)] {
            let (params, tx) = dct_recon_params(qindex, PlaneId::Y, log2_w, log2_h, bd).unwrap();
            let forward =
                ForwardTransform2d::new(log2_w, log2_h, tx.row_type, tx.col_type, bd).unwrap();
            let b = RdoqBlock {
                dc_q2: params.dc_quant,
                ac_q2: params.ac_quant,
                dq_denom: params.dq_denom,
                bit_depth: bd,
                log2_width: log2_w,
                log2_height: log2_h,
                plane: 0,
            };
            let step_q3 = u64::from(b.ac_q2 / b.dq_denom);
            let lambda = step_q3 * step_q3 * 5 / 8;
            for _ in 0..40 {
                let residual = rng.block(1 << log2_w, 1 << log2_h);
                let mut coeffs = vec![0; residual.len()];
                SimdKernels
                    .forward_transform_2d(&forward, &residual, &mut coeffs)
                    .unwrap();
                let (dz, dz_dq) = quantize(&b, 96, &coeffs);
                let (nearest, nearest_dq) = quantize(&b, 128, &coeffs);
                let (rd, rd_dq, _) = run(&SimdKernels, &b, &coeffs, lambda);
                totals[0].add(&b, &coeffs, (&dz, &dz_dq), lambda);
                totals[1].add(&b, &coeffs, (&nearest, &nearest_dq), lambda);
                totals[2].add(&b, &coeffs, (&rd, &rd_dq), lambda);
            }
        }
        let [dz, nearest, rd] = &totals;
        eprintln!(
            "qindex {qindex}: J rdoq/deadzone {:.4} rdoq/nearest {:.4}; D {} {} {}; R bits {} {} {}",
            rd.cost as f64 / dz.cost as f64,
            rd.cost as f64 / nearest.cost as f64,
            dz.distortion,
            nearest.distortion,
            rd.distortion,
            dz.rate / 256,
            nearest.rate / 256,
            rd.rate / 256,
        );
        assert!(
            rd.cost < dz.cost && rd.cost < nearest.cost,
            "qindex {qindex}"
        );
    }
}
