// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>
#![no_main]

use libfuzzer_sys::fuzz_target;
use splot_core::symbol::{CdfUpdateMode, Symbol, SymbolDecoder, SymbolDecoderConfig};
use splot_core::symbol_encoder::{SymbolEncoder, SymbolEncoderConfig};
use splot_core::write::WriteError;

const UPDATE_MODE_FLAG: u8 = 0b0000_0001;
const MAX_OPS: usize = 48;
const MAX_ENCODER_OPERATIONS: usize = MAX_OPS * 32;
const MAX_OUTPUT_BYTES: usize = 1024;
const OP_BOOL: u8 = 0;
const OP_LITERAL: u8 = 1;
const OP_SYMBOL: u8 = 2;
const OP_BAD_CDF: u8 = 3;
const OP_BAD_SYMBOL: u8 = 4;

fuzz_target!(|data: &[u8]| {
    let Some((flags, rest)) = data.split_first() else {
        return;
    };
    let Some((op_count_seed, rest)) = rest.split_first() else {
        return;
    };

    let mode = if flags & UPDATE_MODE_FLAG == 0 {
        CdfUpdateMode::Disabled
    } else {
        CdfUpdateMode::Enabled
    };
    let config = SymbolEncoderConfig::new()
        .with_cdf_update_mode(mode)
        .with_max_output_bytes(MAX_OUTPUT_BYTES)
        .with_max_operations(MAX_ENCODER_OPERATIONS);
    let mut encoder = SymbolEncoder::with_config(config);
    let mut expected = Vec::new();
    let mut cursor = ByteCursor::new(rest);
    let op_count = 1 + usize::from(*op_count_seed % MAX_OPS as u8);

    for _ in 0..op_count {
        match cursor.next().unwrap_or(0) % 5 {
            OP_BOOL => {
                let value = cursor.next().unwrap_or(0) & 1 != 0;
                encoder
                    .write_bool(value)
                    .unwrap_or_else(|err| panic!("valid bool should encode: {err:?}"));
                expected.push(Op::Bool(value));
            }
            OP_LITERAL => {
                let bits = u32::from(cursor.next().unwrap_or(0) % 33);
                let value = literal_value(bits, &mut cursor);
                encoder
                    .write_literal(value, bits)
                    .unwrap_or_else(|err| panic!("valid literal should encode: {err:?}"));
                expected.push(Op::Literal { value, bits });
            }
            OP_SYMBOL => {
                let n = 2 + usize::from(cursor.next().unwrap_or(0) % 7);
                let symbol = cursor.next().unwrap_or(0) % n as u8;
                let mut cdf = cdf_row(n, &mut cursor);
                let before = cdf.clone();
                encoder
                    .write_symbol(&mut cdf, Symbol::new(symbol))
                    .unwrap_or_else(|err| panic!("valid symbol should encode: {err:?}"));
                expected.push(Op::Symbol {
                    symbol,
                    cdf_before: before,
                    cdf_after: cdf,
                });
            }
            OP_BAD_CDF => {
                let mut cdf = malformed_cdf(&mut cursor);
                let before = cdf.clone();
                match encoder.write_symbol(&mut cdf, Symbol::new(0)) {
                    Err(WriteError::InvalidSymbolCdf { .. }) => assert_eq!(cdf, before),
                    Err(err) => panic!("malformed cdf returned unexpected error: {err:?}"),
                    Ok(()) => panic!("malformed cdf was accepted"),
                }
            }
            OP_BAD_SYMBOL => {
                let mut cdf = cdf_row(2, &mut cursor);
                let before = cdf.clone();
                match encoder.write_symbol(&mut cdf, Symbol::new(2)) {
                    Err(WriteError::SymbolOutOfRange { symbol, symbols }) => {
                        assert_eq!(symbol, 2);
                        assert_eq!(symbols, 2);
                        assert_eq!(cdf, before);
                    }
                    Err(err) => panic!("bad symbol returned unexpected error: {err:?}"),
                    Ok(()) => panic!("out-of-range symbol was accepted"),
                }
            }
            _ => {}
        }
    }

    let output = encoder
        .finish()
        .unwrap_or_else(|err| panic!("bounded symbol encoder stream should finish: {err:?}"));
    assert!(output.bytes().len() <= MAX_OUTPUT_BYTES);

    let decoder_config = SymbolDecoderConfig::new().with_cdf_update_mode(mode);
    let mut decoder = SymbolDecoder::with_config(output.bytes(), decoder_config)
        .unwrap_or_else(|err| panic!("encoder output should initialize decoder: {err:?}"));
    for op in expected {
        match op {
            Op::Bool(value) => {
                assert_eq!(
                    decoder
                        .read_bool()
                        .unwrap_or_else(|err| panic!("bool should decode: {err:?}")),
                    value
                );
            }
            Op::Literal { value, bits } => {
                assert_eq!(
                    decoder
                        .read_literal(bits)
                        .unwrap_or_else(|err| panic!("literal should decode: {err:?}")),
                    value
                );
            }
            Op::Symbol {
                symbol,
                mut cdf_before,
                cdf_after,
            } => {
                assert_eq!(
                    decoder
                        .read_symbol(&mut cdf_before)
                        .unwrap_or_else(|err| panic!("symbol should decode: {err:?}")),
                    Symbol::new(symbol)
                );
                assert_eq!(cdf_before, cdf_after);
            }
        }
    }
    let summary = decoder
        .finish()
        .unwrap_or_else(|err| panic!("encoder output should finish decoder: {err:?}"));
    assert_eq!(summary.symbol_count, output.symbol_count());
});

#[derive(Debug)]
enum Op {
    Bool(bool),
    Literal {
        value: u32,
        bits: u32,
    },
    Symbol {
        symbol: u8,
        cdf_before: Vec<i32>,
        cdf_after: Vec<i32>,
    },
}

fn literal_value(bits: u32, cursor: &mut ByteCursor<'_>) -> u32 {
    if bits == 0 {
        return 0;
    }
    let mut value = 0u32;
    let bytes = bits.div_ceil(8);
    for _ in 0..bytes {
        value = (value << 8) | u32::from(cursor.next().unwrap_or(0));
    }
    if bits < 32 {
        value & ((1u32 << bits) - 1)
    } else {
        value
    }
}

fn cdf_row(n: usize, cursor: &mut ByteCursor<'_>) -> Vec<i32> {
    let shape = cursor.next().unwrap_or(0) % 4;
    let mut row = Vec::with_capacity(n + 1);
    match shape {
        0 => row.resize(n - 1, 32_767),
        1 => row.resize(n - 1, 1),
        _ => {
            for _ in 0..n - 1 {
                row.push(1 + i32::from(next_u16(cursor) % 32_767));
            }
            row.sort_unstable();
        }
    }
    row.push(i32::from(cursor.next().unwrap_or(0) % 125));
    row.push(i32::from(cursor.next().unwrap_or(0) % 33));
    row
}

fn malformed_cdf(cursor: &mut ByteCursor<'_>) -> Vec<i32> {
    match cursor.next().unwrap_or(0) % 5 {
        0 => vec![1, 0],
        1 => vec![0, 0, 0],
        2 => vec![100, 99, 0, 0],
        3 => vec![16_384, 125, 0],
        _ => vec![16_384, 0, 33],
    }
}

fn next_u16(cursor: &mut ByteCursor<'_>) -> u16 {
    let hi = u16::from(cursor.next().unwrap_or(0));
    let lo = u16::from(cursor.next().unwrap_or(0));
    (hi << 8) | lo
}

#[derive(Debug)]
struct ByteCursor<'a> {
    bytes: &'a [u8],
    index: usize,
}

impl<'a> ByteCursor<'a> {
    const fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, index: 0 }
    }

    fn next(&mut self) -> Option<u8> {
        let byte = self.bytes.get(self.index).copied()?;
        self.index += 1;
        Some(byte)
    }
}
