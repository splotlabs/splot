// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use crate::error::SymbolCdfErrorKind;
use crate::symbol::{CdfUpdateMode, SymbolDecoder, SymbolDecoderConfig};
use crate::tables::conversion::PARA_ADJUSTMENT_LIST;
use crate::write::WriteError;

use super::*;

fn evenly_spaced_cdf(n: usize) -> Vec<i32> {
    let mut row = Vec::with_capacity(n + 1);
    for index in 1..n {
        row.push((index * 32_768 / n) as i32);
    }
    row.push(0);
    row.push(0);
    row
}

fn irregular_cdf(n: usize) -> Vec<i32> {
    let mut row = Vec::with_capacity(n + 1);
    let mut value = 1i32;
    for index in 0..n - 1 {
        if index > 0 && index % 2 == 1 {
            value += 1 + (index as i32 * 997) % 4096;
        }
        row.push(value);
    }
    row.push(124);
    row.push(32);
    row
}

fn assert_decoder_finish_matches(decoder: SymbolDecoder<'_>, output: &SymbolEncoderOutput) {
    let summary = decoder.finish().unwrap();
    assert_eq!(summary.symbol_count, output.symbol_count());
    assert_eq!(
        summary.padding_end_position.get(),
        (output.bytes().len() as u64) * 8
    );
}

#[test]
fn empty_stream_finalizes_to_valid_symbol_payload() {
    let output = SymbolEncoder::new().finish().unwrap();
    assert_eq!(output.symbol_count(), 0);
    assert_eq!(output.operation_count(), 0);
    assert_eq!(output.bytes(), &[0x80]);

    let decoder = SymbolDecoder::new(output.bytes()).unwrap();
    let summary = decoder.finish().unwrap();
    assert_eq!(summary.consumed_bits.get(), 8);
    assert_eq!(summary.symbol_count, output.symbol_count());
}

#[test]
fn bool_and_literal_decode_back() {
    let mut encoder = SymbolEncoder::new();
    encoder.write_bool(true).unwrap();
    encoder.write_bool(false).unwrap();
    encoder.write_literal(0b1011, 4).unwrap();
    let output = encoder.finish().unwrap();

    let mut decoder = SymbolDecoder::new(output.bytes()).unwrap();
    assert!(decoder.read_bool().unwrap());
    assert!(!decoder.read_bool().unwrap());
    assert_eq!(decoder.read_literal(4).unwrap(), 0b1011);
    assert_eq!(decoder.symbol_count(), 4);
    assert_decoder_finish_matches(decoder, &output);
}

#[test]
fn wide_literals_decode_back_across_public_domain() {
    for (value, bits) in [(0x1_ffff, 17), (0x7fff_ffff, 31), (u32::MAX, 32)] {
        let mut encoder = SymbolEncoder::new();
        encoder.write_literal(value, bits).unwrap();
        let output = encoder.finish().unwrap();

        let mut decoder = SymbolDecoder::new(output.bytes()).unwrap();
        assert_eq!(decoder.read_literal(bits).unwrap(), value);
        assert_eq!(output.symbol_count(), u64::from(bits));
        assert_decoder_finish_matches(decoder, &output);
    }
}

#[test]
fn wide_unary_values_decode_back_across_public_domain() {
    for (value, max_bits) in [
        (0, 0),
        (0, 21),
        (7, 21),
        (8, 21),
        (20, 21),
        (21, 21),
        (31, 32),
        (32, 32),
    ] {
        let mut encoder = SymbolEncoder::new();
        encoder.write_unary(value, max_bits).unwrap();
        let output = encoder.finish().unwrap();
        let expected_bits = if value < max_bits {
            value + 1
        } else {
            max_bits
        };

        let mut decoder = SymbolDecoder::new(output.bytes()).unwrap();
        assert_eq!(decoder.read_unary(max_bits).unwrap(), value);
        assert_eq!(output.symbol_count(), u64::from(expected_bits));
        assert_eq!(output.operation_count(), expected_bits.div_ceil(8) as usize);
        assert_decoder_finish_matches(decoder, &output);
    }
}

#[test]
fn symbols_decode_back_across_all_supported_arities() {
    let config = SymbolEncoderConfig::new().with_cdf_update_mode(CdfUpdateMode::Disabled);
    let decoder_config = SymbolDecoderConfig::new().with_cdf_update_mode(CdfUpdateMode::Disabled);

    for n in 2..=8 {
        for template in [evenly_spaced_cdf(n), irregular_cdf(n)] {
            for symbol in 0..n {
                let mut encoder = SymbolEncoder::with_config(config);
                let mut row = template.clone();
                let before = row.clone();
                encoder
                    .write_symbol(&mut row, Symbol::new(symbol as u8))
                    .unwrap();
                assert_eq!(row, before);
                let output = encoder.finish().unwrap();

                let mut decoder =
                    SymbolDecoder::with_config(output.bytes(), decoder_config).unwrap();
                let mut decode_row = before.clone();
                assert_eq!(
                    decoder.read_symbol(&mut decode_row).unwrap(),
                    Symbol::new(symbol as u8)
                );
                assert_eq!(decode_row, before);
                assert_decoder_finish_matches(decoder, &output);
            }
        }
    }
}

#[test]
fn symbol_cdf_updates_match_decoder_updates() {
    let mut encoder = SymbolEncoder::new();
    let mut row = vec![8192, 16_384, 24_576, 6, 0];
    encoder.write_symbol(&mut row, Symbol::new(1)).unwrap();
    let after_first = row.clone();
    encoder.write_symbol(&mut row, Symbol::new(3)).unwrap();
    let encoder_row = row.clone();
    let output = encoder.finish().unwrap();

    let mut decoder = SymbolDecoder::new(output.bytes()).unwrap();
    let mut decode_row = vec![8192, 16_384, 24_576, 6, 0];
    assert_eq!(
        decoder.read_symbol(&mut decode_row).unwrap(),
        Symbol::new(1)
    );
    assert_eq!(decode_row, after_first);
    assert_eq!(
        decoder.read_symbol(&mut decode_row).unwrap(),
        Symbol::new(3)
    );
    assert_eq!(decode_row, encoder_row);
    assert_eq!(decoder.symbol_count(), 2);
    assert_decoder_finish_matches(decoder, &output);
}

#[test]
fn deterministic_operation_stream_emits_identical_bytes() {
    fn encode() -> Vec<u8> {
        let mut encoder = SymbolEncoder::new();
        let mut row = vec![8192, 16_384, 24_576, 0, 0];
        encoder.write_bool(false).unwrap();
        encoder.write_literal(0b10, 2).unwrap();
        encoder.write_symbol(&mut row, Symbol::new(2)).unwrap();
        encoder.finish().unwrap().into_bytes()
    }

    assert_eq!(encode(), encode());
}

#[test]
fn invalid_cdf_rows_fail_before_mutation() {
    let mut encoder = SymbolEncoder::new();
    encoder.write_bool(true).unwrap();
    let mut row = vec![1, 0];
    let before = row.clone();
    assert!(matches!(
        encoder.write_symbol(&mut row, Symbol::new(0)),
        Err(WriteError::InvalidSymbolCdf {
            kind: SymbolCdfErrorKind::UnsupportedLength { len: 2 }
        })
    ));
    assert_eq!(row, before);
    assert_eq!(encoder.operation_count(), 1);
}

#[test]
fn out_of_range_symbol_fails_before_mutation() {
    let mut encoder = SymbolEncoder::new();
    let mut row = vec![16_384, 0, 0];
    let before = row.clone();
    assert!(matches!(
        encoder.write_symbol(&mut row, Symbol::new(2)),
        Err(WriteError::SymbolOutOfRange {
            symbol: 2,
            symbols: 2
        })
    ));
    assert_eq!(row, before);
    assert_eq!(encoder.operation_count(), 0);
}

#[test]
fn literal_domain_errors_fail_before_mutation() {
    let mut encoder = SymbolEncoder::new();
    assert!(matches!(
        encoder.write_literal(0, 33),
        Err(WriteError::BitWidthTooLarge {
            requested: 33,
            max: 32
        })
    ));
    assert!(matches!(
        encoder.write_literal(2, 1),
        Err(WriteError::ValueTooWide {
            value: 2,
            width_bits: 1
        })
    ));
    assert_eq!(encoder.operation_count(), 0);
    assert_eq!(encoder.symbol_count(), 0);
}

#[test]
fn unary_domain_errors_fail_before_mutation() {
    let baseline = SymbolEncoder::new();

    let mut width_encoder = baseline.clone();
    assert_eq!(
        width_encoder.write_unary(0, 33),
        Err(WriteError::BitWidthTooLarge {
            requested: 33,
            max: 32
        })
    );
    assert_eq!(width_encoder.operation_count(), baseline.operation_count());
    assert_eq!(width_encoder.symbol_count(), baseline.symbol_count());

    let mut value_encoder = baseline.clone();
    assert_eq!(
        value_encoder.write_unary(33, 32),
        Err(WriteError::ValueTooWide {
            value: 33,
            width_bits: 32
        })
    );
    assert_eq!(value_encoder.operation_count(), baseline.operation_count());
    assert_eq!(value_encoder.symbol_count(), baseline.symbol_count());
}

#[test]
fn output_limit_is_checked_before_mutation() {
    let config = SymbolEncoderConfig::new().with_max_output_bytes(1);
    let mut encoder = SymbolEncoder::with_config(config);
    encoder.write_literal(0x55, 7).unwrap();
    assert!(matches!(
        encoder.write_bool(true),
        Err(WriteError::SymbolOutputTooLarge {
            requested: 2,
            limit: 1
        })
    ));
    assert_eq!(encoder.operation_count(), 1);
    assert_eq!(encoder.finish().unwrap().bytes().len(), 1);

    let encoder = SymbolEncoder::with_config(config.with_max_output_bytes(0));
    assert!(matches!(
        encoder.finish(),
        Err(WriteError::SymbolOutputTooLarge {
            requested: 1,
            limit: 0
        })
    ));
}

#[test]
fn operation_limit_bounds_high_skew_zero_bit_symbols() {
    let config = SymbolEncoderConfig::new()
        .with_max_output_bytes(2)
        .with_max_operations(1);
    let mut encoder = SymbolEncoder::with_config(config);
    let mut row = vec![32_767, 0, 0];
    encoder.write_symbol(&mut row, Symbol::new(0)).unwrap();
    assert_eq!(encoder.operation_count(), 1);

    let before = row.clone();
    assert!(matches!(
        encoder.write_symbol(&mut row, Symbol::new(0)),
        Err(WriteError::SymbolOperationLimit {
            requested: 2,
            limit: 1
        })
    ));
    assert_eq!(row, before);
    assert_eq!(encoder.operation_count(), 1);
}

/// Deterministic xorshift64* source for long seeded operation streams.
pub(super) struct Rng(u64);

impl Rng {
    pub(super) fn new(seed: u64) -> Self {
        Self((seed << 1) | 1)
    }

    pub(super) fn below(&mut self, bound: u64) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d) % bound
    }
}

#[derive(Debug, Clone, Copy)]
pub(super) enum MixedOp {
    Bool(bool),
    Literal { value: u32, bits: u32 },
    Unary { value: u32, max_bits: u32 },
    Symbol { row: usize, symbol: u8 },
    SymbolU16 { row: usize, symbol: u8 },
}

/// A seeded operation stream plus the CDF banks it starts from.
pub(super) struct MixedStream {
    pub(super) ops: Vec<MixedOp>,
    pub(super) rows: Vec<Vec<i32>>,
    pub(super) rows_u16: Vec<Vec<u16>>,
}

fn random_cdf(rng: &mut Rng, n: usize) -> Vec<i32> {
    let mut row: Vec<i32> = (1..n).map(|_| 1 + rng.below(32_767) as i32).collect();
    row.sort_unstable();
    row.push(rng.below(PARA_ADJUSTMENT_LIST.len() as u64) as i32);
    row.push(rng.below(33) as i32);
    row
}

/// Draws half the symbols from the row distribution so long runs of likely
/// symbols (and their carries) appear next to uniform picks.
fn pick_symbol(rng: &mut Rng, row: &[i32]) -> u8 {
    let n = row.len() - 1;
    if rng.below(2) == 0 {
        return rng.below(n as u64) as u8;
    }
    let draw = rng.below(32_768) as i32;
    row[..n - 1]
        .iter()
        .take_while(|&&entry| entry <= draw)
        .count() as u8
}

pub(super) fn mixed_stream(seed: u64, len: usize, symbols_only: bool) -> MixedStream {
    let mut rng = Rng::new(seed);
    let rows: Vec<Vec<i32>> = (0..28).map(|i| random_cdf(&mut rng, 2 + i % 7)).collect();
    let rows_u16 = rows
        .iter()
        .map(|row| row.iter().map(|&v| v as u16).collect())
        .collect();
    let ops = (0..len)
        .map(|_| {
            let tag = if symbols_only { 9 } else { rng.below(10) };
            let row = rng.below(rows.len() as u64) as usize;
            match tag {
                0 | 1 => MixedOp::Bool(rng.below(2) == 1),
                2 | 3 => {
                    let bits = rng.below(33) as u32;
                    let value = (rng.below(1 << 32) as u32) & ((1u64 << bits) - 1) as u32;
                    MixedOp::Literal { value, bits }
                }
                4 => {
                    let max_bits = rng.below(33) as u32;
                    let value = rng.below(u64::from(max_bits) + 1) as u32;
                    MixedOp::Unary { value, max_bits }
                }
                5..=7 => MixedOp::Symbol {
                    row,
                    symbol: pick_symbol(&mut rng, &rows[row]),
                },
                _ => MixedOp::SymbolU16 {
                    row,
                    symbol: pick_symbol(&mut rng, &rows[row]),
                },
            }
        })
        .collect();
    MixedStream {
        ops,
        rows,
        rows_u16,
    }
}

pub(super) fn encode_mixed(stream: &MixedStream, mode: CdfUpdateMode) -> SymbolEncoderOutput {
    let config = SymbolEncoderConfig::new()
        .with_cdf_update_mode(mode)
        .with_max_output_bytes(64 << 20)
        .with_max_operations(16 << 20);
    let mut encoder = SymbolEncoder::with_config(config);
    let mut rows = stream.rows.clone();
    let mut rows_u16 = stream.rows_u16.clone();
    for op in &stream.ops {
        match *op {
            MixedOp::Bool(value) => encoder.write_bool(value),
            MixedOp::Literal { value, bits } => encoder.write_literal(value, bits),
            MixedOp::Unary { value, max_bits } => encoder.write_unary(value, max_bits),
            MixedOp::Symbol { row, symbol } => {
                encoder.write_symbol(&mut rows[row], Symbol::new(symbol))
            }
            MixedOp::SymbolU16 { row, symbol } => {
                encoder.write_symbol_u16(&mut rows_u16[row], Symbol::new(symbol))
            }
        }
        .unwrap();
    }
    encoder.finish().unwrap()
}

/// Decodes `output` op by op and runs the decoder's `exit_symbol()` checks.
pub(super) fn assert_mixed_roundtrip(
    stream: &MixedStream,
    mode: CdfUpdateMode,
    output: &SymbolEncoderOutput,
) {
    let config = SymbolDecoderConfig::new().with_cdf_update_mode(mode);
    let mut decoder = SymbolDecoder::with_config(output.bytes(), config).unwrap();
    let mut rows = stream.rows.clone();
    let mut rows_u16 = stream.rows_u16.clone();
    for (index, op) in stream.ops.iter().enumerate() {
        let matches = match *op {
            MixedOp::Bool(value) => decoder.read_bool().unwrap() == value,
            MixedOp::Literal { value, bits } => decoder.read_literal(bits).unwrap() == value,
            MixedOp::Unary { value, max_bits } => decoder.read_unary(max_bits).unwrap() == value,
            MixedOp::Symbol { row, symbol } => {
                decoder.read_symbol(&mut rows[row]).unwrap().get() == symbol
            }
            MixedOp::SymbolU16 { row, symbol } => {
                decoder.read_symbol_u16(&mut rows_u16[row]).unwrap().get() == symbol
            }
        };
        assert!(matches, "op {index} {op:?} decoded differently");
    }
    assert_decoder_finish_matches(decoder, output);
}

#[test]
#[ignore = "timing probe; run with --ignored --nocapture in release"]
fn encode_one_million_symbols_timing() {
    let len = std::env::var("SPLOT_SYMBOL_BENCH_N")
        .ok()
        .and_then(|n| n.parse().ok())
        .unwrap_or(1_000_000);
    let stream = mixed_stream(0x5eed, len, true);
    let start = std::time::Instant::now();
    let output = encode_mixed(&stream, CdfUpdateMode::Enabled);
    let elapsed = start.elapsed();
    println!(
        "encoded {len} symbols into {} bytes in {elapsed:?}",
        output.bytes().len()
    );
    assert_mixed_roundtrip(&stream, CdfUpdateMode::Enabled, &output);
}

fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, &byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
    })
}

/// The brute-force encoder this one replaced always wrote 15 + K bits for K
/// coded bits and picked the smallest valid final value. The streaming
/// encoder picks the same value but stops after the trailing one bit, so its
/// output is the old output without the old all-zero tail bytes. Goldens are
/// (seed, update mode, old length, FNV-1a of the old bytes).
#[test]
fn output_is_old_brute_force_output_without_zero_tail() {
    let goldens = [
        (1, CdfUpdateMode::Enabled, 206, 0xf484_344c_971a_b475),
        (1, CdfUpdateMode::Disabled, 206, 0x0a75_ee69_940b_914e),
        (2, CdfUpdateMode::Enabled, 176, 0x470f_4097_8925_64bf),
        (2, CdfUpdateMode::Disabled, 177, 0x50ed_f1da_f41d_7c22),
        (3, CdfUpdateMode::Enabled, 207, 0x1b98_4988_4da1_175b),
        (3, CdfUpdateMode::Disabled, 206, 0x9ad3_8917_f8cb_fd7a),
        (4, CdfUpdateMode::Enabled, 207, 0xa89f_1423_76f2_4ddd),
        (4, CdfUpdateMode::Disabled, 206, 0x3603_9d68_7fae_7b18),
        (5, CdfUpdateMode::Enabled, 190, 0xa8ec_cb30_7805_6b9c),
        (5, CdfUpdateMode::Disabled, 188, 0xf77f_8a96_60d5_ae52),
        (6, CdfUpdateMode::Enabled, 193, 0x0ff0_42b5_f346_1676),
        (6, CdfUpdateMode::Disabled, 192, 0xd37c_69d2_3641_d2dd),
    ];
    for (seed, mode, old_len, old_hash) in goldens {
        let stream = mixed_stream(seed, 300, false);
        let output = encode_mixed(&stream, mode);
        assert_mixed_roundtrip(&stream, mode, &output);
        let mut bytes = output.into_bytes();
        assert!((1..=2).contains(&(old_len - bytes.len())), "seed {seed}");
        bytes.resize(old_len, 0);
        assert_eq!(fnv1a(&bytes), old_hash, "seed {seed} {mode:?}");
    }
}

#[test]
fn adapt_cdf_matches_the_encoder_update() {
    let rows: [[u16; 5]; 1] = [[8192, 16384, 24576, 0, 0]];
    for symbol in 0..4u8 {
        let mut coded = rows[0];
        let mut adapted = rows[0];
        let mut encoder = SymbolEncoder::new();
        for _ in 0..40 {
            encoder
                .write_symbol_u16(&mut coded, Symbol::new(symbol))
                .unwrap();
            adapt_cdf_u16(&mut adapted, Symbol::new(symbol)).unwrap();
            assert_eq!(coded, adapted);
        }
    }
    let mut row = rows[0];
    assert!(adapt_cdf_u16(&mut row, Symbol::new(4)).is_err());
}

#[test]
fn symbol_split_points_tile_the_scale() {
    let row = [8192u16, 16384, 24576, 0, 0];
    let splits: Vec<u32> = (0..4)
        .map(|symbol| symbol_split_q12(&row, symbol).unwrap())
        .collect();
    assert_eq!(splits, [3072 + 12, 2048 + 8, 1024 + 4, 0]);
    assert_eq!(symbol_split_q12(&row, 4), None);
    assert_eq!(symbol_split_q12(&[0, 0], 0), None);
}
