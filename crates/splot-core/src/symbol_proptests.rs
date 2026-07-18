// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

use super::*;
use proptest::prelude::*;

proptest! {
    /// Symbol decoder operations must never panic on bounded arbitrary payloads.
    #[test]
    fn symbol_decoder_never_panics(
        data in proptest::collection::vec(any::<u8>(), 0..64),
        literal_width in 0u32..40,
        count in 0i32..=32,
    ) {
        let mut decoder = SymbolDecoder::new(&data).unwrap();
        let _ = decoder.read_bool();
        let _ = decoder.read_literal(literal_width);
        let mut cdf = [8192, 16_384, 24_576, 0, count];
        let _ = decoder.read_symbol(&mut cdf);
        let _ = decoder.finish();
    }

    /// Symbol decoding over random valid CDF rows of every arity must keep the
    /// decoded symbol in range, preserve the valid probability range and capped
    /// count on update, be deterministic across fresh decoders, and leave the
    /// row unchanged when CDF update is disabled. It deliberately does not assert
    /// strict monotonicity after update, because §8.2.6 adaptation can drive two
    /// adjacent cumulative entries to equality.
    #[test]
    fn read_symbol_random_valid_cdf_preserves_invariants(
        n in MIN_SYMBOLS..=MAX_SYMBOLS,
        gaps in proptest::collection::vec(1i32..=4000, MAX_SYMBOLS - 1),
        rate_index in 0usize..PARA_ADJUSTMENT_LIST.len(),
        count in 0i32..=MAX_CDF_COUNT,
        data in proptest::collection::vec(any::<u8>(), 2..16),
        update_enabled in any::<bool>(),
    ) {
        let mut row = vec![0i32; n + 1];
        let mut acc = 0i32;
        for (entry, gap) in row.iter_mut().take(n - 1).zip(gaps.iter()) {
            acc += *gap; // gap >= 1 keeps the cumulative values strictly increasing
            *entry = acc; // bounded by (MAX_SYMBOLS - 1) * 4000 < CDF_PROB_MAX
        }
        row[n - 1] = rate_index as i32;
        row[n] = count;

        let config = SymbolDecoderConfig::new().with_cdf_update_mode(if update_enabled {
            CdfUpdateMode::Enabled
        } else {
            CdfUpdateMode::Disabled
        });
        let before = row.clone();

        let mut first = SymbolDecoder::with_config(&data, config).unwrap();
        let mut first_row = row.clone();
        let first_symbol = first.read_symbol(&mut first_row).unwrap();
        prop_assert!((first_symbol.get() as usize) < n);

        let mut second = SymbolDecoder::with_config(&data, config).unwrap();
        let mut second_row = row.clone();
        let second_symbol = second.read_symbol(&mut second_row).unwrap();
        prop_assert_eq!(first_symbol, second_symbol);
        prop_assert_eq!(&first_row, &second_row);

        if update_enabled {
            for value in first_row.iter().take(n - 1) {
                prop_assert!(*value >= 1 && *value <= CDF_PROB_MAX);
            }
            prop_assert!(first_row[n] <= MAX_CDF_COUNT);
        } else {
            prop_assert_eq!(&first_row, &before);
        }
    }

    /// On well-formed rows, trusted CDF validation must decode the same
    /// symbol and produce the same adapted row as full validation, for every
    /// arity; on arbitrary rows it must never panic and only ever return a
    /// typed error.
    #[test]
    fn trusted_cdf_rows_match_validated_decoding(
        n in MIN_SYMBOLS..=MAX_SYMBOLS,
        gaps in proptest::collection::vec(1i32..=4000, MAX_SYMBOLS - 1),
        rate_index in 0usize..PARA_ADJUSTMENT_LIST.len(),
        count in 0i32..=MAX_CDF_COUNT,
        data in proptest::collection::vec(any::<u8>(), 2..16),
        hostile_row in proptest::collection::vec(any::<i32>(), 0..12),
    ) {
        let mut row = vec![0i32; n + 1];
        let mut acc = 0i32;
        for (entry, gap) in row.iter_mut().take(n - 1).zip(gaps.iter()) {
            acc += *gap;
            *entry = acc;
        }
        row[n - 1] = rate_index as i32;
        row[n] = count;

        let trusted_config =
            SymbolDecoderConfig::new().with_cdf_validation_mode(CdfValidationMode::Trusted);
        let mut validated = SymbolDecoder::new(&data).unwrap();
        let mut validated_row = row.clone();
        let validated_symbol = validated.read_symbol(&mut validated_row).unwrap();

        let mut trusted = SymbolDecoder::with_config(&data, trusted_config).unwrap();
        let mut trusted_row = row.clone();
        let trusted_symbol = trusted.read_symbol(&mut trusted_row).unwrap();

        prop_assert_eq!(validated_symbol, trusted_symbol);
        prop_assert_eq!(&validated_row, &trusted_row);
        prop_assert_eq!(validated.checkpoint(), trusted.checkpoint());

        let mut hostile = SymbolDecoder::with_config(&data, trusted_config).unwrap();
        let mut hostile_cdf = hostile_row;
        match hostile.read_symbol(&mut hostile_cdf) {
            Ok(symbol) => {
                prop_assert!(usize::from(symbol.get()) < hostile_cdf.len().saturating_sub(1));
            }
            Err(
                Error::InvalidSymbolCdf { .. }
                | Error::InvalidSymbolDecoderState { .. }
                | Error::UnexpectedEof { .. },
            ) => {}
            Err(other) => prop_assert!(false, "unexpected error kind: {other:?}"),
        }
    }

    /// The windowed bypass peek must match the per-bit `bit_at` reference it
    /// replaced at every reachable width, start alignment, and end-of-payload
    /// overlap, including the zero-padding and inversion steps.
    #[test]
    fn peek_inverted_bits_matches_per_bit_reference(
        data in proptest::collection::vec(any::<u8>(), 2..16),
        advance in 0u32..=32,
        bits in 0u32..=MAX_LITERAL_BITS,
    ) {
        let mut decoder = SymbolDecoder::new(&data).unwrap();
        let _ = decoder.read_literal(advance);
        let num_bits = decoder.num_bits_to_read(bits);
        let start = decoder.reader.consumed_bits();
        let mut value = 0u64;
        for offset in 0..num_bits {
            value = (value << 1)
                | u64::from(decoder.bit_at(start + u64::from(offset)).unwrap_or(0));
        }
        let reference = (value << (bits - num_bits)) ^ mask_for_bits(bits);
        prop_assert_eq!(decoder.peek_inverted_bits(bits), reference);
    }

    /// `read_symbol` must never panic on an arbitrary caller CDF row. The
    /// relaxed `validate_cdf` now admits equal-adjacent (and thus possibly
    /// zero-width) buckets, so the "parsers never panic" property must
    /// exercise arbitrary rows — equal, strictly decreasing, out-of-range
    /// entries, and unsupported lengths — and only ever return `Ok` or a
    /// typed `Err`.
    #[test]
    fn read_symbol_never_panics_on_arbitrary_cdf_rows(
        data in proptest::collection::vec(any::<u8>(), 0..64),
        row in proptest::collection::vec(any::<i32>(), 0..12),
    ) {
        let mut decoder = SymbolDecoder::new(&data).unwrap();
        let mut cdf = row;
        match decoder.read_symbol(&mut cdf) {
            Ok(symbol) => prop_assert!(usize::from(symbol.get()) < cdf.len().saturating_sub(1)),
            Err(
                Error::InvalidSymbolCdf { .. }
                | Error::InvalidSymbolDecoderState { .. }
                | Error::UnexpectedEof { .. },
            ) => {}
            Err(other) => prop_assert!(false, "unexpected error kind: {other:?}"),
        }
    }
}
