## Context

After the `intra_tx_type` trace (#349) and the `sec_tx_type` token (#350), this change
composes the eob=2 trace that carries BOTH §5.20.8.2 transform-type symbols, for the
`enable_intra_ist == 1` configuration.

`sec_tx_type` (§5.20.8.2 line 16613) is read right after `intra_tx_type` (line 16529),
before the coefficient base pass (verified adversarially against the spec mirror for
#350). So the composer takes the tx-type trace and inserts the `sec_tx_type` token
right after the `intra_tx_type` token (derived from the `intra_tx_type` token kind,
with a fallback). For this 4x4 (`Tx_Size_Sqr 0`) DCT_DCT (`intra_tx_type = 0`)
`DC_PRED` (`YMode != PAETH`) eob=2 block the IST condition holds: `enable_intra_ist ==
1`, `eob 2 != 1`, `!Lossless`, `TxType == DCT_DCT`, `eob 2 <= eobLim`. For a 4x4 block
`!large` (Tx_Width 4 < 8), so `eobLim = IST_4X4_HEIGHT = 8`, and `2 <= 8`. The symbol
is `sec_tx_type = 0` (IST off), which reads no `most_probable_stx_set` follow-up.

The twelve-token trace is `[0,0,0, 0, 1, 0, 0, 0, 0, 0, 1, 1]` (the tx-type trace with
`sec_tx_type = 0` inserted at index 6). The composer delegates to
`compose_minimal_intra_two_coeff_block_trace_with_tx_type`, reusing the derived
`coeff_base` context and scan-derived AC position.

## Goals / Non-Goals

**Goals:**

- The eob=2 trace with both `intra_tx_type` and the `sec_tx_type` IST symbol, proven
  through one §8.2 coder.

**Non-Goals:**

- No `most_probable_stx_set` (the IST-set symbol; this trace uses `sec_tx_type = 0`),
  no eob > 2, no non-DCT_DCT transform types, no non-`TX_SET_INTRA_1` sets, no
  runtime IST-condition evaluation, no tile-body, no packet output.

## Decisions

1. **Delegate + insert.** The composer reuses the tx-type trace and inserts one
   `sec_tx_type` token at the `sec_tx_type` position (right after `intra_tx_type`),
   matching the #349 delegate-and-insert pattern.

2. **`sec_tx_type = 0` (IST off).** The minimal IST trace uses symbol 0, which reads
   no `most_probable_stx_set` follow-up — keeping the trace at one extra symbol.

## Flight Manifest

- Change ID: `encoder-block-trace-ist`
- Feature IDs: `ENC-INTRA-BLOCK-TRACE-IST`
- Base commit: `f2c6802a`
- Depends on merged changes: `encoder-block-trace-two-coeff-tx-type`, `encoder-sec-tx-type-token`.
- Exact files/directories owned by this PR:
  - `crates/splot-encode/src/block_symbol_trace.rs`
  - `crates/splot-encode/src/block_symbol_trace_tests.rs`
  - `docs/IMPLEMENTATION-MATRIX.toml`
  - `docs/FEATURE-STATUS.md`
  - `docs/SPEC-COVERAGE.md`
  - `docs/ENCODER-ROADMAP.md`
  - `docs/ENCODER-GAP-AUDIT.md`
  - `openspec/changes/encoder-block-trace-ist/**`
  - `openspec/changes/archive/2026-06-19-encoder-block-trace-ist/**`
  - `openspec/specs/encoder-tools/spec.md`
- Exact files/directories forbidden to this PR:
  - all other crates; `crates/splot-encode/src/coefficient_tokenization.rs` and its
    submodules; `crates/splot-encode/src/error.rs`; `crates/splot-encode/src/lib.rs`;
    `crates/splot-encode/src/closed_loop.rs`
  - workspace manifests and `Cargo.lock`; AV2 spec mirror under `docs/spec/av2/**`
- Public APIs/types owned: none
- Matrix rows owned: `ENC-INTRA-BLOCK-TRACE-IST`
- Generated files owned: `docs/FEATURE-STATUS.md`, `docs/SPEC-COVERAGE.md`
- Open sibling PRs audited: none at base.
- Changed-file intersection with each sibling PR: none. If a decoder-mission PR
  lands first, sync `main`, regenerate the tracking docs, and re-gate BEFORE pushing.
- Semantic overlap with each sibling PR: none.
- Can build/test/merge directly onto main without another open PR: yes.

## Risks / Trade-offs

- [Risk] The IST condition is satisfied by construction, not runtime-evaluated, and
  the `sec_tx_type` symbol is spec-derived. -> Mitigation: the building blocks
  (#349/#350) are merged and tested; the eobLim arithmetic is documented; AVM
  cross-check at the packet milestone exercises it end-to-end.
