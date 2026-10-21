// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use proptest::prelude::*;
use splot_core::headers::frame::{
    CoreSeqView, EncoderStreamParams, EncoderToolFlags, FrameFilterParams, FrameHeaderCore,
    build_encoder_sequence_header, build_key_frame_header,
};
use splot_core::headers::sequence::{BitDepthIdc, ChromaFormatIdc, SequenceHeader, SuperblockSize};
use splot_core::symbol::Symbol;
use splot_core::symbol_encoder::SymbolEncoder;
use splot_decode::encoder_syntax::{
    IntraTileSyntax, PartitionCdf, TransformBlock, dct_recon_params,
};
use splot_recon::{
    BitDepth, CurrentFrameWorkspace, DecodedFrameInfo, OutputIndex, PixelFormat, PlaneId,
    PlaneRect, PlaneSize, reconstruct_transform_block_residual,
};

use super::checkpoint::Checkpoint;
use super::coeff_rate::{RateSink, block_rate};
use super::*;
use crate::tile_writer::coeffs::adjusted_log2;
use crate::tile_writer::{
    BlockDecision, KeyFrameTileWriter, PARTITIONS, Partition, PartitionTree, ResidualCoder,
    SuperblockDecision, TransformDecision, TxType,
};

/// A deterministic generator (xorshift64*).
struct Rng(u64);

impl Rng {
    fn below(&mut self, n: u64) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D) % n
    }
}

fn bits_f64(bits: Bits) -> f64 {
    bits.0 as f64 / 256.0
}

/// A valid `n`-symbol row: increasing cumulative values, a rate index, a zero count.
fn cdf_row() -> impl Strategy<Value = Vec<u16>> {
    (2usize..=8).prop_flat_map(|n| {
        (proptest::collection::vec(1u16..=4000, n - 1), 0u16..125).prop_map(move |(steps, rate)| {
            let mut row = Vec::with_capacity(n + 1);
            let mut acc = 0;
            for step in &steps[..n - 1] {
                acc += step;
                row.push(acc);
            }
            row.push(rate);
            row.push(0);
            row
        })
    })
}

proptest! {
    /// Costing each symbol with the row before it adapts, then adapting the row
    /// through the encoder, sums to the encoder's output length.
    #[test]
    fn symbol_costs_sum_to_the_encoded_length(
        rows in proptest::collection::vec(cdf_row(), 1..6),
        seed in any::<u64>(),
        len in 200usize..3000,
    ) {
        let mut rows = rows;
        let mut rng = Rng(seed | 1);
        let mut encoder = SymbolEncoder::new();
        let mut estimate = Bits::default();
        for _ in 0..len {
            let pick = rng.below(rows.len() as u64) as usize;
            let row = &mut rows[pick];
            let n = row.len() - 1;
            let symbol = (rng.below(n as u64) * rng.below(n as u64) / n as u64) as usize;
            estimate += symbol_cost(row, symbol);
            encoder.write_symbol_u16(row, Symbol::new(symbol as u8)).unwrap();
            if rng.below(8) == 0 {
                estimate += Bits::whole(3);
                encoder.write_literal(5, 3).unwrap();
            }
        }
        let actual = encoder.finish().unwrap().bytes().len() as f64 * 8.0;
        let estimate = bits_f64(estimate);
        prop_assert!(
            (estimate - actual).abs() <= 24.0 + actual * 0.004,
            "estimate {estimate} actual {actual}"
        );
    }
}

#[test]
fn symbol_cost_matches_known_probabilities() {
    assert_eq!(symbol_cost(&[16384, 0, 0], 0), Bits(258));
    assert_eq!(symbol_cost(&[16384, 0, 0], 1), Bits(255));
    assert!(symbol_cost(&[32767, 0, 0], 1) < Bits::whole(12));
    assert_eq!(symbol_cost(&[16384, 0, 0], 2), Bits::IMPOSSIBLE);
    assert_eq!(symbol_cost(&[0], 0), Bits::IMPOSSIBLE);
    assert_eq!(unary_cost(3, 6), Bits::whole(4));
    assert_eq!(unary_cost(6, 6), Bits::whole(6));
    assert_eq!(Bits::IMPOSSIBLE + Bits::whole(1), Bits::IMPOSSIBLE);
}

#[test]
fn lambda_follows_the_sample_domain_step() {
    for bit_depth in [BitDepth::Eight, BitDepth::Ten] {
        let mut previous = 0;
        for qindex in [20, 60, 120, 200, 255] {
            let lambda = Lambda::from_qindex(qindex, bit_depth).unwrap();
            assert!(lambda.sse_per_bit_q16() > previous);
            previous = lambda.sse_per_bit_q16();
        }
    }
    for (qindex, bit_depth) in [(120, BitDepth::Eight), (200, BitDepth::Ten)] {
        let (params, transform) = dct_recon_params(qindex, PlaneId::Y, 3, 3, bit_depth).unwrap();
        let prediction = vec![bit_depth.max_sample() / 2; 64];
        let mut levels = vec![0; 64];
        levels[1] = 1;
        let (mut dequant, mut residual, mut out) = (vec![0; 64], vec![0; 64], vec![0u16; 64]);
        reconstruct_transform_block_residual(
            &prediction,
            &levels,
            &params,
            &transform,
            &mut dequant,
            &mut residual,
            &mut out,
        )
        .unwrap();
        let energy: f64 = out
            .iter()
            .zip(&prediction)
            .map(|(&o, &p)| (f64::from(o) - f64::from(p)).powi(2))
            .sum();
        let step = f64::from(params.ac_quant) / 64.0;
        assert!(
            (energy / (step * step) - 1.0).abs() < 0.1,
            "qindex {qindex}: energy {energy} step {step}"
        );
    }
}

#[test]
fn rd_cost_is_exact_integer_arithmetic() {
    let lambda = Lambda {
        scale_q16: 1 << 16,
        step_sq_q12: 4 << 12,
    };
    assert_eq!(lambda.sse_per_bit_q16(), 4 << 16);
    let a = RdCost {
        distortion: 100,
        rate: Bits::whole(10),
    };
    let b = RdCost {
        distortion: 139,
        rate: Bits::whole(0),
    };
    assert_eq!(a.cost(lambda), 140 << 24);
    assert!(b.cost(lambda) < a.cost(lambda));
    assert_eq!((a + b).distortion, 239);
}

fn headers(
    width: u32,
    height: u32,
    bit_depth: BitDepthIdc,
    qindex: u32,
) -> (SequenceHeader, FrameHeaderCore) {
    let seq = build_encoder_sequence_header(&EncoderStreamParams {
        width,
        height,
        bit_depth,
        chroma: ChromaFormatIdc::Yuv420,
        sb_size: SuperblockSize::Block64x64,
        tools: EncoderToolFlags::default(),
    })
    .unwrap();
    let view = CoreSeqView::from_sequence(&seq).unwrap();
    let core = build_key_frame_header(&view, qindex, 0, &FrameFilterParams::default()).unwrap();
    (seq, core)
}

/// Random levels per transform block, recorded for the rate estimate.
struct RecordedLevels {
    rng: Rng,
    max_level: u64,
    coded: Vec<(TransformBlock, Vec<i32>)>,
}

impl ResidualCoder for RecordedLevels {
    fn code(
        &mut self,
        tx: &TransformBlock,
        _: &[u16],
    ) -> crate::tile_writer::Result<TransformDecision> {
        let (log2_w, log2_h) = adjusted_log2(tx.tx_size)?;
        let rng = &mut self.rng;
        let density = 1 + rng.below(8);
        let levels: Vec<i32> = (0..1usize << (log2_w + log2_h))
            .map(|pos| {
                if rng.below(16) >= density && pos > 2 {
                    return 0;
                }
                let magnitude = if rng.below(6) == 0 {
                    1 + rng.below(self.max_level)
                } else {
                    1 + rng.below(3)
                } as i32;
                if rng.below(2) == 0 {
                    magnitude
                } else {
                    -magnitude
                }
            })
            .collect();
        self.coded.push((*tx, levels.clone()));
        Ok(TransformDecision {
            tx_type: TxType::DctDct,
            levels,
        })
    }
}

/// The 64x64 frame of one `DC_PRED` leaf: the tile writer's bit count and the estimate.
fn leaf_frame_rates(bit_depth: BitDepthIdc, qindex: u32, seed: u64) -> (f64, f64) {
    let (seq, core) = headers(64, 64, bit_depth, qindex);
    let mut writer = KeyFrameTileWriter::new(&seq, &core).unwrap();
    let mut coder = RecordedLevels {
        rng: Rng(seed | 1),
        max_level: 1 + seed % 300,
        coded: Vec::new(),
    };
    let tree = PartitionTree::Leaf(BlockDecision::DC);
    writer
        .write_superblock(&SuperblockDecision { tree }, &mut coder)
        .unwrap();
    let (tile, _) = writer.finish().unwrap();

    let mut syntax = IntraTileSyntax::new(&seq, &core).unwrap();
    let root = syntax.superblock_root(0, 0);
    syntax.begin_superblock(root);
    let options = syntax.partition_options(root).unwrap();
    assert!(options.implied().is_none() && options.allows(Partition::None));
    assert!(PARTITIONS.iter().filter(|&&p| options.allows(p)).count() > 1);
    let leaf = syntax.leaf(root).unwrap();
    assert!(leaf.has_chroma());
    let mut bits = symbol_cost(
        syntax.partition_cdf(root, PartitionCdf::DoSplit).unwrap(),
        0,
    );
    bits += symbol_cost(syntax.y_mode_set_cdf().unwrap(), 0);
    bits += symbol_cost(syntax.y_mode_index_cdf(&leaf).unwrap(), 0);
    bits += symbol_cost(syntax.uv_mode_cdf(false).unwrap(), 0);
    let mut eob_u = false;
    for (tx, levels) in &coder.coded {
        let mut sink = RateSink::adapting();
        bits += block_rate(&mut syntax, &mut sink, tx, levels, tx.plane == 2 && eob_u).unwrap();
        if tx.plane == 1 {
            eob_u = levels.iter().any(|&level| level != 0);
        }
    }
    (tile.len() as f64 * 8.0, bits_f64(bits))
}

#[test]
fn coefficient_rates_match_the_tile_writer() {
    let mut worst: f64 = 0.0;
    for seed in 1..=12u64 {
        for (bit_depth, qindex) in [(BitDepthIdc::Eight, 60), (BitDepthIdc::Ten, 180)] {
            let (actual, estimate) = leaf_frame_rates(bit_depth, qindex, seed);
            let error = estimate - actual;
            worst = worst.max(error.abs() / actual);
            assert!(
                error.abs() <= 24.0 + actual * 0.004,
                "seed {seed}: estimate {estimate} actual {actual}"
            );
        }
    }
    eprintln!("worst relative coefficient-rate error {worst:.5}");
}

#[test]
fn frozen_rates_leave_rows_unchanged_and_track_adapting_rates() {
    let (seq, core) = headers(64, 64, BitDepthIdc::Eight, 90);
    let mut syntax = IntraTileSyntax::new(&seq, &core).unwrap();
    let root = syntax.superblock_root(0, 0);
    syntax.begin_superblock(root);
    let leaf = syntax.leaf(root).unwrap();
    let tx = syntax.transform_blocks(&leaf, BitDepth::Eight).unwrap()[0];
    let mut coder = RecordedLevels {
        rng: Rng(9),
        max_level: 40,
        coded: Vec::new(),
    };
    let levels = coder.code(&tx, &[]).unwrap().levels;
    let pristine = syntax.clone();
    let frozen = block_rate(&mut syntax, &mut RateSink::frozen(), &tx, &levels, false).unwrap();
    let again = block_rate(&mut syntax, &mut RateSink::frozen(), &tx, &levels, false).unwrap();
    assert_eq!(frozen, again, "frozen pricing must not adapt rows");
    let mut adapted = pristine.clone();
    let adapting =
        block_rate(&mut adapted, &mut RateSink::adapting(), &tx, &levels, false).unwrap();
    assert_ne!(frozen, adapting);
    let ratio = bits_f64(frozen) / bits_f64(adapting);
    assert!(
        (0.7..1.5).contains(&ratio),
        "frozen {frozen:?} adapting {adapting:?}"
    );
}

fn workspace(width: usize, height: usize) -> CurrentFrameWorkspace<u16> {
    let info = DecodedFrameInfo::new(
        OutputIndex::new(0),
        BitDepth::Eight,
        PixelFormat::Yuv420,
        PlaneSize::new(width, height).unwrap(),
        PlaneRect::new(0, 0, width, height).unwrap(),
    )
    .unwrap();
    CurrentFrameWorkspace::new(info, 0).unwrap()
}

fn plane_bytes(recon: &CurrentFrameWorkspace<u16>) -> Vec<u16> {
    [PlaneId::Y, PlaneId::U, PlaneId::V]
        .into_iter()
        .flat_map(|plane| recon.samples(plane).unwrap().to_vec())
        .collect()
}

#[test]
fn checkpoint_restores_facade_encoder_and_recon() {
    let (seq, core) = headers(64, 64, BitDepthIdc::Eight, 90);
    let mut syntax = IntraTileSyntax::new(&seq, &core).unwrap();
    let mut symbols = SymbolEncoder::new();
    let mut recon = workspace(64, 64);
    let root = syntax.superblock_root(0, 0);
    syntax.begin_superblock(root);
    symbols.write_literal(0x2b, 7).unwrap();
    recon
        .fill_rect(PlaneId::Y, PlaneRect::new(0, 0, 8, 8).unwrap(), 77)
        .unwrap();
    let before = plane_bytes(&recon);
    let saved = Checkpoint::save(&syntax, &symbols, &recon, (0, 0, 64, 64)).unwrap();
    let mut pristine = (syntax.clone(), symbols.clone());

    let leaf = syntax.leaf(root).unwrap();
    for _ in 0..20 {
        symbols
            .write_symbol_u16(syntax.y_mode_set_cdf().unwrap(), Symbol::new(1))
            .unwrap();
    }
    syntax.publish_dc_leaf(&leaf).unwrap();
    recon
        .fill_rect(PlaneId::U, PlaneRect::new(4, 4, 16, 16).unwrap(), 200)
        .unwrap();
    recon
        .fill_rect(PlaneId::Y, PlaneRect::new(0, 0, 64, 64).unwrap(), 9)
        .unwrap();
    assert_ne!(plane_bytes(&recon), before);

    saved
        .restore(&mut syntax, &mut symbols, &mut recon)
        .unwrap();
    assert_eq!(plane_bytes(&recon), before);
    assert_eq!(
        syntax.y_mode_set_cdf().unwrap(),
        pristine.0.y_mode_set_cdf().unwrap()
    );
    for (syntax, symbols) in [
        (&mut syntax, &mut symbols),
        (&mut pristine.0, &mut pristine.1),
    ] {
        let leaf = syntax.leaf(root).unwrap();
        symbols
            .write_symbol_u16(syntax.y_mode_index_cdf(&leaf).unwrap(), Symbol::new(0))
            .unwrap();
    }
    assert_eq!(
        symbols.finish().unwrap().bytes(),
        pristine.1.finish().unwrap().bytes()
    );
}

/// Prints what a checkpoint costs at 1920x1080; run with `--release --ignored --nocapture`.
#[test]
#[ignore = "timing report"]
fn checkpoint_cost_at_1080p() {
    use std::time::Instant;
    const ROUNDS: u32 = 200;
    for sb_size in [SuperblockSize::Block64x64, SuperblockSize::Block128x128] {
        let seq = build_encoder_sequence_header(&EncoderStreamParams {
            width: 1920,
            height: 1080,
            bit_depth: BitDepthIdc::Eight,
            chroma: ChromaFormatIdc::Yuv420,
            sb_size,
            tools: EncoderToolFlags::default(),
        })
        .unwrap();
        let core = build_key_frame_header(
            &CoreSeqView::from_sequence(&seq).unwrap(),
            90,
            0,
            &FrameFilterParams::default(),
        )
        .unwrap();
        let mut syntax = IntraTileSyntax::new(&seq, &core).unwrap();
        let copy = syntax.clone();
        let start = Instant::now();
        for _ in 0..ROUNDS {
            syntax.clone_from(std::hint::black_box(&copy));
        }
        let facade = start.elapsed() / ROUNDS;

        let mut symbols = SymbolEncoder::with_config(
            splot_core::symbol_encoder::SymbolEncoderConfig::new()
                .with_max_output_bytes(1 << 24)
                .with_max_operations(1 << 26),
        );
        for value in 0..(300_000u32 / 4) {
            symbols.write_literal(value, 32).unwrap();
        }
        let start = Instant::now();
        for _ in 0..ROUNDS {
            std::hint::black_box(symbols.clone());
        }
        let encoder = start.elapsed() / ROUNDS;

        let mut recon = workspace(1920, 1088);
        let sb = if sb_size == SuperblockSize::Block64x64 {
            64
        } else {
            128
        };
        let start = Instant::now();
        let mut saved = None;
        for _ in 0..ROUNDS {
            saved = Some(
                Checkpoint::save(&syntax, &SymbolEncoder::new(), &recon, (sb, sb, sb, sb)).unwrap(),
            );
        }
        let save = start.elapsed() / ROUNDS;
        let saved = saved.unwrap();
        let mut empty = SymbolEncoder::new();
        let start = Instant::now();
        for _ in 0..ROUNDS {
            saved.restore(&mut syntax, &mut empty, &mut recon).unwrap();
        }
        let restore = start.elapsed() / ROUNDS;
        eprintln!(
            "{sb}x{sb} SB: facade clone {facade:?}, encoder clone at 300 KB {encoder:?}, \
             save (facade + empty encoder + SB recon) {save:?}, restore {restore:?}"
        );
    }
}

#[path = "partition_tests.rs"]
mod partition_tests;
