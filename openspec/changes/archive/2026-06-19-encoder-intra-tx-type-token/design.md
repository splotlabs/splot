## Context

For `eob > 1` intra blocks, §5.20.7.27 `transform_type()` reads an `intra_tx_type`
symbol between `eob_pt` and the coefficient base pass (the `eob == 1 → DCT_DCT`
shortcut no longer applies). The eob=2 trace (#336) scoped this away; this change
models the symbol for the default-`reduced_tx_set` 4x4 intra set.

Facts (verified vs the spec + §9 tables):

- `get_tx_set(TX_4X4, intra, reduced_tx_set = 0)` → `TX_SET_INTRA_1` (decoder
  `ordinary_pass/geometry.rs:958`).
- §8.3.2 Table 8.2 (`08-parsing-process.md:1621`): `TX_SET_INTRA_1` uses
  `TileIntraTxTypeSet1Cdf[Tx_Size_Sqr[txSz]]`; `Tx_Size_Sqr[TX_4X4] = 0`, so the CDF
  is `DEFAULT_INTRA_TX_TYPE_SET1_CDF[0]` (`[[i32; 8]; 3]`, 7 symbols).
- §5.20.7.27 line 16569: `TxType = Md_Idx_To_Type[Size_Class[txSz]][intraDir]
  [intra_tx_type]`. `Size_Class[TX_4X4] = 0`, `intraDir = DC_PRED = 0`, and
  `Md_Idx_To_Type[0][0] = [0, 3, 1, 2, 7, 8, 13]`, so **symbol 0 → TxType 0 =
  DCT_DCT** — the symbol the encoder emits for a plain DCT_DCT block.

The `intra_tx_type` CDF is not coefficient-CDF-q-context indexed (it keys on
`Tx_Size_Sqr`), so the generic router stores the single 4x4 (`Tx_Size_Sqr 0`) row.
The decoder does not yet read this symbol (only `get_tx_set` exists in splot-decode),
so the token is derived from the spec and the §9 tables, with a test guarding the
`Md_Idx_To_Type` derivation; conformance vs a real decoder is established at the
packet milestone (AVM cross-check).

## Goals / Non-Goals

**Goals:**

- The `intra_tx_type` (`TX_SET_INTRA_1`) token, its CDF row in the §8.2 router, and a
  roundtrip proof; the router extracted to a submodule for the source budget.

**Non-Goals:**

- No general `eob > 1` trace composition (a later brick), no `sec_tx_type`
  (intra secondary transform), no non-`TX_SET_INTRA_1` sets (WIDE/HIGH/SET2/inter),
  no non-`DC_PRED` `intraDir`, no tile-body, no packet output.

## Decisions

1. **Submodule for the budget.** The new syntax/selector/router-wiring pushed the
   parent over 1000 lines, so the `transform_type` accessor lands in its own
   submodule and the generic router (`CoefficientTokenCdfRows`) is extracted to a
   `cdf_rows` submodule (no behaviour change).

2. **Parameterized but 4x4-routed.** `intra_tx_type_set1_token(tx_size_sqr, symbol)`
   is parameterized, but the router currently stores only the 4x4 (`Tx_Size_Sqr 0`)
   row — the minimal block's case; other sizes are future bricks.

## Flight Manifest

- Change ID: `encoder-intra-tx-type-token`
- Feature IDs: `ENC-INTRA-TX-TYPE-TOKEN`
- Base commit: `ed44d1b3`
- Depends on merged changes: the multi-coefficient token bricks.
- Exact files/directories owned by this PR:
  - `crates/splot-encode/src/coefficient_tokenization.rs`
  - `crates/splot-encode/src/coefficient_tokenization/transform_type.rs`
  - `crates/splot-encode/src/coefficient_tokenization/cdf_rows.rs`
  - `crates/splot-encode/src/coefficient_tokenization_tests.rs`
  - `crates/splot-encode/src/closed_loop.rs`
  - `docs/IMPLEMENTATION-MATRIX.toml`
  - `docs/FEATURE-STATUS.md`
  - `docs/SPEC-COVERAGE.md`
  - `docs/ENCODER-ROADMAP.md`
  - `docs/ENCODER-GAP-AUDIT.md`
  - `openspec/changes/encoder-intra-tx-type-token/**`
  - `openspec/changes/archive/2026-06-19-encoder-intra-tx-type-token/**`
  - `openspec/specs/encoder-tools/spec.md`
- Exact files/directories forbidden to this PR:
  - all other crates; `crates/splot-encode/src/block_symbol_trace.rs`;
    `crates/splot-encode/src/error.rs`; `crates/splot-encode/src/lib.rs`
  - workspace manifests and `Cargo.lock`; AV2 spec mirror under `docs/spec/av2/**`
- Public APIs/types owned: none
- Matrix rows owned: `ENC-INTRA-TX-TYPE-TOKEN`
- Generated files owned: `docs/FEATURE-STATUS.md`, `docs/SPEC-COVERAGE.md`
- Open sibling PRs audited: none at base.
- Changed-file intersection with each sibling PR: none. If a decoder-mission PR
  lands first, sync `main`, regenerate the tracking docs, and re-gate BEFORE pushing.
- Semantic overlap with each sibling PR: none.
- Can build/test/merge directly onto main without another open PR: yes.

## Risks / Trade-offs

- [Risk] No decoder mirror for `intra_tx_type` reading → spec-only derivation. ->
  Mitigation: derived from §5.20.7.27 + the §9 `Md_Idx_To_Type`/`Size_Class`/
  `Tx_Size_Sqr` tables and the §8.3.2 CDF mapping, with a test guarding the
  `Md_Idx_To_Type` DCT_DCT derivation; AVM cross-check at the packet milestone.
