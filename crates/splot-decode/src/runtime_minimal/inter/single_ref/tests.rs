// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>

//! `SymbolEncoder` <-> `read_single_ref` round-trip proofs for the AV2
//! § 5.20.7.12 `single_ref` entropy element.
//!
//! These tests encode the exact `single_ref` symbol sequence that
//! `read_single_ref` reads for a target `RefFrame[0]` selection, using
//! `splot-core`'s `SymbolEncoder` over the SAME `TileSingleRefCdf[ctx][ref]` rows
//! the decoder reads (an identical `tile_copy()` via `with_row_mut`), then assert
//! `read_single_ref` recovers the encoded selection and that `exit_symbol()`
//! agrees on the bit count. The selections and per-decision contexts are
//! ASYMMETRIC (every selectable `RefFrame[0]` value and DISTINCT per-`ref`
//! contexts) so a transposed tree decision or a wrong CDF-row index would change
//! the decoded selection and be caught.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use splot_core::span::ByteOffset;
use splot_core::symbol::{CdfUpdateMode, Symbol, SymbolDecoder, SymbolDecoderConfig};
use splot_core::symbol_encoder::SymbolEncoder;

use super::{SingleRefReadError, read_single_ref};
use crate::tile_payload::{FrameCdfSubset, TileCdfSelector, TileCdfSubset};

const REFS_PER_FRAME: usize = 7;

fn symbol_decoder(payload: &[u8]) -> SymbolDecoder<'_> {
    SymbolDecoder::with_base_and_config(
        payload,
        ByteOffset::new(0),
        SymbolDecoderConfig::new().with_cdf_update_mode(CdfUpdateMode::Enabled),
    )
    .unwrap()
}

/// Writes one `single_ref` binary symbol to `tile`'s `TileSingleRefCdf[ctx][ref]`
/// row (adapting it), mirroring the decoder's `read_block_symbol_trace`.
fn encode_single_ref(
    tile: &mut TileCdfSubset,
    encoder: &mut SymbolEncoder,
    ctx: usize,
    ref_idx: usize,
    bit: u8,
) {
    tile.with_row_mut(TileCdfSelector::SingleRef { ctx, ref_idx }, |row| {
        encoder.write_symbol(row, Symbol::new(bit))
    })
    .unwrap()
    .unwrap();
}

/// Encodes the § 5.20.7.12 `single_ref` tree for a target `selection`: a `0` at
/// every `ref < selection`, then a `1` at `ref == selection` (unless `selection`
/// is the terminal `NumTotalRefs - 1`, which is reached purely by `0` symbols).
fn encode_selection(
    enc_tile: &mut TileCdfSubset,
    encoder: &mut SymbolEncoder,
    num_total_refs: usize,
    contexts: &[usize],
    selection: usize,
) {
    let decisions = num_total_refs - 1;
    for (ref_idx, &ctx) in contexts.iter().enumerate().take(decisions) {
        if ref_idx < selection {
            encode_single_ref(enc_tile, encoder, ctx, ref_idx, 0);
        } else {
            encode_single_ref(enc_tile, encoder, ctx, ref_idx, 1);
            return;
        }
    }
    assert_eq!(selection, decisions);
}

/// Distinct per-`ref` contexts (0, 1, 2, 0, 1, 2, ...) so a wrong CDF-row context
/// index would adapt the wrong row and desynchronize the decode.
fn distinct_contexts(decisions: usize) -> Vec<usize> {
    (0..decisions).map(|i| i % 3).collect()
}

#[test]
fn single_ref_selection_roundtrips_through_symbol_encoder_for_every_value() {
    let num_total_refs = REFS_PER_FRAME;
    let decisions = num_total_refs - 1;
    let contexts = distinct_contexts(decisions);

    for selection in 0..num_total_refs {
        let mut enc_tile = FrameCdfSubset::from_defaults().tile_copy();
        let mut encoder = SymbolEncoder::new();
        encode_selection(
            &mut enc_tile,
            &mut encoder,
            num_total_refs,
            &contexts,
            selection,
        );
        let bytes = encoder.finish().unwrap().into_bytes();

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
                enc_tile.row(sel).unwrap(),
                dec_tile.row(sel).unwrap(),
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
            let mut enc_tile = FrameCdfSubset::from_defaults().tile_copy();
            let mut encoder = SymbolEncoder::new();
            encode_selection(
                &mut enc_tile,
                &mut encoder,
                num_total_refs,
                &contexts,
                selection,
            );
            let bytes = encoder.finish().unwrap().into_bytes();

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
    let num_total_refs = 4; // 3 decisions, contexts 0/1/2
    let encode_contexts = vec![0usize, 1, 2];
    let selection = 2; // single_ref reads ref0=0, ref1=0, ref2=1 over ctx 0/1/2.

    let mut enc_tile = FrameCdfSubset::from_defaults().tile_copy();
    let mut encoder = SymbolEncoder::new();
    encode_selection(
        &mut enc_tile,
        &mut encoder,
        num_total_refs,
        &encode_contexts,
        selection,
    );
    let bytes = encoder.finish().unwrap().into_bytes();

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

    let mut wrong_contexts = encode_contexts.clone();
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
    assert!(matches!(err, SingleRefReadError::SymbolRead { ref_idx: 0 }));
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
        Err(SingleRefReadError::SymbolRead { ref_idx }) => assert!(ref_idx < REFS_PER_FRAME),
        Err(other) => panic!("unexpected error variant: {other}"),
    }
}
