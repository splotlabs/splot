// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::unwrap_used)]

use splot_core::span::ByteOffset;
use splot_core::symbol::{CdfUpdateMode, SymbolDecoderConfig};
use splot_core::symbol_encoder::SymbolEncoder;

use super::*;

const ENTRY: CoeffScanEntry = CoeffScanEntry::new(3, 9, 1, 1);

fn symbol_decoder(payload: &[u8]) -> SymbolDecoder<'_> {
    SymbolDecoder::with_base_and_config(
        payload,
        ByteOffset::new(0),
        SymbolDecoderConfig::new().with_cdf_update_mode(CdfUpdateMode::Enabled),
    )
    .unwrap()
}

fn config(hr_level_avg: u32) -> CoeffReadQuantConfig {
    CoeffReadQuantConfig {
        is_hidden: false,
        allow_tcq: false,
        hr_level_avg,
    }
}

fn input(level: u32, max_level: u32) -> CoeffReadQuantInput {
    CoeffReadQuantInput {
        entry: ENTRY,
        level,
        max_level,
    }
}

fn encode_extended(q: u32, c_max: u32, prefix: u32, length: u32, remainder: u32) -> Vec<u8> {
    let mut encoder = SymbolEncoder::new();
    encoder.write_unary(q, c_max).unwrap();
    if q == c_max {
        encoder
            .write_unary(prefix, MAX_EXP_GOLOMB_PREFIX_BITS)
            .unwrap();
    }
    encoder.write_literal(remainder, length).unwrap();
    encoder.finish().unwrap().into_bytes()
}

#[test]
fn below_threshold_consumes_no_bits() {
    let mut symbols = symbol_decoder(&[0x80]);
    let consumed_before = symbols.consumed_bits();
    let mut state = CoeffReadQuantState::new(config(7));

    let quant = state.read_one(&mut symbols, 0, input(2, 5)).unwrap();

    assert_eq!(quant.quant, 2);
    assert_eq!(symbols.consumed_bits(), consumed_before);
}

#[test]
fn finite_and_golomb_extensions_decode_expected_quant() {
    let finite = encode_extended(2, 6, 0, 4, 10);
    let mut symbols = symbol_decoder(&finite);
    let mut state = CoeffReadQuantState::new(config(16));
    assert_eq!(
        state.read_one(&mut symbols, 0, input(3, 3)).unwrap().quant,
        45
    );

    let golomb = encode_extended(5, 5, 1, 3, 5);
    let mut symbols = symbol_decoder(&golomb);
    let mut state = CoeffReadQuantState::new(config(1));
    assert_eq!(
        state.read_one(&mut symbols, 0, input(2, 2)).unwrap().quant,
        21
    );
}

#[test]
fn state_carries_hr_average_between_entries() {
    let mut encoder = SymbolEncoder::new();
    encoder.write_unary(3, 5).unwrap();
    encoder.write_literal(1, 1).unwrap();
    encoder.write_unary(1, 6).unwrap();
    encoder.write_literal(3, 2).unwrap();
    let bytes = encoder.finish().unwrap().into_bytes();
    let mut symbols = symbol_decoder(&bytes);
    let mut state = CoeffReadQuantState::new(config(1));
    let first = state.read_one(&mut symbols, 0, input(4, 4)).unwrap();
    let second = state
        .read_one(
            &mut symbols,
            1,
            CoeffReadQuantInput {
                entry: CoeffScanEntry::new(2, 10, 1, 2),
                level: 4,
                max_level: 4,
            },
        )
        .unwrap();

    assert_eq!(first.quant, 11);
    assert_eq!(second.quant, 11);
    assert!(symbols.finish().is_ok());
}

#[test]
fn invalid_max_level_is_rejected() {
    let mut symbols = symbol_decoder(&[0xff, 0x80]);
    let mut state = CoeffReadQuantState::new(CoeffReadQuantConfig {
        is_hidden: false,
        allow_tcq: true,
        hr_level_avg: 0,
    });
    assert!(matches!(
        state.read_one(&mut symbols, 0, input(0, 0)),
        Err(CoeffReadQuantError::InvalidMaxLevel {
            max_level: 0,
            allow_tcq: true,
            ..
        })
    ));
}

#[test]
fn golomb_prefix_boundary_is_writer_backed() {
    let maximum = encode_extended(5, 5, 20, 22, 0);
    let mut symbols = symbol_decoder(&maximum);
    let mut state = CoeffReadQuantState::new(config(1));
    assert!(state.read_one(&mut symbols, 0, input(2, 2)).is_ok());

    let overlong = encode_extended(5, 5, 21, 0, 0);
    let mut symbols = symbol_decoder(&overlong);
    let mut state = CoeffReadQuantState::new(config(1));
    assert!(matches!(
        state.read_one(&mut symbols, 0, input(2, 2)),
        Err(CoeffReadQuantError::OverlongGolombPrefix { index: 0 })
    ));
}
