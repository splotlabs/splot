## Context

The eob=2 trace (#336) scoped away the §5.20.8.2 `transform_type()` signaling. The
`intra_tx_type` token (#347) now models the symbol. This change composes the general
eob > 1 trace for the default `reduced_tx_set` `TX_SET_INTRA_1` configuration by
inserting the `intra_tx_type` symbol into the eob=2 trace.

`transform_type()` is read right after the eob reading (§5.20.7.27 line 15474) and
before the coefficient base pass. So the composer takes the eob=2 trace and inserts
the `intra_tx_type` token after the `eob_pt_16` token (trace index 4 = 3 modes +
`all_zero` + `eob_pt`). For a 4x4 (`Tx_Size_Sqr 0`) `DC_PRED` block the symbol is 0
(`DCT_DCT`, `Md_Idx_To_Type[0][0][0] = 0`, verified in #347). The eleven-token trace
is `[0,0,0, 0, 1, 0, 0, 0, 0, 1, 1]`.

This removes the eob=2 trace's `reduced_tx_set == 2` / DCT-only assumption (the
default `reduced_tx_set` reads `intra_tx_type`); it still assumes
`enable_intra_ist == 0`, since §5.20.7.29 would otherwise read a `sec_tx_type`
symbol — that signaling is a later brick.

The composer delegates to `compose_minimal_intra_two_coeff_block_trace` and inserts
the one token, reusing the derived `coeff_base` context and the scan-derived AC
position from #336. The §5.20.7.28 golomb-tail composers are extracted into a
`block_symbol_trace/golomb.rs` submodule (`use super::*`) to keep the parent file
under the 1000-line budget; no behaviour change.

## Goals / Non-Goals

**Goals:**

- The eob=2 trace with the `TX_SET_INTRA_1` `intra_tx_type` DCT_DCT symbol after
  `eob_pt_16`, proven through one §8.2 coder; the golomb composers split out for the
  budget.

**Non-Goals:**

- No `sec_tx_type` (still `enable_intra_ist == 0`), no eob > 2, no non-DCT_DCT
  transform types, no non-`TX_SET_INTRA_1` sets, no non-`DC_PRED` directions, no
  tile-body, no packet output.

## Decisions

1. **Delegate + insert.** The composer reuses the eob=2 trace and inserts one
   `intra_tx_type` token at the `transform_type()` position, rather than rebuilding
   the trace — minimal and keeps the derived-context logic in one place.

2. **Golomb submodule for the budget.** Adding the composer + CDF row pushed the
   parent over 1000 lines, so the cohesive golomb-tail composers move to a
   `golomb.rs` submodule (the established split pattern).

## Flight Manifest

- Change ID: `encoder-block-trace-two-coeff-tx-type`
- Feature IDs: `ENC-INTRA-BLOCK-TRACE-TWO-COEFF-TX-TYPE`
- Base commit: `3a88c871`
- Depends on merged changes: `encoder-block-trace-two-coeff`, `encoder-intra-tx-type-token`.
- Exact files/directories owned by this PR:
  - `crates/splot-encode/src/block_symbol_trace.rs`
  - `crates/splot-encode/src/block_symbol_trace/golomb.rs`
  - `crates/splot-encode/src/block_symbol_trace_tests.rs`
  - `docs/IMPLEMENTATION-MATRIX.toml`
  - `docs/FEATURE-STATUS.md`
  - `docs/SPEC-COVERAGE.md`
  - `docs/ENCODER-ROADMAP.md`
  - `docs/ENCODER-GAP-AUDIT.md`
  - `openspec/changes/encoder-block-trace-two-coeff-tx-type/**`
  - `openspec/changes/archive/2026-06-19-encoder-block-trace-two-coeff-tx-type/**`
  - `openspec/specs/encoder-tools/spec.md`
- Exact files/directories forbidden to this PR:
  - all other crates; `crates/splot-encode/src/coefficient_tokenization.rs` and its
    submodules; `crates/splot-encode/src/error.rs`; `crates/splot-encode/src/lib.rs`;
    `crates/splot-encode/src/closed_loop.rs`
  - workspace manifests and `Cargo.lock`; AV2 spec mirror under `docs/spec/av2/**`
- Public APIs/types owned: none
- Matrix rows owned: `ENC-INTRA-BLOCK-TRACE-TWO-COEFF-TX-TYPE`
- Generated files owned: `docs/FEATURE-STATUS.md`, `docs/SPEC-COVERAGE.md`
- Open sibling PRs audited: none at base.
- Changed-file intersection with each sibling PR: none. If a decoder-mission PR
  lands first, sync `main`, regenerate the tracking docs, and re-gate BEFORE pushing.
- Semantic overlap with each sibling PR: none.
- Can build/test/merge directly onto main without another open PR: yes.

## Risks / Trade-offs

- [Risk] The data-dependent `coeff_base` context and the spec-derived `intra_tx_type`
  symbol are not validated by the §8.2 roundtrip alone. -> Mitigation: both come from
  the merged, tested building blocks (#336/#347); AVM cross-check at the packet
  milestone.
