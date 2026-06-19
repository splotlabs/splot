## Context

The eob > 1 trace composes a coded multi-coefficient block. Its CDF tokens are: the
coded `all_zero` (0), `eob_pt_16` (symbol 1 → eob 2), the EOB-position AC
`coeff_base_eob` at context 1, and the DC `coeff_base` at context 1. The single-DC
bricks only expose `all_zero` (1), `eob_pt_16` (0), and `coeff_base_eob` at the DC
context 0, so this change adds the missing reusable accessors.

Key facts (verified vs the decoder `coeff_loop.rs` / `coeff_context.rs` and
§5.20.7.27):

- `eob_pt_16` symbol `s` with no extra bits gives `eobPt = s + 1`; for `eobPt < 3`,
  `eob = eobPt`. So symbol 1 → `eobPt = 2` → `eob = 2`.
- The EOB-position `coeff_base_eob` level is `coeff_base_eob + 1`, so the symbol is
  `level − 1`.
- The eob = 2 AC at scan index 1 uses `coeff_base_eob_ctx(c=1, bwl=2, h=4) =
  SIG_COEF_CONTEXTS_EOB − 3 = 1` (`c <= numCoeffs/8 = 2`), and (pos 1 is
  low-frequency, `row+col = 1 < 4`) the LF EOB base CDF `TileCoeffBaseLfEobCdf`.

The generic `CoefficientTokenCdfRows` router already routes `eob_pt_16` (any symbol;
the row is symbol-agnostic) and the coded `all_zero` (the luma `txb_skip` row,
symbol 0 vs 1). It only needs the new context-1 `coeff_base_lf_eob` row added.

The accessors live in a `coefficient_tokenization/multi_coeff.rs` submodule because
the parent file is near the 1000-line budget; the submodule accesses the parent's
private token types and `all_zero_token` via `super::`.

## Goals / Non-Goals

**Goals:**

- The three reusable multi-coefficient token accessors, the context-1
  `coeff_base_lf_eob` router row, and roundtrip proofs through the in-tree § 8.2
  coder.

**Non-Goals:**

- No trace composition (the eob > 1 trace brick composes these), no chroma /
  high-frequency contexts, no AC `coeff_br`, no `block_symbol_trace` wiring, no
  packet output.

## Decisions

1. **Submodule for the budget.** The parent file is near 1000 lines, so the new
   accessors land in `multi_coeff.rs`, mirroring the `coeff_base_lf.rs` split.

2. **Parameterized accessors.** `eob_pt_16_token` and `coeff_base_lf_eob_token` take
   the context/symbol/level so they serve both the single-DC and eob = 2 cases (and
   future contexts) without per-case duplication.

## Flight Manifest

- Change ID: `encoder-coeff-multi-coeff-tokens`
- Feature IDs: `ENC-COEFF-MULTI-TOKENS`
- Base commit: `5d642997` (`feat(encode): non-EOB coeff_base low-frequency luma token (#332)`)
- Depends on merged changes: `encoder-coeff-base-lf-token`.
- Exact files/directories owned by this PR:
  - `crates/splot-encode/src/coefficient_tokenization.rs`
  - `crates/splot-encode/src/coefficient_tokenization/multi_coeff.rs`
  - `crates/splot-encode/src/coefficient_tokenization_tests.rs`
  - `docs/IMPLEMENTATION-MATRIX.toml`
  - `docs/FEATURE-STATUS.md`
  - `docs/SPEC-COVERAGE.md`
  - `docs/ENCODER-ROADMAP.md`
  - `docs/ENCODER-GAP-AUDIT.md`
  - `openspec/changes/encoder-coeff-multi-coeff-tokens/**`
  - `openspec/changes/archive/2026-06-19-encoder-coeff-multi-coeff-tokens/**`
  - `openspec/specs/encoder-tools/spec.md`
- Exact files/directories forbidden to this PR:
  - all other crates; `crates/splot-encode/src/block_symbol_trace.rs`;
    `crates/splot-encode/src/error.rs`; `crates/splot-encode/src/lib.rs`;
    `crates/splot-encode/src/closed_loop.rs`
  - workspace manifests and `Cargo.lock`; AV2 spec mirror under `docs/spec/av2/**`
- Public APIs/types owned: none
- Matrix rows owned: `ENC-COEFF-MULTI-TOKENS`
- Generated files owned: `docs/FEATURE-STATUS.md`, `docs/SPEC-COVERAGE.md`
- Open sibling PRs audited: none open at base commit `5d642997`.
- Changed-file intersection with each sibling PR: none. If a decoder-mission PR
  lands first, sync `main`, regenerate the tracking docs (the merge changes the
  feature count), and re-gate BEFORE pushing.
- Semantic overlap with each sibling PR: none; private encoder accessors.
- Can build/test/merge directly onto main without another open PR: yes.

## Risks / Trade-offs

- [Risk] The §8.2 roundtrip proves self-consistency, not conformance. ->
  Mitigation: the contexts/CDF rows are decoder-mirrored; AVM cross-check is at the
  packet milestone.
