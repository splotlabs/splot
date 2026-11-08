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

    /// The vector adaptation of compact `u16` rows must match the scalar
    /// `i32` adaptation for every arity and every entry up to
    /// `CDF_PROB_SCALE`, including the 0 and `CDF_PROB_SCALE` extremes.
    #[test]
    fn compact_rows_decode_and_adapt_like_i32_rows(
        n in MIN_SYMBOLS..=MAX_SYMBOLS,
        mut probs in proptest::collection::vec(0i32..=CDF_PROB_SCALE as i32, MAX_SYMBOLS - 1),
        rate_index in 0usize..PARA_ADJUSTMENT_LIST.len(),
        count in 0i32..=MAX_CDF_COUNT,
        data in proptest::collection::vec(any::<u8>(), 2..16),
    ) {
        probs[..n - 1].sort_unstable();
        let mut row = vec![0i32; n + 1];
        row[..n - 1].copy_from_slice(&probs[..n - 1]);
        row[n - 1] = rate_index as i32;
        row[n] = count;
        let mut compact: Vec<u16> = row.iter().map(|&value| value as u16).collect();
        let trusted = SymbolDecoderConfig::new().with_cdf_validation_mode(CdfValidationMode::Trusted);

        let mut wide = SymbolDecoder::with_config(&data, trusted).unwrap();
        let wide_symbol = wide.read_symbol(&mut row);
        let mut narrow = SymbolDecoder::with_config(&data, trusted).unwrap();
        let narrow_symbol = narrow.read_symbol_u16(&mut compact);

        prop_assert_eq!(wide_symbol.is_ok(), narrow_symbol.is_ok());
        if let (Ok(wide_symbol), Ok(narrow_symbol)) = (wide_symbol, narrow_symbol) {
            prop_assert_eq!(wide_symbol, narrow_symbol);
            let widened: Vec<i32> = compact.iter().map(|&value| i32::from(value)).collect();
            prop_assert_eq!(widened, row);
        }
    }

    /// On well-formed rows, trusted CDF validation must decode the same
    /// symbol and produce the same adapted row as full validation, for every
    /// arity.
    #[test]
    fn trusted_cdf_rows_match_validated_decoding(
        n in MIN_SYMBOLS..=MAX_SYMBOLS,
        gaps in proptest::collection::vec(1i32..=4000, MAX_SYMBOLS - 1),
        rate_index in 0usize..PARA_ADJUSTMENT_LIST.len(),
        count in 0i32..=MAX_CDF_COUNT,
        data in proptest::collection::vec(any::<u8>(), 2..16),
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
    }

    /// `read_trusted` and, in trusted validation mode, `read_trusted_row` must
    /// decode, adapt and advance exactly like the checked read on well-formed
    /// rows of every arity, in both update modes, over a run of reads that
    /// keeps adapting the same row.
    #[test]
    fn read_trusted_matches_checked_reads(
        n in MIN_SYMBOLS..=MAX_SYMBOLS,
        gaps in proptest::collection::vec(1u16..=4000, MAX_SYMBOLS - 1),
        rate_index in 0u16..PARA_ADJUSTMENT_LIST.len() as u16,
        count in 0u16..=MAX_CDF_COUNT as u16,
        update in any::<bool>(),
        data in proptest::collection::vec(any::<u8>(), 2..24),
    ) {
        let mut row = [0u16; MAX_SYMBOLS + 1];
        let mut acc = 0u16;
        for (entry, gap) in row.iter_mut().take(n - 1).zip(&gaps) {
            acc += gap;
            *entry = acc;
        }
        row[n - 1] = rate_index;
        row[n] = count;
        let mode = if update { CdfUpdateMode::Enabled } else { CdfUpdateMode::Disabled };
        let config = SymbolDecoderConfig::new().with_cdf_update_mode(mode);
        let mut checked = SymbolDecoder::with_config(&data, config).unwrap();
        let mut trusted = SymbolDecoder::with_config(&data, config).unwrap();
        let row_config = config.with_cdf_validation_mode(CdfValidationMode::Trusted);
        let mut row_reader = SymbolDecoder::with_config(&data, row_config).unwrap();
        let (mut checked_row, mut trusted_row, mut sliced_row) = (row, row, row);
        prop_assert_eq!(checked.read_trusted_row(&mut sliced_row[..=n]), None);
        for _ in 0..12 {
            let expected = checked.read_symbol_u16(&mut checked_row[..=n]).unwrap();
            let got = match (n, update) {
                (2, true) => trusted_read::<3, true>(&mut trusted, &mut trusted_row),
                (2, false) => trusted_read::<3, false>(&mut trusted, &mut trusted_row),
                (3, true) => trusted_read::<4, true>(&mut trusted, &mut trusted_row),
                (3, false) => trusted_read::<4, false>(&mut trusted, &mut trusted_row),
                (4, true) => trusted_read::<5, true>(&mut trusted, &mut trusted_row),
                (4, false) => trusted_read::<5, false>(&mut trusted, &mut trusted_row),
                (5, true) => trusted_read::<6, true>(&mut trusted, &mut trusted_row),
                (5, false) => trusted_read::<6, false>(&mut trusted, &mut trusted_row),
                (6, true) => trusted_read::<7, true>(&mut trusted, &mut trusted_row),
                (6, false) => trusted_read::<7, false>(&mut trusted, &mut trusted_row),
                (7, true) => trusted_read::<8, true>(&mut trusted, &mut trusted_row),
                (7, false) => trusted_read::<8, false>(&mut trusted, &mut trusted_row),
                (_, true) => trusted_read::<9, true>(&mut trusted, &mut trusted_row),
                (_, false) => trusted_read::<9, false>(&mut trusted, &mut trusted_row),
            };
            prop_assert_eq!(u32::from(expected.get()), got);
            prop_assert_eq!(checked_row, trusted_row);
            prop_assert_eq!(checked.checkpoint(), trusted.checkpoint());
            prop_assert_eq!(row_reader.read_trusted_row(&mut sliced_row[..=n]), Some(got));
            prop_assert_eq!(sliced_row, trusted_row);
            prop_assert_eq!(row_reader.checkpoint(), trusted.checkpoint());
        }
    }

    /// `read_bool_trusted` must match the checked one-bit literal read.
    #[test]
    fn read_bool_trusted_matches_read_literal(
        data in proptest::collection::vec(any::<u8>(), 2..24),
        skew in 0usize..8,
    ) {
        let mut checked = SymbolDecoder::new(&data).unwrap();
        let mut trusted = SymbolDecoder::new(&data).unwrap();
        let mut row = [16384u16, 0, 0];
        for step in 0..40 {
            if step % 8 == skew {
                checked.read_symbol_u16(&mut row.clone()).unwrap();
                trusted.read_symbol_u16(&mut row).unwrap();
            }
            prop_assert_eq!(checked.read_literal(1).unwrap() != 0, trusted.read_bool_trusted());
            prop_assert_eq!(checked.checkpoint(), trusted.checkpoint());
        }
    }

    /// A bypass run must read the bits and leave the state of one-bit literal
    /// reads, across chunk boundaries and runs split by symbol reads.
    #[test]
    fn bypass_run_matches_read_literal(
        data in proptest::collection::vec(any::<u8>(), 2..40),
        runs in proptest::collection::vec(0usize..70, 1..5),
    ) {
        let mut checked = SymbolDecoder::new(&data).unwrap();
        let mut bulk = SymbolDecoder::new(&data).unwrap();
        let (mut checked_row, mut bulk_row) = ([12000u16, 0, 0], [12000u16, 0, 0]);
        for len in runs {
            checked.read_symbol_u16(&mut checked_row).unwrap();
            bulk.read_symbol_u16(&mut bulk_row).unwrap();
            let mut run = bulk.bypass_run();
            for _ in 0..len {
                prop_assert_eq!(checked.read_literal(1).unwrap() != 0, bulk.read_bool_in(&mut run));
            }
            bulk.end_bypass_run(run);
            prop_assert_eq!(checked.checkpoint(), bulk.checkpoint());
        }
    }

    /// `read_trusted` must stay panic-free on arbitrary row contents.
    #[test]
    fn read_trusted_never_panics_on_hostile_rows(
        mut row in proptest::array::uniform6(any::<u16>()),
        data in proptest::collection::vec(any::<u8>(), 2..16),
    ) {
        let mut decoder = SymbolDecoder::new(&data).unwrap();
        for _ in 0..8 {
            prop_assert!(decoder.read_trusted::<6, true>(&mut row) < 5);
        }
    }

    /// Trusted-mode decoding must never panic and only ever return a typed
    /// error when the shape gates pass but the probability entries and count
    /// are hostile — the case that actually reaches the arithmetic loop and
    /// the CDF update.
    #[test]
    fn trusted_mode_survives_hostile_probability_entries(
        n in MIN_SYMBOLS..=MAX_SYMBOLS,
        rate_index in 0usize..PARA_ADJUSTMENT_LIST.len(),
        probs in proptest::collection::vec(any::<i32>(), MAX_SYMBOLS - 1),
        count in any::<i32>(),
        data in proptest::collection::vec(any::<u8>(), 2..16),
    ) {
        let mut row = vec![0i32; n + 1];
        row[..n - 1].copy_from_slice(&probs[..n - 1]);
        row[n - 1] = rate_index as i32;
        row[n] = count;

        let trusted_config =
            SymbolDecoderConfig::new().with_cdf_validation_mode(CdfValidationMode::Trusted);
        let mut decoder = SymbolDecoder::with_config(&data, trusted_config).unwrap();
        match decoder.read_symbol(&mut row) {
            Ok(symbol) => prop_assert!(usize::from(symbol.get()) < n),
            Err(
                Error::InvalidSymbolCdf { .. }
                | Error::InvalidSymbolDecoderState { .. }
                | Error::UnexpectedEof { .. },
            ) => {}
            Err(other) => prop_assert!(false, "unexpected error kind: {other:?}"),
        }
    }

    /// The buffered bypass window must match the historical per-bit
    /// reference it replaced — `SymbolValue` scaled up by `bits` with the
    /// next payload bits read MSB-first, clamped by `SymbolMaxBits`,
    /// zero-padded past the payload end, and inverted — at every reachable
    /// width, start alignment, and end-of-payload overlap.
    #[test]
    fn scaled_bypass_value_matches_per_bit_reference(
        data in proptest::collection::vec(any::<u8>(), 2..16),
        advance in 0u32..=32,
        bits in 0u32..=MAX_LITERAL_BITS,
    ) {
        let mut decoder = SymbolDecoder::new(&data).unwrap();
        let _ = decoder.read_literal(advance);
        let checkpoint = decoder.checkpoint();
        let num_bits = decoder
            .symbol_max_bits()
            .clamp(0, i64::from(bits)) as u32;
        let start = checkpoint.consumed_bits.get();
        let mut value = 0u64;
        for offset in 0..num_bits {
            value = (value << 1)
                | u64::from(decoder.bit_at(start + u64::from(offset)).unwrap_or(0));
        }
        let reference = (u64::from(checkpoint.symbol_value) << bits)
            | ((value << (bits - num_bits)) ^ mask_for_bits(bits));
        prop_assert_eq!(decoder.scaled_bypass_value(bits), reference);
        prop_assert_eq!(decoder.checkpoint(), checkpoint);
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

/// The first `L` entries of `row` as a fixed-length row for `read_trusted`.
fn trusted_read<const L: usize, const UPDATE: bool>(
    decoder: &mut SymbolDecoder<'_>,
    row: &mut [u16; MAX_SYMBOLS + 1],
) -> u32 {
    let Some(prefix) = row.first_chunk_mut::<L>() else {
        return u32::MAX;
    };
    decoder.read_trusted::<L, UPDATE>(prefix)
}
