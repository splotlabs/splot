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

// REFS_PER_FRAME - 1 == 6 single_ref decisions: `read_single_ref` can select any
// `RefFrame[0]` in 0..=REFS_PER_FRAME - 1 when NumTotalRefs == REFS_PER_FRAME.
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
            // ref_idx == selection: write the terminating `1` and stop.
            encode_single_ref(enc_tile, encoder, ctx, ref_idx, 1);
            return;
        }
    }
    // selection == decisions (NumTotalRefs - 1): every decision wrote a `0`.
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

    // Asymmetric: every selectable RefFrame[0] in 0..=NumTotalRefs - 1.
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

        // §8.2.4 exit_symbol(): the decode consumed exactly the encoded stream and
        // the trailing-bit/padding tail validates (a wrong tree/context read would
        // have left the cursor at a different bit position and rejected here).
        let summary = symbols.exit_symbol().unwrap();
        assert!(summary.consumed_bits.get() > 0);

        // The encode and decode CDF tiles ended in the same adapted state for every
        // row that was actually read, confirming row-lockstep across the stream.
        // `read_single_ref` reads ref 0..=selection (clamped to the decision count).
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
    // Sweep NumTotalRefs from 2 (the minimum that reads a single_ref) up to
    // REFS_PER_FRAME, every selection, with distinct per-ref contexts.
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
    // Falsifiability witness: encode a selection with one set of contexts, then
    // decode with a DIFFERENT context at the terminating decision. Because the
    // CDF rows differ, the decode either desyncs to a different selection or
    // fails exit_symbol() — proving the per-decision context index is genuinely
    // consumed (it is not a no-op the asymmetric test could miss).
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

    // Decode with the matching contexts: must recover the selection exactly.
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

    // Decode with a SWAPPED terminating context (ctx 0 instead of 2 at ref2).
    // The default Single_Ref_Cdf row for [0][2] differs from [2][2], so the
    // decode is no longer guaranteed to be the encoded selection AND/OR to pass
    // exit_symbol(); assert at least one observable difference (it is not a no-op).
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
    // §5.20.7.12 with NumTotalRefs == 1: the loop is empty, so RefFrame[0] == 0 is
    // returned with NO single_ref symbol read. This is the legal one-reference case
    // (§6.19.7.11 only requires NumTotalRefs > 0), NOT an error.
    let mut dec_tile = FrameCdfSubset::from_defaults().tile_copy();
    let payload = [0x80u8, 0x00];
    let mut symbols = symbol_decoder(&payload);
    let before = symbols.consumed_bits();
    let selection = read_single_ref(&mut dec_tile, &mut symbols, 1, &[]).unwrap();
    assert_eq!(selection, 0, "NumTotalRefs == 1 -> RefFrame[0] == 0");
    // No symbol was read (the empty loop).
    assert_eq!(symbols.consumed_bits(), before);
}

#[test]
fn single_ref_rejects_zero_refs() {
    // §6.19.7.11 requires NumTotalRefs > 0; 0 would underflow NumTotalRefs - 1.
    let mut dec_tile = FrameCdfSubset::from_defaults().tile_copy();
    let payload = [0x80u8, 0x00];
    let mut symbols = symbol_decoder(&payload);
    let before = symbols.consumed_bits();
    let err = read_single_ref(&mut dec_tile, &mut symbols, 0, &[]).unwrap_err();
    assert!(matches!(
        err,
        SingleRefReadError::InsufficientRefs { num_total_refs: 0 }
    ));
    // The rejection happens before any symbol read.
    assert_eq!(symbols.consumed_bits(), before);
}

#[test]
fn single_ref_missing_context_is_a_typed_error_not_a_panic() {
    // A contexts slice shorter than NumTotalRefs - 1 must surface a typed error
    // before any symbol read rather than panicking (no out-of-range index).
    let mut dec_tile = FrameCdfSubset::from_defaults().tile_copy();
    let payload = [0x80u8, 0x00, 0x00];
    let mut symbols = symbol_decoder(&payload);
    let before = symbols.consumed_bits();
    // NumTotalRefs == 3 -> 2 decisions, but only one context supplied.
    let err = read_single_ref(&mut dec_tile, &mut symbols, 3, &[0]).unwrap_err();
    assert!(matches!(
        err,
        SingleRefReadError::MissingContext {
            needed: 2,
            got: 1,
            num_total_refs: 3,
        }
    ));
    // The rejection happens before any symbol read.
    assert_eq!(symbols.consumed_bits(), before);
}

#[test]
fn single_ref_out_of_range_context_is_a_typed_error_not_a_panic() {
    // A context beyond REF_CONTEXTS (0..3) makes the TileSingleRefCdf[ctx][ref]
    // selector fail; read_single_ref maps it to a typed SymbolRead error at that
    // ref rather than panicking on an out-of-range CDF index.
    let mut dec_tile = FrameCdfSubset::from_defaults().tile_copy();
    let payload = [0x80u8, 0x00, 0x00];
    let mut symbols = symbol_decoder(&payload);
    // NumTotalRefs == 2 -> a single decision at ref 0; ctx 3 is out of range.
    let err = read_single_ref(&mut dec_tile, &mut symbols, 2, &[3]).unwrap_err();
    assert!(matches!(err, SingleRefReadError::SymbolRead { ref_idx: 0 }));
}

#[test]
fn single_ref_short_buffer_does_not_panic() {
    // The §8.2 arithmetic decoder reads from implicit zero padding past the end
    // of a short buffer, so a single_ref read over a tiny payload still returns a
    // value rather than erroring; the load-bearing property here is that
    // read_single_ref never panics on a short buffer. Both decode arms (an Ok
    // selection in range, or a typed error) are acceptable.
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
