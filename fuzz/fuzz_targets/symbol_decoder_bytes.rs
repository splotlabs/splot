// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>
//
// Fuzz target: the public AV2 §8.2 symbol decoder must return typed results,
// never panic, on bounded arbitrary tile-payload bytes and bounded valid or
// malformed caller-supplied CDF rows. This target intentionally does not perform
// §8.3 syntax-element CDF selection, tile-payload traversal, reconstruction,
// runtime output, filesystem I/O, or AVM/dav2d invocation. Run with:
//
//     cargo install cargo-fuzz --locked
//     cargo +nightly fuzz run symbol_decoder_bytes
#![no_main]

use libfuzzer_sys::fuzz_target;
use splot_core::error::{Error, SymbolDecoderErrorKind};
use splot_core::symbol::{
    CdfUpdateMode, SymbolDecoder, SymbolDecoderCheckpoint, SymbolDecoderConfig,
};

const UPDATE_MODE_FLAG: u8 = 0b0000_0001;
const FINISH_ALIAS_FLAG: u8 = 0b0000_0010;
const MAX_PAYLOAD_BYTES: usize = 128;
const MAX_OPS: usize = 32;
const MIN_SYMBOLS: usize = 2;
const CDF_PROB_MAX: i32 = 32_767;
const CDF_RATE_ROWS: u8 = 125;
const MAX_CDF_COUNT: u8 = 32;
const OP_READ_BOOL: u8 = 0;
const OP_READ_LITERAL: u8 = 1;
const OP_READ_VALID_SYMBOL: u8 = 2;
const OP_READ_MALFORMED_SYMBOL: u8 = 3;
const OP_CHECKPOINT: u8 = 4;
const OP_FINISH: u8 = 5;

fuzz_target!(|data: &[u8]| {
    let Some((flags, rest)) = data.split_first() else {
        return;
    };
    let Some((payload_len_seed, rest)) = rest.split_first() else {
        return;
    };

    let payload_len = usize::from(*payload_len_seed).min(MAX_PAYLOAD_BYTES);
    let payload_len = payload_len.min(rest.len());
    let (payload, rest) = rest.split_at(payload_len);
    let Some((op_count_seed, ops)) = rest.split_first() else {
        return;
    };

    let config = if flags & UPDATE_MODE_FLAG == 0 {
        SymbolDecoderConfig::new().with_cdf_update_mode(CdfUpdateMode::Disabled)
    } else {
        SymbolDecoderConfig::new().with_cdf_update_mode(CdfUpdateMode::Enabled)
    };

    let Ok(mut decoder) = SymbolDecoder::with_config(payload, config) else {
        return;
    };

    let mut cursor = ByteCursor::new(ops);
    let op_count = 1 + usize::from(*op_count_seed % MAX_OPS as u8);
    let mut previous_checkpoint = decoder.checkpoint();

    for _ in 0..op_count {
        let op = cursor.next().unwrap_or(0) % 6;
        match op {
            OP_READ_BOOL => {
                let _ = decoder.read_bool();
            }
            OP_READ_LITERAL => {
                let width = u32::from(cursor.next().unwrap_or(0) % 41);
                let before_count = decoder.symbol_count();
                match decoder.read_literal(width) {
                    Ok(value) => assert_literal_result(width, value),
                    Err(Error::InvalidSymbolDecoderState {
                        kind: SymbolDecoderErrorKind::LiteralWidthTooLarge { requested, max },
                        ..
                    }) => {
                        assert!(requested > max);
                        assert_eq!(decoder.symbol_count(), before_count);
                    }
                    Err(_) => {}
                }
            }
            OP_READ_VALID_SYMBOL => {
                let n = MIN_SYMBOLS + usize::from(cursor.next().unwrap_or(0) % 7);
                let mut cdf = valid_cdf_row(n, &mut cursor);
                let before = cdf.clone();
                let mode = config.cdf_update_mode();
                if let Ok(symbol) = decoder.read_symbol(&mut cdf) {
                    assert!(usize::from(symbol.get()) < n);
                    assert_valid_cdf_after_read(mode, &before, &cdf);
                }
            }
            OP_READ_MALFORMED_SYMBOL => {
                let mut cdf = malformed_cdf_row(&mut cursor);
                let before = cdf.clone();
                if decoder.read_symbol(&mut cdf).is_err() {
                    assert_eq!(cdf, before);
                }
            }
            OP_CHECKPOINT => {
                let checkpoint = decoder.checkpoint();
                assert_checkpoint_monotonic(previous_checkpoint, checkpoint);
                previous_checkpoint = checkpoint;
            }
            OP_FINISH => {
                let symbol_count = decoder.symbol_count();
                let result = if flags & FINISH_ALIAS_FLAG == 0 {
                    decoder.exit_symbol()
                } else {
                    decoder.finish()
                };
                if let Ok(summary) = result {
                    assert_symbol_summary(payload.len(), symbol_count, summary);
                }
                return;
            }
            _ => {}
        }
    }
});

fn assert_literal_result(width: u32, value: u32) {
    assert!(width <= 32);
    if width < 32 {
        assert!(value < (1u32 << width));
    }
}

fn assert_valid_cdf_after_read(mode: CdfUpdateMode, before: &[i32], after: &[i32]) {
    assert_eq!(after.len(), before.len());
    if mode == CdfUpdateMode::Disabled {
        assert_eq!(after, before);
    }

    let n = after.len() - 1;
    for index in 0..n - 1 {
        let value = after[index];
        assert!((1..=CDF_PROB_MAX).contains(&value));
        if index > 0 {
            assert!(value > after[index - 1]);
        }
    }
    assert!((0..i32::from(CDF_RATE_ROWS)).contains(&after[n - 1]));
    assert!((0..=i32::from(MAX_CDF_COUNT)).contains(&after[n]));
}

fn assert_checkpoint_monotonic(before: SymbolDecoderCheckpoint, after: SymbolDecoderCheckpoint) {
    assert!(after.consumed_bits >= before.consumed_bits);
    assert!(after.symbol_count >= before.symbol_count);
}

fn assert_symbol_summary(
    payload_len: usize,
    symbol_count: u64,
    summary: splot_core::symbol::SymbolDecoderSummary,
) {
    let total_bits = (payload_len as u64).saturating_mul(8);
    assert_eq!(summary.symbol_count, symbol_count);
    assert!(summary.consumed_bits.get() <= total_bits);
    assert!(summary.trailing_bit_position.get() < total_bits);
    assert!(summary.padding_end_position.get() <= total_bits);
    assert_eq!(summary.padding_end_position.get() % 8, 0);
    assert!(summary.trailing_bit_position <= summary.padding_end_position);
    assert_eq!(summary.consumed_bits, summary.padding_end_position);
}

fn valid_cdf_row(n: usize, cursor: &mut ByteCursor<'_>) -> Vec<i32> {
    let mut row = Vec::with_capacity(n + 1);
    for index in 0..n - 1 {
        let value = ((index + 1) * 32_768 / n) as i32;
        row.push(value.clamp(1, CDF_PROB_MAX));
    }
    row.push(i32::from(cursor.next().unwrap_or(0) % CDF_RATE_ROWS));
    row.push(i32::from(cursor.next().unwrap_or(0) % (MAX_CDF_COUNT + 1)));
    row
}

fn malformed_cdf_row(cursor: &mut ByteCursor<'_>) -> Vec<i32> {
    match cursor.next().unwrap_or(0) % 6 {
        0 => vec![1, 0],
        1 => vec![0, 0, 0],
        2 => vec![100, 100, 0, 0],
        3 => vec![
            16_384,
            i32::from(CDF_RATE_ROWS) + i32::from(cursor.next().unwrap_or(0)),
            0,
        ],
        4 => vec![16_384, 0, 33 + i32::from(cursor.next().unwrap_or(0))],
        _ => vec![-(1 + i32::from(cursor.next().unwrap_or(0))), 0, 0],
    }
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
