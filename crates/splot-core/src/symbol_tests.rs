// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::*;

fn default_binary_cdf() -> [i32; 3] {
    [16_384, 0, 0]
}

/// Adapts a real generated AV2 § 9.3 default CDF row through the § 8.2.6
/// update step (always decoding symbol 0, so every cumulative entry is
/// incremented toward `1 << 15`) until two adjacent cumulative entries become
/// equal, re-deriving the CDF shape each step exactly as `read_symbol` does.
/// Returns the adapted row and its arity; the caller asserts that the
/// equal-adjacent state was actually reached.
fn adapt_default_row_to_equal_adjacent() -> ([i32; 8], usize) {
    let mut cdf = crate::tables::cdf::DEFAULT_CCTX_TYPE_CDF.map(i32::from);
    let n = cdf.len() - 1;
    let mut steps = 0;
    while steps < 512 {
        let shape = CdfShape {
            n,
            rate_index: cdf[n - 1] as usize,
            count: cdf[n],
        };
        update_cdf(&mut cdf, shape, 0);
        steps += 1;
        if (1..n - 1).any(|i| cdf[i] == cdf[i - 1]) {
            break;
        }
    }
    (cdf, n)
}

#[test]
fn init_symbol_tracks_boundary_sizes() {
    let empty = SymbolDecoder::new(&[]).unwrap();
    assert_eq!(empty.consumed_bits().get(), 0);
    assert_eq!(empty.symbol_max_bits(), -15);

    let one = SymbolDecoder::new(&[0x80]).unwrap();
    assert_eq!(one.consumed_bits().get(), 8);
    assert_eq!(one.symbol_max_bits(), -7);

    let two = SymbolDecoder::new(&[0x80, 0x00]).unwrap();
    assert_eq!(two.consumed_bits().get(), 15);
    assert_eq!(two.symbol_max_bits(), 1);
}

#[test]
fn finish_validates_trailing_one_and_padding() {
    let one = SymbolDecoder::new(&[0x80]).unwrap();
    let summary = one.finish().unwrap();
    assert_eq!(summary.consumed_bits.get(), 8);
    assert_eq!(summary.trailing_bit_position.get(), 0);
    assert_eq!(summary.padding_end_position.get(), 8);

    let two = SymbolDecoder::new(&[0x80, 0x00]).unwrap();
    let summary = two.finish().unwrap();
    assert_eq!(summary.consumed_bits.get(), 16);
    assert_eq!(summary.trailing_bit_position.get(), 0);
    assert_eq!(summary.padding_end_position.get(), 16);
}

#[test]
fn finish_rejects_empty_payload_and_bad_padding() {
    let empty = SymbolDecoder::new(&[]).unwrap();
    assert!(matches!(
        empty.finish(),
        Err(Error::InvalidSymbolDecoderState {
            kind: SymbolDecoderErrorKind::SymbolMaxBitsTooSmall {
                symbol_max_bits: -15
            },
            ..
        })
    ));

    let missing_one = SymbolDecoder::new(&[0x00]).unwrap();
    assert!(matches!(
        missing_one.finish(),
        Err(Error::InvalidSymbolDecoderState {
            kind: SymbolDecoderErrorKind::MissingTrailingOneBit,
            ..
        })
    ));

    let nonzero_padding = SymbolDecoder::new(&[0xA0]).unwrap();
    assert!(matches!(
        nonzero_padding.finish(),
        Err(Error::InvalidSymbolDecoderState {
            kind: SymbolDecoderErrorKind::NonZeroPaddingBit,
            ..
        })
    ));
}

#[test]
fn finish_accepts_symbol_max_bits_minus_fourteen() {
    let mut decoder = SymbolDecoder::new(&[0x81]).unwrap();
    for _ in 0..7 {
        let _ = decoder.read_bool().unwrap();
    }
    assert_eq!(decoder.symbol_max_bits(), -14);
    let summary = decoder.finish().unwrap();
    assert_eq!(summary.trailing_bit_position.get(), 7);
}

#[test]
fn read_bool_and_literal_return_pseudo_raw_bits() {
    let mut bool_decoder = SymbolDecoder::new(&[0b1000_0000, 0]).unwrap();
    assert!(bool_decoder.read_bool().unwrap());

    let mut literal_decoder = SymbolDecoder::new(&[0b1011_0000, 0]).unwrap();
    assert_eq!(literal_decoder.read_literal(4).unwrap(), 0b1011);
    assert_eq!(literal_decoder.symbol_count(), 4);
}

#[test]
fn checkpoint_preserves_arithmetic_state() {
    let mut decoder = SymbolDecoder::new(&[0x00, 0x80]).unwrap();
    let initial = decoder.checkpoint();
    assert_eq!(initial.consumed_bits, decoder.consumed_bits());
    assert_eq!(initial.symbol_count, 0);
    assert_eq!(initial.symbol_max_bits, decoder.symbol_max_bits());
    assert_eq!(initial.symbol_value, decoder.symbol_value());
    assert_eq!(initial.symbol_range, SYMBOL_RANGE_INIT);

    let mut cdf = default_binary_cdf();
    decoder.read_symbol(&mut cdf).unwrap();
    let checkpoint = decoder.checkpoint();

    assert_eq!(checkpoint.consumed_bits, decoder.consumed_bits());
    assert_eq!(checkpoint.symbol_count, 1);
    assert_eq!(checkpoint.symbol_max_bits, decoder.symbol_max_bits());
    assert_eq!(checkpoint.symbol_value, decoder.symbol_value());
    assert_eq!(checkpoint.symbol_range, decoder.symbol_range);
    assert_ne!(checkpoint, initial);
}

#[test]
fn read_literal_rejects_wide_width() {
    let mut decoder = SymbolDecoder::new(&[0x80]).unwrap();
    assert!(matches!(
        decoder.read_literal(33),
        Err(Error::InvalidSymbolDecoderState {
            kind: SymbolDecoderErrorKind::LiteralWidthTooLarge {
                requested: 33,
                max: 32
            },
            ..
        })
    ));
    assert_eq!(decoder.symbol_count(), 0);
}

#[test]
fn read_unary_matches_literal_bit_loop_state() {
    let payloads = [
        [0xA7, 0x39, 0xC1, 0x5E, 0x82, 0x44, 0x19, 0xD0],
        [0x12, 0xF4, 0x67, 0x88, 0x9A, 0xBC, 0xDE, 0xF0],
        [0xFF, 0x00, 0x81, 0x7E, 0x42, 0x24, 0x18, 0x80],
    ];
    let widths = [1, 5, 6, 21, 32];

    for payload in payloads {
        for max_bits in widths {
            let mut unary = SymbolDecoder::new(&payload).unwrap();
            let mut literal = SymbolDecoder::new(&payload).unwrap();
            prime_unary_comparison_decoder(&mut unary);
            prime_unary_comparison_decoder(&mut literal);

            let unary_value = unary.read_unary(max_bits).unwrap();
            let literal_value = read_unary_as_literal_bits(&mut literal, max_bits);

            assert_eq!(unary_value, literal_value, "max_bits={max_bits}");
            assert_eq!(
                unary.checkpoint(),
                literal.checkpoint(),
                "max_bits={max_bits}"
            );
        }
    }
}

#[test]
fn read_literal_chunks_match_one_bit_loop_state() {
    let payloads = [
        [0xA7, 0x39, 0xC1, 0x5E, 0x82, 0x44, 0x19, 0xD0],
        [0x12, 0xF4, 0x67, 0x88, 0x9A, 0xBC, 0xDE, 0xF0],
        [0xFF, 0x00, 0x81, 0x7E, 0x42, 0x24, 0x18, 0x80],
    ];
    let widths = [2, 3, 6, 7, 8, 9, 16, 21, 32];

    for payload in payloads {
        for width in widths {
            let mut chunked = SymbolDecoder::new(&payload).unwrap();
            let mut bit_loop = SymbolDecoder::new(&payload).unwrap();
            prime_unary_comparison_decoder(&mut chunked);
            prime_unary_comparison_decoder(&mut bit_loop);

            let chunked_value = chunked.read_literal(width).unwrap();
            let loop_value = read_literal_as_one_bit_chunks(&mut bit_loop, width);

            assert_eq!(chunked_value, loop_value, "width={width}");
            assert_eq!(chunked.checkpoint(), bit_loop.checkpoint(), "width={width}");
        }
    }
}

fn prime_unary_comparison_decoder(decoder: &mut SymbolDecoder<'_>) {
    let mut cdf = [8192, 16_384, 24_576, 0, 0];
    let _ = decoder.read_symbol(&mut cdf).unwrap();
    let _ = decoder.read_literal(3).unwrap();
    let _ = decoder.read_bool().unwrap();
}

fn read_unary_as_literal_bits(decoder: &mut SymbolDecoder<'_>, max_bits: u32) -> u32 {
    let mut value = 0;
    for _ in 0..max_bits {
        if decoder.read_literal(1).unwrap() == 0 {
            value += 1;
        } else {
            break;
        }
    }
    value
}

fn read_literal_as_one_bit_chunks(decoder: &mut SymbolDecoder<'_>, width: u32) -> u32 {
    let mut value = 0;
    for _ in 0..width {
        value = (value << 1) | decoder.read_literal(1).unwrap();
    }
    value
}

#[test]
fn ec_prob_shift_matches_av2_constant() {
    assert_eq!(EC_PROB_SHIFT, 7);
}

#[test]
fn read_symbol_decodes_multiarity_threshold_vectors() {
    let config = SymbolDecoderConfig::new().with_cdf_update_mode(CdfUpdateMode::Disabled);
    let cdf = [8192, 16_384, 24_576, 0, 0];
    let cases = [
        ([0x00, 0x00], 0),
        ([0x63, 0xBE], 1),
        ([0xB1, 0xDE], 2),
        ([0xFF, 0xFF], 3),
    ];

    for (data, expected) in cases {
        let mut decoder = SymbolDecoder::with_config(&data, config).unwrap();
        let mut row = cdf;
        assert_eq!(decoder.read_symbol(&mut row).unwrap().get(), expected);
        assert_eq!(row, cdf);
    }
}

#[test]
fn read_symbol_decodes_binary_row_and_updates_cdf() {
    let mut decoder = SymbolDecoder::new(&[0x00, 0x80]).unwrap();
    let mut cdf = default_binary_cdf();
    let symbol = decoder.read_symbol(&mut cdf).unwrap();
    assert_eq!(symbol.get(), 0);
    assert_eq!(decoder.symbol_count(), 1);
    assert_eq!(cdf[2], 1);
    assert!(cdf[0] > 16_384);
}

#[test]
fn read_symbol_updates_last_symbol_multiarity_vector() {
    let mut decoder = SymbolDecoder::new(&[0xFF, 0xFF]).unwrap();
    let mut cdf = [8192, 16_384, 24_576, 0, 0];
    let symbol = decoder.read_symbol(&mut cdf).unwrap();
    assert_eq!(symbol.get(), 3);
    assert_eq!(cdf, [7936, 15_872, 23_808, 0, 1]);
}

#[test]
fn read_symbol_can_disable_cdf_update() {
    let config = SymbolDecoderConfig::new().with_cdf_update_mode(CdfUpdateMode::Disabled);
    let mut decoder = SymbolDecoder::with_config(&[0x80, 0x00], config).unwrap();
    let mut cdf = default_binary_cdf();
    let before = cdf;
    let symbol = decoder.read_symbol(&mut cdf).unwrap();
    assert_eq!(symbol.get(), 1);
    assert_eq!(cdf, before);
}

#[test]
fn read_symbol_caps_cdf_count_at_thirty_two() {
    let mut decoder = SymbolDecoder::new(&[0x00, 0x80]).unwrap();
    let mut cdf = [16_384, 0, 32];
    let _ = decoder.read_symbol(&mut cdf).unwrap();
    assert_eq!(cdf[2], 32);
}

#[test]
fn cdf_update_count_intervals_and_nonzero_rate_rows_are_exact() {
    let cases = [
        (0, [7936, 16_896, 24_832, 6, 1]),
        (16, [7936, 16_896, 24_832, 6, 17]),
        (32, [8064, 16_640, 24_704, 6, 32]),
    ];

    for (count, expected) in cases {
        let mut cdf = [8192, 16_384, 24_576, 6, count];
        update_cdf(
            &mut cdf,
            CdfShape {
                n: 4,
                rate_index: 6,
                count,
            },
            1,
        );
        assert_eq!(cdf, expected);
    }
}

#[test]
fn invalid_cdf_rows_are_rejected_before_mutation() {
    let cases: [(&[i32], SymbolCdfErrorKind); 5] = [
        (&[1, 0], SymbolCdfErrorKind::UnsupportedLength { len: 2 }),
        (
            &[-1, 0, 0],
            SymbolCdfErrorKind::ProbabilityOutOfRange {
                index: 0,
                value: -1,
            },
        ),
        (
            &[100, 99, 0, 0],
            SymbolCdfErrorKind::DecreasingCumulative {
                previous_index: 0,
                index: 1,
            },
        ),
        (
            &[16_384, 125, 0],
            SymbolCdfErrorKind::AdaptationRateOutOfRange {
                index: 1,
                value: 125,
            },
        ),
        (
            &[16_384, 0, 33],
            SymbolCdfErrorKind::CountOutOfRange {
                index: 2,
                value: 33,
            },
        ),
    ];

    for (row, expected_kind) in cases {
        let mut decoder = SymbolDecoder::new(&[0x00, 0x80]).unwrap();
        let mut cdf = row.to_vec();
        let before = cdf.clone();
        assert!(matches!(
            decoder.read_symbol(&mut cdf),
            Err(Error::InvalidSymbolCdf { kind, .. }) if kind == expected_kind
        ));
        assert_eq!(cdf, before);
        assert_eq!(decoder.symbol_count(), 0);
    }
}

#[test]
fn read_symbol_extreme_values_select_first_and_last_symbol_for_all_arities() {
    let config = SymbolDecoderConfig::new().with_cdf_update_mode(CdfUpdateMode::Disabled);
    for n in MIN_SYMBOLS..=MAX_SYMBOLS {
        let step = CDF_PROB_SCALE as i32 / n as i32;
        let mut row = vec![0i32; n + 1];
        for (i, entry) in row.iter_mut().take(n - 1).enumerate() {
            *entry = step * (i as i32 + 1);
        }

        let mut first = SymbolDecoder::with_config(&[0x00, 0x00], config).unwrap();
        let mut first_row = row.clone();
        assert_eq!(
            first.read_symbol(&mut first_row).unwrap().get(),
            0,
            "maximal SymbolValue must select symbol 0 (N={n})"
        );
        assert_eq!(
            first_row, row,
            "disabled update must not mutate the row (N={n})"
        );

        let mut last = SymbolDecoder::with_config(&[0xFF, 0xFF], config).unwrap();
        let mut last_row = row.clone();
        assert_eq!(
            last.read_symbol(&mut last_row).unwrap().get() as usize,
            n - 1,
            "zero SymbolValue must select the last symbol (N={n})"
        );
        assert_eq!(last_row, row);
    }
}

#[test]
fn adaptation_can_equalize_adjacent_cumulative_entries() {
    let (cdf, n) = adapt_default_row_to_equal_adjacent();
    assert!(
        (1..n - 1).any(|i| cdf[i] == cdf[i - 1]),
        "adaptation from a default § 9.3 row should equalize adjacent cumulative entries: {cdf:?}"
    );
    assert!(
        cdf[..n - 1]
            .iter()
            .all(|&v| (1..=CDF_PROB_MAX).contains(&v))
    );
}

#[test]
fn read_symbol_accepts_and_decodes_equal_adjacent_cumulative_entries() {
    let config = SymbolDecoderConfig::new().with_cdf_update_mode(CdfUpdateMode::Disabled);
    let row = [16_384, 16_384, 24_576, 0, 0];
    let cases = [
        ([0x00u8, 0x00], 0u8),
        ([0x7F, 0x40], 1),
        ([0x7F, 0x80], 2),
        ([0xBF, 0xC0], 3),
    ];

    for (data, expected) in cases {
        let mut decoder = SymbolDecoder::with_config(&data, config).unwrap();
        let mut cdf = row;
        let symbol = decoder.read_symbol(&mut cdf).unwrap();
        assert_eq!(symbol.get(), expected, "payload {data:02X?}");
        assert_eq!(cdf, row, "disabled update must not mutate the row");
        assert_eq!(decoder.symbol_count(), 1);
    }
}

#[test]
fn update_cdf_rate_extremes_are_exact() {
    let mut min_rate = [16_384, 50, 0];
    update_cdf(
        &mut min_rate,
        CdfShape {
            n: 2,
            rate_index: 50,
            count: 0,
        },
        1,
    );
    assert_eq!(min_rate, [12_288, 50, 1]);

    let mut low = [8192, 16_384, 24_576, 50, 0];
    update_cdf(
        &mut low,
        CdfShape {
            n: 4,
            rate_index: 50,
            count: 0,
        },
        1,
    );
    assert_eq!(low, [7168, 18_432, 25_600, 50, 1]);

    let mut high = [8192, 16_384, 24_576, 3, 32];
    update_cdf(
        &mut high,
        CdfShape {
            n: 4,
            rate_index: 3,
            count: 32,
        },
        1,
    );
    assert_eq!(high, [8160, 16_448, 24_608, 3, 32]);
}

#[test]
fn deep_negative_symbol_max_bits_pads_deterministically() {
    let decode_run = || {
        let mut decoder = SymbolDecoder::new(&[0x5A, 0xC3]).unwrap();
        let mut row = [10_922, 21_844, 0, 0]; // N=3, rate index 0, count 0
        let mut symbols = Vec::with_capacity(20);
        for _ in 0..20 {
            let symbol = decoder.read_symbol(&mut row).unwrap();
            assert!((symbol.get() as usize) < 3);
            symbols.push(symbol.get());
        }
        (symbols, decoder.symbol_max_bits())
    };
    let (first, smb_first) = decode_run();
    let (second, smb_second) = decode_run();
    assert_eq!(first, second, "padded decoding must be deterministic");
    assert_eq!(smb_first, smb_second);
    assert!(
        smb_first < 0,
        "20 symbol reads must drive SymbolMaxBits negative, got {smb_first}"
    );
}

#[test]
fn adapted_row_with_equal_adjacent_entries_is_accepted_and_decodes() {
    let (mut cdf, n) = adapt_default_row_to_equal_adjacent();
    assert!(
        (1..n - 1).any(|i| cdf[i] == cdf[i - 1]),
        "expected an equalized adapted row: {cdf:?}"
    );

    let config = SymbolDecoderConfig::new().with_cdf_update_mode(CdfUpdateMode::Disabled);
    let mut decoder = SymbolDecoder::with_config(&[0xFF, 0xFF], config).unwrap();
    let symbol = decoder.read_symbol(&mut cdf).unwrap();
    assert!(usize::from(symbol.get()) < n);
    assert_eq!(decoder.symbol_count(), 1);
}

#[test]
fn trusted_mode_survives_reviewer_reproducer_rows() {
    let trusted = SymbolDecoderConfig::new().with_cdf_validation_mode(CdfValidationMode::Trusted);
    let mut over = [100_000, 0, 5];
    let mut decoder = SymbolDecoder::with_config(&[0xAA, 0x55], trusted).unwrap();
    let _ = decoder.read_symbol(&mut over);
    let mut negative = [-1, 0, 0];
    let mut decoder = SymbolDecoder::with_config(&[0xAA, 0x55], trusted).unwrap();
    let _ = decoder.read_symbol(&mut negative);
    let mut extreme = [i32::MIN, i32::MAX, 3, i32::MAX];
    let mut decoder = SymbolDecoder::with_config(&[0xAA, 0x55], trusted).unwrap();
    let _ = decoder.read_symbol(&mut extreme);
}
