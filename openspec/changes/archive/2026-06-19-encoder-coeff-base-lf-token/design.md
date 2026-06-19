## Context

`ENC-COEFF-BASE-LF-CONTEXT` added the §8.3.2 `coeff_base` low-frequency luma
context derivation. This change adds the token that carries the resulting symbol:
the non-EOB `coeff_base` (the lower-scan coefficients in an eob > 1 block read
`coeff_base`, not `coeff_base_eob`), coded with the `TileCoeffBaseLfCdf` row for
low-frequency luma.

Key facts (verified vs the decoder `base_symbol.rs` / `coeff_rows.rs` and §5.20.7.27):

- The non-EOB base level equals the decoded symbol (the EOB-position
  `coeff_base_eob` level is symbol + 1; the non-EOB `coeff_base` level is the
  symbol).
- The CDF is `TileCoeffBaseLfCdf[coeff_cdf_q_ctx][tx_size][ctx][tcq_ctx]`
  (`DEFAULT_COEFF_BASE_LF_CDF`, `[4][5][33][2][7]`). `tcq_ctx = (tcqState >> 1) & 1`
  is 0 when TCQ is off (the decoder only updates `tcqState` when `use_tcq`, so it
  stays 0). The row length is 7 (six symbols under the `len - 1` symbol convention).

The generic `CoefficientTokenCdfRows` roundtrip router stores one row per token type
at its minimal-trace context (e.g. `coeff_base_lf_eob` at its DC context 0). The
`coeff_base_lf` row is stored at the eob=2 trace's DC context: an AC coefficient of
level 1 at scan pos 1 is the DC's sole significant neighbour, so
`coeff_base_lf_luma_context` returns 1 (`mag = 1`, `ctx = 1`, low-frequency
`c == 0` band). The router accepts `CoeffBaseLf { ctx: 1, tcq_ctx: 0 }`.

Adding the `CoeffBase` syntax variant ripples to the two exhaustive matches in the
crate (`CoefficientTokenSyntax::as_str` and the closed-loop single-DC recovery
helper). The recovery helper is a no-op for `CoeffBase` because the single-DC
closed loop never carries a non-EOB base.

## Goals / Non-Goals

**Goals:**

- The non-EOB `coeff_base` low-frequency luma token, its `TileCoeffBaseLfCdf` row in
  the generic roundtrip router, and a roundtrip proof through the in-tree §8.2 coder.

**Non-Goals:**

- No trace composition (the eob > 1 trace brick composes it), no
  `block_symbol_trace` CDF-rows wiring (that brick adds it), no chroma `coeff_base`,
  no high-frequency `coeff_base`, no `coeff_br` for the AC, no packet output.

## Decisions

1. **One fixed context in the row router.** Mirroring the existing per-token
   minimal-context rows, `coeff_base_lf` is stored at ctx 1 / tcq_ctx 0 — the eob=2
   trace's DC context, derived (not magic) via `coeff_base_lf_luma_context`. Other
   contexts are future bricks.

2. **`CoeffBase` recovery is a no-op.** The closed-loop single-DC recovery helper
   never sees a non-EOB `coeff_base`, so the new match arm is a documented no-op
   rather than introducing multi-coefficient recovery here.

## Flight Manifest

- Change ID: `encoder-coeff-base-lf-token`
- Feature IDs: `ENC-COEFF-BASE-LF-TOKEN`
- Base commit: `3a38d4b0` (`feat(encode): coeff_base low-frequency luma context for multi-coefficient (#330)`)
- Depends on merged changes: `encoder-coeff-base-lf-context`.
- Exact files/directories owned by this PR:
  - `crates/splot-encode/src/coefficient_tokenization.rs`
  - `crates/splot-encode/src/coefficient_tokenization_tests.rs`
  - `crates/splot-encode/src/closed_loop.rs`
  - `docs/IMPLEMENTATION-MATRIX.toml`
  - `docs/FEATURE-STATUS.md`
  - `docs/SPEC-COVERAGE.md`
  - `docs/ENCODER-ROADMAP.md`
  - `docs/ENCODER-GAP-AUDIT.md`
  - `openspec/changes/encoder-coeff-base-lf-token/**`
  - `openspec/changes/archive/2026-06-19-encoder-coeff-base-lf-token/**`
  - `openspec/specs/encoder-tools/spec.md`
- Exact files/directories forbidden to this PR:
  - all other crates; `crates/splot-encode/src/block_symbol_trace.rs`;
    `crates/splot-encode/src/error.rs`; `crates/splot-encode/src/lib.rs`
  - workspace manifests and `Cargo.lock`; AV2 spec mirror under `docs/spec/av2/**`
- Public APIs/types owned: none
- Matrix rows owned: `ENC-COEFF-BASE-LF-TOKEN`
- Generated files owned: `docs/FEATURE-STATUS.md`, `docs/SPEC-COVERAGE.md`
- Open sibling PRs audited: none open at base commit `3a38d4b0`.
- Changed-file intersection with each sibling PR: none. If a decoder-mission PR
  lands first, sync `main`, regenerate the tracking docs (the merge changes the
  feature count), and re-gate BEFORE pushing.
- Semantic overlap with each sibling PR: none; private encoder token.
- Can build/test/merge directly onto main without another open PR: yes.

## Risks / Trade-offs

- [Risk] The §8.2 roundtrip proves self-consistency, not that the context/CDF match
  a real decoder. -> Mitigation: the context comes from the merged, decoder-mirrored
  `coeff_base_lf_luma_context`; the CDF row is the spec `DEFAULT_COEFF_BASE_LF_CDF`;
  AVM cross-check is at the packet milestone.
- [Risk] A new `CoefficientTokenSyntax` variant ripples to exhaustive matches. ->
  Mitigation: both matches (`as_str`, closed-loop recovery) are updated; the
  recovery arm is a documented no-op.
