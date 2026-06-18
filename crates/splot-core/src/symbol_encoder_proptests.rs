// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use crate::symbol::{CDF_PROB_MAX, CdfUpdateMode, MAX_SYMBOLS, SymbolDecoder, SymbolDecoderConfig};
use crate::tables::conversion::PARA_ADJUSTMENT_LIST;
use proptest::prelude::*;

use super::*;

// ENC-BITSTREAM-WRITER property evidence for the generic AV2 § 8.2 symbol encoder.

#[derive(Debug, Clone)]
enum Op {
    Bool(bool),
    Literal { value: u32, bits: u32 },
    Symbol { symbol: u8, cdf: Vec<i32> },
}

#[derive(Debug, Clone)]
enum ExpectedOp {
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

prop_compose! {
    fn literal_op()(bits in 0u32..=32, seed in any::<u32>()) -> Op {
        let value = if bits == 0 {
            0
        } else if bits == u32::BITS {
            seed
        } else {
            seed & ((1u32 << bits) - 1)
        };
        Op::Literal { value, bits }
    }
}

prop_compose! {
    fn symbol_op()(
        n in 2usize..=8,
        symbol_seed in any::<u8>(),
        entries in proptest::collection::vec(1i32..=CDF_PROB_MAX, MAX_SYMBOLS - 1),
        rate in 0usize..PARA_ADJUSTMENT_LIST.len(),
        count in 0i32..=32,
    ) -> Op {
        let mut entries = entries;
        entries.truncate(n - 1);
        entries.sort_unstable();
        let mut cdf = entries;
        cdf.push(rate as i32);
        cdf.push(count);
        Op::Symbol {
            symbol: symbol_seed % n as u8,
            cdf,
        }
    }
}

prop_compose! {
    fn op_strategy()(
        tag in 0u8..3,
        bool_value in any::<bool>(),
        literal in literal_op(),
        symbol in symbol_op(),
    ) -> Op {
        match tag {
            0 => Op::Bool(bool_value),
            1 => literal,
            _ => symbol,
        }
    }
}

proptest! {
    #[test]
    fn symbol_encoder_roundtrips_bounded_operation_stream(
        ops in proptest::collection::vec(op_strategy(), 0..24),
        updates_enabled in any::<bool>(),
    ) {
        let mode = if updates_enabled {
            CdfUpdateMode::Enabled
        } else {
            CdfUpdateMode::Disabled
        };
        let encoder_config = SymbolEncoderConfig::new()
            .with_cdf_update_mode(mode)
            .with_max_output_bytes(512);
        let decoder_config = SymbolDecoderConfig::new().with_cdf_update_mode(mode);

        let mut encoder = SymbolEncoder::with_config(encoder_config);
        let mut expected_ops = Vec::with_capacity(ops.len());
        for op in ops {
            match op {
                Op::Bool(value) => {
                    encoder.write_bool(value).unwrap();
                    expected_ops.push(ExpectedOp::Bool(value));
                }
                Op::Literal { value, bits } => {
                    encoder.write_literal(value, bits).unwrap();
                    expected_ops.push(ExpectedOp::Literal { value, bits });
                }
                Op::Symbol { symbol, mut cdf } => {
                    let before = cdf.clone();
                    encoder.write_symbol(&mut cdf, Symbol::new(symbol)).unwrap();
                    expected_ops.push(ExpectedOp::Symbol {
                        symbol,
                        cdf_before: before,
                        cdf_after: cdf,
                    });
                }
            }
        }

        let output = encoder.finish().unwrap();
        prop_assert!(output.bytes().len() <= encoder_config.max_output_bytes());

        let mut decoder = SymbolDecoder::with_config(output.bytes(), decoder_config).unwrap();
        for op in expected_ops {
            match op {
                ExpectedOp::Bool(value) => {
                    prop_assert_eq!(decoder.read_bool().unwrap(), value);
                }
                ExpectedOp::Literal { value, bits } => {
                    prop_assert_eq!(decoder.read_literal(bits).unwrap(), value);
                }
                ExpectedOp::Symbol {
                    symbol,
                    mut cdf_before,
                    cdf_after,
                } => {
                    prop_assert_eq!(
                        decoder.read_symbol(&mut cdf_before).unwrap(),
                        Symbol::new(symbol)
                    );
                    prop_assert_eq!(cdf_before, cdf_after);
                }
            }
        }
        let summary = decoder.finish().unwrap();
        prop_assert_eq!(summary.symbol_count, output.symbol_count());
    }
}
