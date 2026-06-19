## Context

`ENC-INTRA-BLOCK-MODE-TRACE` composes the mode-info prefix through one §8.2 coder
using only intra-mode tokens. A coded tile body interleaves mode and coefficient
symbols through that same coder, so the trace must span both token kinds. This
change extends `block_symbol_trace` with a unified `BlockSymbolToken` and proves
the mode prefix followed by the first `residual()` symbol — the luma `txb_skip`
(`all_zero`) — roundtrips through one coder with shared CDF state.

AV2 §5.20.5.3 `intra_frame_mode_info()` reads the mode symbols before
`residual()`; the luma `txb_skip` / `all_zero` is the first §5.20.7.27 residual
symbol. For an all-zero block the luma `txb_skip` is `1` and no further luma
coefficient symbols follow.

Normative AV2 v1.0.0 sections:

- §5.20.5.3 mode info before `residual()`
  (`docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-20-5-3`).
- §5.20.7.27 `all_zero` (`#s-5-20-7-27`).
- §8.2 / §8.3.2 — the shared-CDF roundtrip and `TileTxbSkipCdf` row.

## Goals / Non-Goals

**Goals:**

- Add `compose_minimal_intra_dc_all_zero_block_trace` for
  `ENC-INTRA-BLOCK-TRACE-LUMA-SKIP`, returning the ordered `y_mode_set`,
  `y_mode_index`, `uv_mode`, luma `txb_skip` token sequence across the two token
  kinds.
- Add a unified §8.2 roundtrip holding the mode and `txb_skip` CDF rows from
  `splot-core` defaults, routing each token to its scoped row, proving the
  combined sequence through one coder with shared CDF state.
- Preserve the no-packet invariant.

**Non-Goals:**

- No chroma `txb_skip` (U/V all-zero), non-all-zero luma coefficients, partition
  syntax, tile CDF lifecycle, tile-body emission, packet output, CLI success, or
  Baseline Encoder Profile v1 claim.
- No dependency graph change and no AVM/dav2d evidence.

## Decisions

1. **Unified `BlockSymbolToken` in `block_symbol_trace`.** A two-variant enum
   `{ Mode(IntraModeToken), Coeff(CoefficientEntropyToken) }` lets the trace carry
   both token kinds in order. The module owns a unified CDF holder built directly
   from `splot-core` default tables (`DEFAULT_Y_MODE_SET_CDF`,
   `DEFAULT_Y_MODE_INDEX_CDF[0]`, `DEFAULT_UV_MODE_CFL_NOT_ALLOWED_CDF[0]`,
   `DEFAULT_TXB_SKIP_CDF[0][0][0][0]`) and routes each token's scoped selector to
   the matching row. Keeping the holder in the trace module avoids exposing the
   emitter modules' private CDF-row internals.

2. **Reuse tokens, expose only a small accessor.** The mode tokens come from the
   merged emitters; the luma `txb_skip` token comes from a new `pub(crate)`
   `luma_all_zero_token` accessor over the existing private `all_zero_token`
   helper in `coefficient_tokenization`. No other emitter internals are exposed.

3. **Errors keyed by token index.** The unified roundtrip uses typed
   `BlockSymbolTrace*` errors keyed by the failing token index, so it does not
   depend on either emitter module's private syntax-name helpers.

4. **Minimal-tier `coeff_cdf_q_ctx = 0`.** The luma `txb_skip` uses the minimal
   `coeff_cdf_q_ctx` 0 row; the roundtrip is self-consistent (the same row is used
   to encode and decode), so the exact q-context does not affect the proof.

## Flight Manifest

- Change ID: `encoder-block-trace-luma-skip`
- Feature IDs: `ENC-INTRA-BLOCK-TRACE-LUMA-SKIP`
- Base commit: `0a757637` (`feat(encode): compose minimal intra-block mode trace (#310)`)
- Depends on merged changes: `encoder-intra-block-mode-trace`,
  `encoder-coefficient-tokenization-minimal`
- Exact files/directories owned by this PR:
  - `crates/splot-encode/src/block_symbol_trace.rs`
  - `crates/splot-encode/src/coefficient_tokenization.rs`
  - `crates/splot-encode/src/error.rs`
  - `docs/IMPLEMENTATION-MATRIX.toml`
  - `docs/FEATURE-STATUS.md`
  - `docs/SPEC-COVERAGE.md`
  - `docs/ENCODER-ROADMAP.md`
  - `docs/ENCODER-GAP-AUDIT.md`
  - `openspec/changes/encoder-block-trace-luma-skip/**`
  - `openspec/changes/archive/2026-06-19-encoder-block-trace-luma-skip/**`
  - `openspec/specs/encoder-tools/spec.md`
- Exact files/directories forbidden to this PR:
  - all other crates; `crates/splot-encode/src/intra_mode_emission.rs` (reused via
    its existing `pub(crate)` API); `crates/splot-encode/src/lib.rs` (no module
    added; `block_symbol_trace` already declared)
  - workspace manifests and `Cargo.lock`; AV2 spec mirror under `docs/spec/av2/**`
- Public APIs/types owned: none
- Matrix rows owned: `ENC-INTRA-BLOCK-TRACE-LUMA-SKIP`
- Generated files owned: `docs/FEATURE-STATUS.md`, `docs/SPEC-COVERAGE.md`
- Open sibling PRs audited: none open at base commit `0a757637`.
- Changed-file intersection with each sibling PR: none. If a decoder-mission PR
  lands first, sync `main`, regenerate the tracking docs, and re-gate.
- Semantic overlap with each sibling PR: none; private encoder composition.
- Can build/test/merge directly onto main without another open PR: yes.

## Risks / Trade-offs

- [Risk] The unified CDF holder re-selects rows the emitter modules also select,
  duplicating a little routing logic. -> Mitigation: it covers only the four
  minimal-tier rows and keeps the emitter modules decoupled (no internal
  exposure beyond one token accessor).
- [Risk] The trace omits chroma `txb_skip`, so it is not yet a complete block. ->
  Mitigation: this is the unified-coder abstraction plus the first residual
  symbol; chroma `txb_skip` and the complete block trace follow with their own
  Feature IDs.
