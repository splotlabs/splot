// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use crate::error::SymbolCdfErrorKind;
use crate::symbol::{CdfUpdateMode, SymbolDecoder, SymbolDecoderConfig};
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
    assert_eq!(output.bytes(), &[0x80, 0x00]);

    let decoder = SymbolDecoder::new(output.bytes()).unwrap();
    let summary = decoder.finish().unwrap();
    assert_eq!(summary.consumed_bits.get(), 16);
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
    let mut encoder = SymbolEncoder::new();
    assert!(matches!(
        encoder.write_unary(0, 33),
        Err(WriteError::BitWidthTooLarge {
            requested: 33,
            max: 32
        })
    ));
    assert!(matches!(
        encoder.write_unary(33, 32),
        Err(WriteError::ValueTooWide {
            value: 33,
            width_bits: 32
        })
    ));
    assert_eq!(encoder.operation_count(), 0);
    assert_eq!(encoder.symbol_count(), 0);
}

#[test]
fn output_limit_is_checked_before_mutation() {
    let config = SymbolEncoderConfig::new().with_max_output_bytes(1);
    let mut encoder = SymbolEncoder::with_config(config);
    assert!(matches!(
        encoder.write_bool(true),
        Err(WriteError::SymbolOutputTooLarge {
            requested: 2,
            limit: 1
        })
    ));
    assert_eq!(encoder.operation_count(), 0);

    let encoder = SymbolEncoder::with_config(config);
    assert!(matches!(
        encoder.finish(),
        Err(WriteError::SymbolOutputTooLarge {
            requested: 2,
            limit: 1
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
