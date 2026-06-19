## Context

After `intra_tx_type` (#347) and its trace (#349), the next §5.20.8.2
`transform_type()` symbol is `sec_tx_type` (the IST secondary transform). This change
models the `sec_tx_type` token, mirroring the `intra_tx_type` token machinery.

The §5.20.8.2 facts were verified adversarially against the committed spec mirror (4
independent readers + 4 verifiers) before implementation:

- **Read location/order.** `sec_tx_type` (line 16613) is read inside `transform_type()`
  — the SAME function as `intra_tx_type` (line 16529), right after it, before the
  coefficient base pass. It is NOT in `compute_tx_type()` (§5.20.7.29, lines
  15975-16033), which is a pure derivation function that reads no symbols. (This
  corrected the initial assumption.)
- **CDF.** `TileSecTxTypeCdf[is_inter][Tx_Size_Sqr[txSz]]` (§8.3.2,
  `08-parsing-process.md:867`). The default table is `[[[i32; 5]; 5]; 2]`: outer
  `[2]` = `is_inter`, middle `[5]` = `Tx_Size_Sqr` (0..=4), inner `[5]` = `STX_TYPES
  (4) + 1` → 4 `sec_tx_type` symbol values.
- **Intra condition.** `enable_intra_ist && eob != 1 && !Lossless && (TxType ==
  ADST_ADST || DCT_DCT) && YMode != PAETH_PRED && eob <= eobLim`. `sec_tx_type == 0`
  reads no `most_probable_stx_set` follow-up (that S() is read only when `sec_tx_type
  != 0 && !is_inter`, line 16615-16617).

The encoder's minimal subset is intra, so the selector fixes the `is_inter = 0` bank
and routes `DEFAULT_SEC_TX_TYPE_CDF[0][tx_size_sqr]`. The token carries one of the
four `sec_tx_type` values; `symbol 0` (IST off) is the minimal case.

## Goals / Non-Goals

**Goals:**

- The `sec_tx_type` IST token + its intra CDF routing, proven through one §8.2 coder
  for every `Tx_Size_Sqr` row and every `sec_tx_type` value.

**Non-Goals:**

- No trace inserting the token (a later brick), no `most_probable_stx_set`, no
  runtime evaluation of the IST condition, no inter bank, no eob > 2, no tile-body,
  no packet output.

## Decisions

1. **Mirror the `intra_tx_type` token.** `sec_tx_type` is read in the same function
   with the same shape (a CDF selected by `Tx_Size_Sqr`), so it reuses the
   `transform_type` submodule + the generic CDF-row router pattern.

2. **Intra-only selector.** The selector fixes `is_inter = 0` (the encoder's minimal
   subset is intra), keeping the variant `SecTxTypeIntra { tx_size_sqr }` minimal; the
   inter bank is deferred.

3. **Correct the `IntraTxType` doc in passing.** The `IntraTxType` enum-variant doc
   still cited §5.20.7.27 (the caller); #347 corrected the submodule/matrix but not
   this variant. Since `SecTxType` is added right beside it and both are read in
   `transform_type()` (§5.20.8.2), fix the `IntraTxType` doc here for consistency.

## Flight Manifest

- Change ID: `encoder-sec-tx-type-token`
- Feature IDs: `ENC-SEC-TX-TYPE-TOKEN`
- Base commit: `ef97f6db`
- Depends on merged changes: `encoder-intra-tx-type-token`, `encoder-block-trace-two-coeff-tx-type`.
- Exact files/directories owned by this PR:
  - `crates/splot-encode/src/coefficient_tokenization.rs`
  - `crates/splot-encode/src/coefficient_tokenization/transform_type.rs`
  - `crates/splot-encode/src/coefficient_tokenization/cdf_rows.rs`
  - `crates/splot-encode/src/closed_loop.rs`
  - `crates/splot-encode/src/coefficient_tokenization_tests.rs`
  - `docs/IMPLEMENTATION-MATRIX.toml`
  - `docs/FEATURE-STATUS.md`
  - `docs/SPEC-COVERAGE.md`
  - `docs/ENCODER-ROADMAP.md`
  - `docs/ENCODER-GAP-AUDIT.md`
  - `openspec/changes/encoder-sec-tx-type-token/**`
  - `openspec/changes/archive/2026-06-19-encoder-sec-tx-type-token/**`
  - `openspec/specs/encoder-tools/spec.md`
- Exact files/directories forbidden to this PR:
  - all other crates; `crates/splot-encode/src/block_symbol_trace.rs` and its
    submodules; `crates/splot-encode/src/error.rs`; `crates/splot-encode/src/lib.rs`
  - workspace manifests and `Cargo.lock`; AV2 spec mirror under `docs/spec/av2/**`
- Public APIs/types owned: none
- Matrix rows owned: `ENC-SEC-TX-TYPE-TOKEN`
- Generated files owned: `docs/FEATURE-STATUS.md`, `docs/SPEC-COVERAGE.md`
- Open sibling PRs audited: none at base.
- Changed-file intersection with each sibling PR: none. If a decoder-mission PR
  lands first, sync `main`, regenerate the tracking docs, and re-gate BEFORE pushing.
- Semantic overlap with each sibling PR: none.
- Can build/test/merge directly onto main without another open PR: yes.

## Risks / Trade-offs

- [Risk] The `sec_tx_type` CDF context (intra bank, `Tx_Size_Sqr` index) and the read
  position are spec-derived, not yet exercised by a runtime trace. -> Mitigation:
  the facts were adversarially verified against the mirror; a later trace brick and
  the AVM cross-check at the packet milestone exercise them end-to-end.
