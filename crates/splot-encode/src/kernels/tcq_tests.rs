// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::unwrap_used)]

use proptest::prelude::*;
use splot_core::headers::frame::{
    CoreSeqView, EncoderStreamParams, EncoderToolFlags, FrameFilterParams, KeyFrameTxParams,
    UniformTiles, build_encoder_sequence_header, build_key_frame_header,
};
use splot_core::headers::sequence::{BitDepthIdc, ChromaFormatIdc, SuperblockSize};
use splot_decode::encoder_syntax::{
    CoeffLevels, IntraTileSyntax, TransformBlock, TxType, recon_params, tcq_index, tcq_next_state,
    tcq_quantizer,
};
use splot_recon::{BitDepth, PlaneId, dequant_coefficient};

use super::super::rdoq::{BIT, CoeffRates, LevelContext, base_levels, golomb_bits};
use super::super::{EncoderKernels, ScalarKernels, SimdKernels};
use super::{
    Branches, EVEN, FIRST, MAX_DIST, ODD, RdoqBlock, SECOND, STATES, ZERO, candidates, tcq,
};
use crate::rd::coeff_rate::CdfCoeffRates;
use crate::test_support::refs_for;

fn branches() -> impl Strategy<Value = ([u64; STATES], Branches, u64)> {
    let cost = prop_oneof![1 => Just(u64::MAX), 6 => 0u64..(1 << 60)];
    (
        proptest::array::uniform8(cost),
        proptest::array::uniform3(proptest::array::uniform8(prop_oneof![
            1 => Just(MAX_DIST),
            6 => 0u64..MAX_DIST,
        ])),
        proptest::array::uniform3(proptest::array::uniform8(0u32..(1 << 24))),
        0u64..(1 << 30),
    )
        .prop_map(|(prev, dist, rate, lambda)| (prev, Branches { dist, rate }, lambda))
}

proptest! {
    /// The SIMD survivor step equals its scalar twin over asymmetric inputs.
    #[test]
    fn simd_step_matches_scalar((prev, b, lambda) in branches()) {
        prop_assert_eq!(
            SimdKernels.tcq_step(&prev, &b, lambda),
            ScalarKernels.tcq_step(&prev, &b, lambda)
        );
    }
}

/// Publication check: the survivor lanes, the per-state quantizer, and the
/// reconstruction index the trellis assumes are the decoder's.
#[test]
fn trellis_assumptions_are_the_decoder_rules() {
    for state in 0..STATES {
        for parity in 0..2 {
            let lane = parity * STATES + state;
            let next = tcq_next_state(state, parity as u32);
            assert!(FIRST[next] == lane || SECOND[next] == lane, "{state} {parity}");
        }
        for quant in 0..300 {
            let expected = if quant == 0 {
                0
            } else {
                2 * quant - tcq_quantizer(state) as u32
            };
            assert_eq!(tcq_index(state, quant).unwrap(), expected);
        }
    }
    let lanes: std::collections::BTreeSet<usize> = FIRST.iter().chain(&SECOND).copied().collect();
    assert_eq!(lanes.len(), 2 * STATES);
}

/// A key frame with TCQ: the rows the rates read, and its blocks.
fn tcq_syntax(bit_depth: BitDepthIdc) -> IntraTileSyntax {
    let seq = build_encoder_sequence_header(&EncoderStreamParams {
        width: 64,
        height: 64,
        bit_depth,
        chroma: ChromaFormatIdc::Yuv420,
        sb_size: SuperblockSize::Block64x64,
        tools: EncoderToolFlags {
            tcq: true,
            ..EncoderToolFlags::default()
        },
    })
    .unwrap();
    let core = build_key_frame_header(
        &CoreSeqView::from_sequence(&seq).unwrap(),
        90,
        0,
        &FrameFilterParams::default(),
        KeyFrameTxParams {
            allow_tcq: true,
            ..KeyFrameTxParams::default()
        },
        UniformTiles::default(),
    )
    .unwrap();
    IntraTileSyntax::new(&seq, &core, &refs_for(&seq), 0).unwrap()
}

const fn tx(plane: usize, tx_size: usize, tx_type: TxType) -> TransformBlock {
    TransformBlock {
        tx_type,
        plane,
        x: 0,
        y: 0,
        tx_size,
        fills_block: true,
    }
}

/// `dqDenom` takes `+1` for TCQ levels only: luma blocks of class 2D.
#[test]
fn tcq_doubles_dq_denom_for_luma_2d_only() {
    let syntax = tcq_syntax(BitDepthIdc::Eight);
    for (tx_size, log2) in [(0, 2), (1, 3), (2, 4), (3, 5), (4, 6)] {
        let mut blocks = vec![
            tx(0, tx_size, TxType::DctDct),
            tx(1, tx_size, TxType::DctDct),
            tx(2, tx_size, TxType::DctDct),
        ];
        if log2 <= 4 {
            blocks.extend([tx(0, tx_size, TxType::Idtx), tx(0, tx_size, TxType::VDct)]);
        }
        for block in blocks {
            let plane = [PlaneId::Y, PlaneId::U, PlaneId::V][block.plane];
            let doubled = block.plane == 0 && block.tx_type != TxType::VDct;
            assert_eq!(syntax.uses_tcq(&block), doubled, "{block:?}");
            let params = |tcq| {
                recon_params(90, plane, (log2, log2), (block.tx_type, tcq), BitDepth::Eight)
                    .unwrap()
                    .0
                    .dq_denom
            };
            let coded = params(syntax.uses_tcq(&block));
            assert_eq!(coded, params(false) << u32::from(doubled), "{block:?}");
        }
    }
}

/// The cost a path of coded `levels` has under `rates`, walked as the
/// writer codes it: distortion of the TCQ reconstruction plus the rates.
fn path_cost(
    rates: &mut impl CoeffRates,
    b: &RdoqBlock,
    coeffs: &[i32],
    levels: &[i32],
    lambda: u64,
) -> u64 {
    let mut grid = CoeffLevels::new(1 << b.log2_width, 1 << b.log2_height, b.tx_type).unwrap();
    let scan = grid.scan();
    let eob = scan
        .iter()
        .rposition(|&pos| levels[usize::from(pos)] != 0)
        .map_or(0, |last| last + 1);
    let mut cost = 0u64;
    for &pos in &scan[eob..] {
        cost += u64::from(coeffs[usize::from(pos)].unsigned_abs()).pow(2) << 16;
    }
    let mut rate = u64::from(rates.eob(b, eob));
    let (mut state, mut hr) = (0, 0u32);
    grid.set_tcq_state(0);
    for c in (0..eob).rev() {
        let pos = usize::from(scan[c]);
        let (row, col) = (pos >> b.log2_width, pos & ((1 << b.log2_width) - 1));
        let is_lf = grid.is_low_frequency(row, col, 0);
        let quant = levels[pos].unsigned_abs();
        let index = tcq_index(state, quant).unwrap();
        let q2 = if pos == 0 { b.dc_q2 } else { b.ac_q2 };
        let dq = dequant_coefficient(levels[pos].signum() * index as i32, q2, b.dq_denom, b.bit_depth);
        cost += (i64::from(coeffs[pos]) - i64::from(dq)).unsigned_abs().pow(2) << 16;
        let (level, remainder) = grid.split(c, quant, is_lf, 0);
        let ctx = LevelContext {
            levels: &grid,
            c,
            row,
            col,
            is_lf,
            is_last: c + 1 == eob,
            base_class: 0,
            br_class: 0,
        };
        let base = level.min(base_levels(is_lf) + 1);
        rate += u64::from(rates.base(&ctx)[base as usize]);
        if base > base_levels(is_lf) {
            rate += u64::from(rates.br(&ctx)[(level - base) as usize]);
        }
        if quant > 0 {
            rate += u64::from(if grid.sign_has_cdf(row, col, 0) {
                rates.sign(&ctx)
            } else {
                BIT
            });
        }
        if let Some(x) = remainder {
            rate += u64::from(golomb_bits(x, hr));
            hr = (x + hr) >> 1;
        }
        grid.set(row, col, level).unwrap();
        state = tcq_next_state(state, level);
        grid.set_tcq_state(state);
    }
    cost + lambda * rate
}

/// Visits every path from scan index `c` down in `state` over the candidates `cand`.
fn walk(
    c: usize,
    state: usize,
    cand: &[super::Candidates],
    scan: &[u16],
    levels: &mut [i32],
    coeffs: &[i32],
    visit: &mut dyn FnMut(&[i32]),
) {
    let pos = usize::from(scan[c]);
    let slots = cand[c][tcq_quantizer(state)];
    for (quant, dist) in slots {
        let last = levels.iter().all(|&l| l == 0);
        if dist >= MAX_DIST || (last && (quant == 0 || dist >= slots[ZERO].1)) {
            continue;
        }
        levels[pos] = if coeffs[pos] < 0 { -(quant as i32) } else { quant as i32 };
        if c == 0 {
            visit(levels);
        } else {
            walk(c - 1, tcq_next_state(state, quant), cand, scan, levels, coeffs, visit);
        }
        levels[pos] = 0;
    }
}

/// The cheapest path over the trellis candidates, by enumeration: the last
/// level is one whose distortion beats zero.
fn brute_force(
    rates: &mut impl CoeffRates,
    b: &RdoqBlock,
    coeffs: &[i32],
    end: usize,
    lambda: u64,
) -> u64 {
    let scan = CoeffLevels::new(1 << b.log2_width, 1 << b.log2_height, b.tx_type)
        .unwrap()
        .scan();
    let cand: Vec<_> = (0..end)
        .map(|c| candidates(b, usize::from(scan[c]), coeffs[usize::from(scan[c])]))
        .collect();
    let mut best = path_cost(rates, b, coeffs, &vec![0; coeffs.len()], lambda);
    let mut levels = vec![0; coeffs.len()];
    for last in 0..end {
        walk(last, 0, &cand, scan, &mut levels, coeffs, &mut |levels| {
            best = best.min(path_cost(rates, b, coeffs, levels, lambda));
        });
    }
    best
}

/// The trellis finds the cheapest path over its candidates, with every rate
/// in the contexts of its own path, on blocks small enough to enumerate.
#[test]
fn trellis_is_optimal_on_small_blocks() {
    let mut seed = 0x7c9_u64;
    let mut next = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    for case in 0..96 {
        let (bit_depth, idc) = if case % 2 == 0 {
            (BitDepth::Eight, BitDepthIdc::Eight)
        } else {
            (BitDepth::Ten, BitDepthIdc::Ten)
        };
        let mut syntax = tcq_syntax(idc);
        let (tx_size, log2) = [(0, 2), (1, 3)][case % 3 / 2];
        let block = tx(0, tx_size, TxType::DctDct);
        let qindex = 20 + (next() % 200) as u32;
        let (params, _) =
            recon_params(qindex, PlaneId::Y, (log2, log2), (TxType::DctDct, true), bit_depth)
                .unwrap();
        let b = RdoqBlock {
            dc_q2: params.dc_quant,
            ac_q2: params.ac_quant,
            dq_denom: params.dq_denom,
            bit_depth,
            log2_width: log2,
            log2_height: log2,
            plane: 0,
            tx_type: TxType::DctDct,
        };
        let scan = CoeffLevels::new(1 << log2, 1 << log2, TxType::DctDct)
            .unwrap()
            .scan();
        let mut coeffs = vec![0i32; 1 << (2 * log2)];
        let span = 3 + case % 4;
        for &pos in &scan[..span] {
            let magnitude = (next() % u64::from(params.ac_quant * 6 / 8)) as i32;
            coeffs[usize::from(pos)] = if next() % 3 == 0 { -magnitude } else { magnitude };
        }
        let lambda = 1 + next() % 4000;
        let mut rates = CdfCoeffRates::new(&mut syntax, &block);
        let mut levels = vec![0; coeffs.len()];
        let eob = tcq(&SimdKernels, &b, &coeffs, &mut rates, lambda, &mut levels).unwrap();
        let end = scan[..span]
            .iter()
            .rposition(|&pos| super::reaches_half_step(&b, usize::from(pos), coeffs[usize::from(pos)]))
            .map_or(0, |last| last + 1);
        assert_eq!(
            eob,
            scan.iter()
                .rposition(|&pos| levels[usize::from(pos)] != 0)
                .map_or(0, |l| l + 1)
        );
        let found = path_cost(&mut rates, &b, &coeffs, &levels, lambda);
        let best = brute_force(&mut rates, &b, &coeffs, end, lambda);
        assert_eq!(found, best, "case {case}: {coeffs:?} {levels:?}");
    }
}

/// Slots: zero, then one even and one odd level of each quantizer around the
/// coefficient, each with the exact distortion.
#[test]
fn candidates_bracket_the_coefficient() {
    let b = RdoqBlock {
        dc_q2: 400,
        ac_q2: 320,
        dq_denom: 2,
        bit_depth: BitDepth::Eight,
        log2_width: 2,
        log2_height: 2,
        plane: 0,
        tx_type: TxType::DctDct,
    };
    for coeff in -2000..2000 {
        let cand = candidates(&b, 1, coeff);
        for (z, slots) in cand.iter().enumerate() {
            assert_eq!(slots[ZERO], (0, u64::from(coeff.unsigned_abs()).pow(2)));
            assert_eq!(slots[ODD].0 & 1, 1);
            assert!(slots[EVEN].1 == MAX_DIST || slots[EVEN].0.is_multiple_of(2) && slots[EVEN].0 > 0);
            let state = (0..STATES).find(|&s| tcq_quantizer(s) == z).unwrap();
            let best = (0..200u32)
                .map(|q| {
                    let dq = dequant_coefficient(
                        if coeff < 0 { -1 } else { 1 } * tcq_index(state, q).unwrap() as i32,
                        320,
                        2,
                        BitDepth::Eight,
                    );
                    (i64::from(coeff) - i64::from(dq)).unsigned_abs().pow(2)
                })
                .min()
                .unwrap();
            assert_eq!(slots.iter().map(|&(_, d)| d).min().unwrap(), best, "{coeff} {z}");
        }
    }
}
