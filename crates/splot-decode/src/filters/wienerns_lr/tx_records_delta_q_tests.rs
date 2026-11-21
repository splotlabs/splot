// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use splot_core::span::ByteOffset;
use splot_core::symbol::{CdfUpdateMode, Symbol, SymbolDecoder, SymbolDecoderConfig};
use splot_core::symbol_encoder::SymbolEncoder;

use super::{DELTA_Q_SMALL, delta_q_is_coded_for_block, read_delta_q_abs};
use crate::bitstream::tile_payload::{FrameCdfSubset, TileCdfSelector, make_test_work_unit};

const BLOCK_64X64: usize = 12;
const BLOCK_128X128: usize = 15;

#[test]
fn skipped_full_superblock_omits_delta_q() {
    assert!(!delta_q_is_coded_for_block(BLOCK_128X128, 32, true).unwrap());
}

#[test]
fn non_skipped_or_subdivided_superblock_codes_delta_q() {
    assert!(delta_q_is_coded_for_block(BLOCK_128X128, 32, false).unwrap());
    assert!(delta_q_is_coded_for_block(BLOCK_64X64, 32, true).unwrap());
}

fn read_delta_q_abs_from(symbol: u8, escape: Option<(u32, u32)>) -> usize {
    let mut cdfs = FrameCdfSubset::from_defaults().tile_copy();
    let mut encoder = SymbolEncoder::new();
    cdfs.with_row_mut(TileCdfSelector::DeltaQ, |row| {
        encoder.write_symbol_u16(row, Symbol::new(symbol))
    })
    .unwrap()
    .unwrap();
    if let Some((rem_bits, bits)) = escape {
        encoder.write_literal(rem_bits - 1, 3).unwrap();
        encoder.write_literal(bits, rem_bits).unwrap();
    }
    let payload = encoder.finish().unwrap().into_bytes();
    let mut work_unit = make_test_work_unit(&payload, CdfUpdateMode::Enabled);
    let mut symbols = SymbolDecoder::with_base_and_config(
        &payload,
        ByteOffset::new(0),
        SymbolDecoderConfig::new().with_cdf_update_mode(CdfUpdateMode::Enabled),
    )
    .unwrap();
    read_delta_q_abs(&mut work_unit, &mut symbols, ByteOffset::new(0)).unwrap()
}

#[test]
fn delta_q_abs_escape_adds_the_rem_bits_threshold() {
    assert_eq!(read_delta_q_abs_from(3, None), 3);
    let symbol = u8::try_from(DELTA_Q_SMALL).unwrap();
    for (rem_bits, bits) in [(1, 0), (1, 1), (3, 5), (8, 255)] {
        assert_eq!(
            read_delta_q_abs_from(symbol, Some((rem_bits, bits))),
            bits as usize + (1 << rem_bits) + 5
        );
    }
}
