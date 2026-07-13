// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::unwrap_used)]

use splot_core::span::ByteOffset;
use splot_core::symbol::{CdfUpdateMode, SymbolDecoderConfig};
use splot_core::symbol_encoder::SymbolEncoder;

use super::*;

fn symbol_decoder(payload: &[u8]) -> SymbolDecoder<'_> {
    SymbolDecoder::with_base_and_config(
        payload,
        ByteOffset::new(0),
        SymbolDecoderConfig::new().with_cdf_update_mode(CdfUpdateMode::Enabled),
    )
    .unwrap()
}

fn encode_extended(
    enc: &mut SymbolEncoder,
    q: u32,
    c_max: u32,
    golomb_prefix: u32,
    length: u32,
    coeff_rem: u32,
) {
    enc.write_unary(q, c_max).unwrap();
    if q == c_max {
        enc.write_unary(golomb_prefix, MAX_EXP_GOLOMB_PREFIX_BITS)
            .unwrap();
    }
    enc.write_literal(coeff_rem, length).unwrap();
}

const WALK_ENTRIES: [CoeffScanEntry; 4] = [
    CoeffScanEntry::new(3, 9, 1, 1),
    CoeffScanEntry::new(2, 1, 0, 1),
    CoeffScanEntry::new(1, 8, 1, 0),
    CoeffScanEntry::new(0, 0, 0, 0),
];

fn input(entry: CoeffScanEntry, level: u32, max_level: u32) -> CoeffReadQuantInput {
    CoeffReadQuantInput {
        entry,
        level,
        max_level,
    }
}

fn config(hr_level_avg: u32) -> CoeffReadQuantConfig {
    CoeffReadQuantConfig {
        is_hidden: false,
        allow_tcq: false,
        hr_level_avg,
    }
}

#[test]
fn read_quant_below_threshold_consumes_no_bits() {
    let entries = WALK_ENTRIES;
    let mut symbols = symbol_decoder(&[0x80]);
    let consumed_before = symbols.consumed_bits();
    let symbol_count_before = symbols.symbol_count();
    let inputs = [input(entries[0], 2, 5)];
    let walk = NonZeroCoeffScanWalk::from_entries_for_test(vec![entries[0]]);

    let reads = read_nonzero_coeff_quants(&mut symbols, &walk, &inputs, config(7)).unwrap();

    assert_eq!(reads.len(), 1);
    assert_eq!(reads[0].path(), CoeffReadQuantPath::BelowThreshold);
    assert_eq!(reads[0].quant_input().quant, 2);
    assert_eq!(reads[0].quant_input().hr_level_avg, 7);
    assert_eq!(symbols.consumed_bits(), consumed_before);
    assert_eq!(symbols.symbol_count(), symbol_count_before);
}

#[test]
fn read_quant_finite_q_length_updates_quant_and_hr_average() {
    let entries = WALK_ENTRIES;
    let mut symbols = symbol_decoder(&[0b0011_0100, 0x80]);
    let inputs = [input(entries[0], 3, 3)];
    let walk = NonZeroCoeffScanWalk::from_entries_for_test(vec![entries[0]]);

    let reads = read_nonzero_coeff_quants(&mut symbols, &walk, &inputs, config(16)).unwrap();

    assert_eq!(reads[0].quant_input().quant, 45);
    assert_eq!(reads[0].quant_input().hr_level_avg, 29);
    assert_eq!(symbols.symbol_count(), 7);
    assert_eq!(
        reads[0].path(),
        CoeffReadQuantPath::Extended {
            m: 4,
            k: 5,
            c_max: 6,
            q: 2,
            length: 4,
            x_base: 32,
            coeff_rem: 10,
            x: 42,
        }
    );
}

#[test]
fn read_quant_golomb_extension_path_reads_until_terminator() {
    let entries = WALK_ENTRIES;
    let mut symbols = symbol_decoder(&[0x03, 0x40, 0x80]);
    let inputs = [input(entries[0], 2, 2)];
    let walk = NonZeroCoeffScanWalk::from_entries_for_test(vec![entries[0]]);

    let reads = read_nonzero_coeff_quants(&mut symbols, &walk, &inputs, config(1)).unwrap();

    assert_eq!(reads[0].quant_input().quant, 21);
    assert_eq!(reads[0].quant_input().hr_level_avg, 10);
    assert_eq!(symbols.symbol_count(), 10);
    assert_eq!(
        reads[0].path(),
        CoeffReadQuantPath::Extended {
            m: 1,
            k: 2,
            c_max: 5,
            q: 5,
            length: 3,
            x_base: 14,
            coeff_rem: 5,
            x: 19,
        }
    );
}

#[test]
fn read_quant_hidden_dc_and_tcq_adjust_predicted_extension() {
    let entries = WALK_ENTRIES;
    let hidden_dc = entries[3];
    let mut symbols = symbol_decoder(&[0b1000_0100, 0x80]);
    let inputs = [input(hidden_dc, 2, 3)];
    let walk = NonZeroCoeffScanWalk::from_entries_for_test(vec![hidden_dc]);
    let config = CoeffReadQuantConfig {
        is_hidden: true,
        allow_tcq: true,
        hr_level_avg: 64,
    };

    let reads = read_nonzero_coeff_quants(&mut symbols, &walk, &inputs, config).unwrap();

    assert_eq!(reads[0].quant_input().quant, 4);
    assert_eq!(reads[0].quant_input().hr_level_avg, 33);
    assert_eq!(symbols.symbol_count(), 6);
    assert_eq!(
        reads[0].path(),
        CoeffReadQuantPath::Extended {
            m: 5,
            k: 6,
            c_max: 6,
            q: 0,
            length: 5,
            x_base: 0,
            coeff_rem: 1,
            x: 1,
        }
    );
}

#[test]
fn read_quant_rejects_input_mismatch_before_consumption() {
    let entries = WALK_ENTRIES;
    let mut symbols = symbol_decoder(&[0xff, 0x80]);
    let consumed_before = symbols.consumed_bits();
    let symbol_count_before = symbols.symbol_count();
    let inputs = [input(entries[1], 3, 3)];
    let walk = NonZeroCoeffScanWalk::from_entries_for_test(vec![entries[0]]);

    let err = read_nonzero_coeff_quants(&mut symbols, &walk, &inputs, config(1)).unwrap_err();

    assert!(matches!(
        err,
        CoeffReadQuantError::ScanEntryMismatch { index: 0, .. }
    ));
    assert_eq!(symbols.consumed_bits(), consumed_before);
    assert_eq!(symbols.symbol_count(), symbol_count_before);
}

#[test]
fn read_quant_rejects_unterminated_golomb_prefix() {
    let entries = WALK_ENTRIES;
    let mut symbols = symbol_decoder(&[]);
    let inputs = [input(entries[0], 3, 3)];
    let walk = NonZeroCoeffScanWalk::from_entries_for_test(vec![entries[0]]);

    let err = read_nonzero_coeff_quants(&mut symbols, &walk, &inputs, config(1)).unwrap_err();

    assert!(matches!(
        err,
        CoeffReadQuantError::QuantOverflow {
            operation: "coeff_rem literal width",
            ..
        }
    ));
}

#[test]
fn read_quant_rejects_pathological_max_level_and_overflow() {
    let entries = WALK_ENTRIES;
    let mut invalid_symbols = symbol_decoder(&[0xff, 0x80]);
    let inputs = [input(entries[0], 0, 0)];
    let walk_one = NonZeroCoeffScanWalk::from_entries_for_test(vec![entries[0]]);
    let invalid = CoeffReadQuantConfig {
        is_hidden: false,
        allow_tcq: true,
        hr_level_avg: 1,
    };

    let err =
        read_nonzero_coeff_quants(&mut invalid_symbols, &walk_one, &inputs, invalid).unwrap_err();

    assert!(matches!(
        err,
        CoeffReadQuantError::InvalidMaxLevel {
            index: 0,
            max_level: 0,
            allow_tcq: true,
        }
    ));

    let mut overflow_symbols = symbol_decoder(&[0b1100_0000, 0x80]);
    let overflow_inputs = [input(entries[0], u32::MAX, u32::MAX)];
    let err = read_nonzero_coeff_quants(
        &mut overflow_symbols,
        &walk_one,
        &overflow_inputs,
        config(1),
    )
    .unwrap_err();

    assert!(matches!(
        err,
        CoeffReadQuantError::QuantOverflow {
            operation: "quant + x << allowTcq",
            ..
        }
    ));
}

#[test]
fn read_quant_rejects_oversized_golomb_remainder_width() {
    let entries = WALK_ENTRIES;
    let mut symbols = symbol_decoder(&[0x00, 0x00, 0x00, 0x00, 0x08, 0x80]);
    let inputs = [input(entries[0], 2, 2)];
    let walk = NonZeroCoeffScanWalk::from_entries_for_test(vec![entries[0]]);

    let err = read_nonzero_coeff_quants(&mut symbols, &walk, &inputs, config(1)).unwrap_err();

    assert!(matches!(
        err,
        CoeffReadQuantError::QuantOverflow {
            operation: "coeff_rem literal width",
            ..
        }
    ));
}

#[test]
fn read_quant_finite_q_roundtrips_through_symbol_encoder() {
    let entry = CoeffScanEntry::new(3, 9, 1, 1);
    let (m, k, c_max) = (4u32, 5u32, 6u32);
    let (q, coeff_rem) = (2u32, 10u32);
    let length = m;
    let x_base = q << m;
    let x = x_base + coeff_rem;
    let level = 3u32;
    let expected_quant = level + x;

    let mut enc = SymbolEncoder::new();
    encode_extended(&mut enc, q, c_max, 0, length, coeff_rem);
    let bytes = enc.finish().unwrap().into_bytes();

    let mut symbols = symbol_decoder(&bytes);
    let walk = NonZeroCoeffScanWalk::from_entries_for_test(vec![entry]);
    let inputs = [input(entry, level, level)];
    let reads = read_nonzero_coeff_quants(&mut symbols, &walk, &inputs, config(16)).unwrap();

    assert_eq!(reads[0].quant_input().quant, expected_quant);
    assert_eq!(
        reads[0].path(),
        CoeffReadQuantPath::Extended {
            m,
            k,
            c_max,
            q,
            length,
            x_base,
            coeff_rem,
            x,
        }
    );
}

#[test]
fn read_quant_golomb_extension_roundtrips_through_symbol_encoder() {
    let entry = CoeffScanEntry::new(3, 9, 1, 1);
    let (m, k, c_max) = (1u32, 2u32, 5u32);
    let (golomb_prefix, coeff_rem) = (1u32, 5u32);
    let q = c_max;
    let length = golomb_prefix + k;
    let x_base = (q << m) + ((1 << length) - (1 << k));
    let x = x_base + coeff_rem;
    let level = 2u32;
    let expected_quant = level + x;

    let mut enc = SymbolEncoder::new();
    encode_extended(&mut enc, q, c_max, golomb_prefix, length, coeff_rem);
    let bytes = enc.finish().unwrap().into_bytes();

    let mut symbols = symbol_decoder(&bytes);
    let walk = NonZeroCoeffScanWalk::from_entries_for_test(vec![entry]);
    let inputs = [input(entry, level, level)];
    let reads = read_nonzero_coeff_quants(&mut symbols, &walk, &inputs, config(1)).unwrap();

    assert_eq!(reads[0].quant_input().quant, expected_quant);
    assert_eq!(
        reads[0].path(),
        CoeffReadQuantPath::Extended {
            m,
            k,
            c_max,
            q,
            length,
            x_base,
            coeff_rem,
            x,
        }
    );
}

#[test]
fn read_quant_finite_q_roundtrips_across_parameter_grid() {
    let entry = CoeffScanEntry::new(3, 9, 1, 1);
    let (m, k, c_max) = (4u32, 5u32, 6u32);
    let level = 4u32;
    let mut cases = 0u32;
    for q in 0..c_max {
        for coeff_rem in [0u32, 1, 7, 15] {
            let length = m;
            let x_base = q << m;
            let x = x_base + coeff_rem;
            let expected_quant = level + x;

            let mut enc = SymbolEncoder::new();
            encode_extended(&mut enc, q, c_max, 0, length, coeff_rem);
            let bytes = enc.finish().unwrap().into_bytes();

            let mut symbols = symbol_decoder(&bytes);
            let walk = NonZeroCoeffScanWalk::from_entries_for_test(vec![entry]);
            let inputs = [input(entry, level, level)];
            let reads =
                read_nonzero_coeff_quants(&mut symbols, &walk, &inputs, config(16)).unwrap();

            assert_eq!(
                reads[0].quant_input().quant,
                expected_quant,
                "q={q} coeff_rem={coeff_rem}"
            );
            assert_eq!(
                reads[0].path(),
                CoeffReadQuantPath::Extended {
                    m,
                    k,
                    c_max,
                    q,
                    length,
                    x_base,
                    coeff_rem,
                    x,
                }
            );
            cases += 1;
        }
    }
    assert_eq!(cases, c_max * 4);
}

#[allow(clippy::many_single_char_names)]
#[test]
fn read_quant_multi_coeff_roundtrips_with_state_carry() {
    let a = CoeffScanEntry::new(1, 8, 1, 0);
    let b = CoeffScanEntry::new(0, 0, 0, 0);
    let (m, c_max) = (4u32, 6u32);
    let (q, coeff_rem) = (1u32, 3u32);
    let length = m;
    let x = (q << m) + coeff_rem;
    let level_a = 4u32;
    let quant_a = level_a + x;
    let level_b = 1u32;
    let max_b = 5u32;

    let mut enc = SymbolEncoder::new();
    encode_extended(&mut enc, q, c_max, 0, length, coeff_rem);
    let bytes = enc.finish().unwrap().into_bytes();

    let mut symbols = symbol_decoder(&bytes);
    let walk = NonZeroCoeffScanWalk::from_entries_for_test(vec![a, b]);
    let inputs = [input(a, level_a, level_a), input(b, level_b, max_b)];
    let reads = read_nonzero_coeff_quants(&mut symbols, &walk, &inputs, config(16)).unwrap();

    assert_eq!(reads[0].quant_input().quant, quant_a);
    assert_eq!(reads[1].quant_input().quant, level_b);
    assert_eq!(reads[1].path(), CoeffReadQuantPath::BelowThreshold);
}
