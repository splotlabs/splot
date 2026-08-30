// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use splot_core::span::ByteOffset;
use splot_core::symbol::{CdfUpdateMode, Symbol, SymbolDecoder, SymbolDecoderConfig};
use splot_core::symbol_encoder::SymbolEncoder;

use super::{SingleRefReadError, read_single_ref};
use crate::bitstream::tile_payload::{
    BlockSymbolTraceReadError, FrameCdfSubset, TileCdfSelector, TileCdfSubset,
};

const REFS_PER_FRAME: usize = 7;

fn symbol_decoder(payload: &[u8]) -> SymbolDecoder<'_> {
    SymbolDecoder::with_base_and_config(
        payload,
        ByteOffset::new(0),
        SymbolDecoderConfig::new().with_cdf_update_mode(CdfUpdateMode::Enabled),
    )
    .unwrap()
}

fn encode_single_ref(
    tile: &mut TileCdfSubset,
    encoder: &mut SymbolEncoder,
    ctx: usize,
    ref_idx: usize,
    bit: u8,
) {
    tile.with_row_mut(TileCdfSelector::SingleRef { ctx, ref_idx }, |row| {
        encoder.write_symbol_u16(row, Symbol::new(bit))
    })
    .unwrap()
    .unwrap();
}

fn encode_selection(
    num_total_refs: usize,
    contexts: &[usize],
    selection: usize,
) -> (TileCdfSubset, Vec<u8>) {
    let decisions = num_total_refs - 1;
    let mut enc_tile = FrameCdfSubset::from_defaults().tile_copy();
    let mut encoder = SymbolEncoder::new();
    let mut reached_selection = false;

    for (ref_idx, &ctx) in contexts.iter().enumerate().take(decisions) {
        if ref_idx < selection {
            encode_single_ref(&mut enc_tile, &mut encoder, ctx, ref_idx, 0);
        } else {
            encode_single_ref(&mut enc_tile, &mut encoder, ctx, ref_idx, 1);
            reached_selection = true;
            break;
        }
    }

    if !reached_selection {
        assert_eq!(selection, decisions);
    }

    (enc_tile, encoder.finish().unwrap().into_bytes())
}

fn distinct_contexts(decisions: usize) -> Vec<usize> {
    (0..decisions).map(|i| i % 3).collect()
}

#[test]
fn single_ref_selection_roundtrips_through_symbol_encoder_for_every_value() {
    let num_total_refs = REFS_PER_FRAME;
    let decisions = num_total_refs - 1;
    let contexts = distinct_contexts(decisions);

    for selection in 0..num_total_refs {
        let (mut enc_tile, bytes) = encode_selection(num_total_refs, &contexts, selection);

        let mut dec_tile = FrameCdfSubset::from_defaults().tile_copy();
        let mut symbols = symbol_decoder(&bytes);
        let decoded = read_single_ref(&mut dec_tile, &mut symbols, num_total_refs, &contexts)
            .unwrap_or_else(|e| panic!("read_single_ref failed for selection {selection}: {e}"));

        assert_eq!(
            decoded, selection,
            "decoded RefFrame[0] must equal the encoded selection"
        );

        let summary = symbols.exit_symbol().unwrap();
        assert!(summary.consumed_bits.get() > 0);

        let reads_read = (selection + 1).min(decisions);
        for (ref_idx, &ctx) in contexts.iter().enumerate().take(reads_read) {
            let sel = TileCdfSelector::SingleRef { ctx, ref_idx };
            assert_eq!(
                enc_tile.with_row_mut(sel, |row| row.to_vec()).unwrap(),
                dec_tile.with_row_mut(sel, |row| row.to_vec()).unwrap(),
                "single_ref CDF row [{ctx}][{ref_idx}] desynced for selection {selection}"
            );
        }
    }
}

#[test]
fn single_ref_roundtrips_across_num_total_refs_and_distinct_contexts() {
    for num_total_refs in 2..=REFS_PER_FRAME {
        let decisions = num_total_refs - 1;
        let contexts = distinct_contexts(decisions);
        for selection in 0..num_total_refs {
            let (_, bytes) = encode_selection(num_total_refs, &contexts, selection);

            let mut dec_tile = FrameCdfSubset::from_defaults().tile_copy();
            let mut symbols = symbol_decoder(&bytes);
            let decoded =
                read_single_ref(&mut dec_tile, &mut symbols, num_total_refs, &contexts).unwrap();

            assert_eq!(
                decoded, selection,
                "selection {selection} at NumTotalRefs {num_total_refs}"
            );
            symbols.exit_symbol().unwrap();
        }
    }
}

#[test]
fn single_ref_context_indexing_is_load_bearing() {
    let num_total_refs = 4;
    let encode_contexts = [0usize, 1, 2];
    let selection = 2;

    let (_, bytes) = encode_selection(num_total_refs, &encode_contexts, selection);

    let mut dec_tile = FrameCdfSubset::from_defaults().tile_copy();
    let mut symbols = symbol_decoder(&bytes);
    let decoded = read_single_ref(
        &mut dec_tile,
        &mut symbols,
        num_total_refs,
        &encode_contexts,
    )
    .unwrap();
    assert_eq!(decoded, selection);
    symbols.exit_symbol().unwrap();

    let mut wrong_contexts = encode_contexts;
    wrong_contexts[2] = 0;
    let mut dec_tile = FrameCdfSubset::from_defaults().tile_copy();
    let mut symbols = symbol_decoder(&bytes);
    let wrong = read_single_ref(&mut dec_tile, &mut symbols, num_total_refs, &wrong_contexts);
    let mismatched = match wrong {
        Ok(value) => value != selection || symbols.exit_symbol().is_err(),
        Err(_) => true,
    };
    assert!(
        mismatched,
        "swapping the terminating single_ref context must change the decode or fail exit_symbol()"
    );
}

#[test]
fn single_ref_num_total_refs_one_returns_zero_without_reading() {
    let mut dec_tile = FrameCdfSubset::from_defaults().tile_copy();
    let payload = [0x80u8, 0x00];
    let mut symbols = symbol_decoder(&payload);
    let before = symbols.consumed_bits();
    let selection = read_single_ref(&mut dec_tile, &mut symbols, 1, &[]).unwrap();
    assert_eq!(selection, 0, "NumTotalRefs == 1 -> RefFrame[0] == 0");
    assert_eq!(symbols.consumed_bits(), before);
}

#[test]
fn single_ref_rejects_zero_refs() {
    let mut dec_tile = FrameCdfSubset::from_defaults().tile_copy();
    let payload = [0x80u8, 0x00];
    let mut symbols = symbol_decoder(&payload);
    let before = symbols.consumed_bits();
    let err = read_single_ref(&mut dec_tile, &mut symbols, 0, &[]).unwrap_err();
    assert!(matches!(
        err,
        SingleRefReadError::InsufficientRefs { num_total_refs: 0 }
    ));
    assert_eq!(symbols.consumed_bits(), before);
}

#[test]
fn single_ref_missing_context_is_a_typed_error_not_a_panic() {
    let mut dec_tile = FrameCdfSubset::from_defaults().tile_copy();
    let payload = [0x80u8, 0x00, 0x00];
    let mut symbols = symbol_decoder(&payload);
    let before = symbols.consumed_bits();
    let err = read_single_ref(&mut dec_tile, &mut symbols, 3, &[0]).unwrap_err();
    assert!(matches!(
        err,
        SingleRefReadError::MissingContext {
            needed: 2,
            got: 1,
            num_total_refs: 3,
        }
    ));
    assert_eq!(symbols.consumed_bits(), before);
}

#[test]
fn single_ref_out_of_range_context_is_a_typed_error_not_a_panic() {
    let mut dec_tile = FrameCdfSubset::from_defaults().tile_copy();
    let payload = [0x80u8, 0x00, 0x00];
    let mut symbols = symbol_decoder(&payload);
    let err = read_single_ref(&mut dec_tile, &mut symbols, 2, &[3]).unwrap_err();
    assert!(matches!(
        err,
        SingleRefReadError::SymbolRead {
            ref_idx: 0,
            source: BlockSymbolTraceReadError::Cdf(_),
        }
    ));
}

#[test]
fn single_ref_short_buffer_does_not_panic() {
    let mut dec_tile = FrameCdfSubset::from_defaults().tile_copy();
    let payload = [0x00u8];
    let mut symbols = symbol_decoder(&payload);
    match read_single_ref(
        &mut dec_tile,
        &mut symbols,
        REFS_PER_FRAME,
        &distinct_contexts(6),
    ) {
        Ok(selection) => assert!(selection < REFS_PER_FRAME),
        Err(SingleRefReadError::SymbolRead { ref_idx, source }) => {
            assert!(ref_idx < REFS_PER_FRAME);
            assert!(matches!(source, BlockSymbolTraceReadError::Symbol(_)));
        }
        Err(other) => panic!("unexpected error variant: {other}"),
    }
}
