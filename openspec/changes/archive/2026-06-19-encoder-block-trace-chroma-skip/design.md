## Context

The unified `block_symbol_trace` proves the mode prefix plus the luma `txb_skip`
through one §8.2 coder. A coded all-zero intra block reads a per-plane `all_zero`
for luma, U, and V in `residual()` order, so this change completes the minimal
all-zero block symbol sequence with the chroma U and V `txb_skip` symbols.

Chroma `txb_skip` CDF selection (AV2 §8.3.2):

- **U (plane 1)** reuses `TileTxbSkipCdf` with `plane_type == 1`; the neutral
  (`above == 0`, `left == 0`) context adds a fixed `+6`, so the minimal U context
  is `6` → `DEFAULT_TXB_SKIP_CDF[q][1][0][6]`.
- **V (plane 2)** uses the dedicated `TileVTxbSkipCdf[q][ctx]`; for a block whose
  chroma block size equals its transform size with an all-zero U plane the
  context is `0` → `DEFAULT_V_TXB_SKIP_CDF[q][0]`.

Normative AV2 v1.0.0 sections:

- §5.20.7.27 per-plane `all_zero` (`docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-27`),
  read once per plane in `residual()` plane order Y, U, V.
- §8.3.2 chroma `txb_skip` context derivation (U `+6`; dedicated V CDF).

## Goals / Non-Goals

**Goals:**

- Add `compose_minimal_intra_dc_complete_all_zero_block_trace` for
  `ENC-INTRA-BLOCK-TRACE-CHROMA-SKIP`, the ordered mode prefix then luma/U/V
  `txb_skip` (`all_zero == 1`) tokens.
- Add `pub(crate)` chroma U/V `all_zero` token accessors and a `VTxbSkip` CDF-row
  selector to `coefficient_tokenization`.
- Prove the complete six-symbol trace through one §8.2 coder with shared CDF
  state, routing each `txb_skip` to its scoped row.
- Preserve the no-packet invariant.

**Non-Goals:**

- No non-all-zero coefficients (EOB/base/sign), CfL/CCTX, partition syntax, tile
  CDF lifecycle, tile-body emission, packet output, CLI success, or Baseline
  Encoder Profile v1 claim.
- No dependency graph change and no AVM/dav2d evidence.

## Decisions

1. **U reuses `TxbSkip`; V needs a new `VTxbSkip` selector.** There is no separate
   U `txb_skip` table — U is `TileTxbSkipCdf` with `plane_type == 1`. V uses the
   dedicated `TileVTxbSkipCdf`, so a `VTxbSkip { coeff_cdf_q_ctx, ctx }` variant is
   added to `CoefficientCdfRowSelector`. The coefficient tokenization module never
   produces `VTxbSkip` (it is luma-only); its row router falls through to its
   existing unsupported-selector error for it.

2. **Minimal-tier chroma block equals transform size.** The encoder chooses the
   simplest legal geometry where the chroma block size equals its transform size
   with an all-zero U plane, giving V context `0` (no chroma-larger-than-tx or
   `EobU != 0` contributions). The roundtrip is self-consistent (the same row is
   used to encode and decode), so the exact context does not affect the proof, and
   the chosen contexts are spec-justified by §8.3.2.

3. **Complete trace reuses the luma-skip trace.** The new compose function appends
   the U and V tokens to `compose_minimal_intra_dc_all_zero_block_trace` (mode
   prefix + luma `txb_skip`), keeping the 4-symbol and 6-symbol units both
   available.

## Flight Manifest

- Change ID: `encoder-block-trace-chroma-skip`
- Feature IDs: `ENC-INTRA-BLOCK-TRACE-CHROMA-SKIP`
- Base commit: `0f451031` (`feat(encode): unify block-symbol trace with luma txb_skip (#311)`)
- Depends on merged changes: `encoder-block-trace-luma-skip`,
  `encoder-coefficient-tokenization-minimal`
- Exact files/directories owned by this PR:
  - `crates/splot-encode/src/block_symbol_trace.rs`
  - `crates/splot-encode/src/coefficient_tokenization.rs`
  - `docs/IMPLEMENTATION-MATRIX.toml`
  - `docs/FEATURE-STATUS.md`
  - `docs/SPEC-COVERAGE.md`
  - `docs/ENCODER-ROADMAP.md`
  - `docs/ENCODER-GAP-AUDIT.md`
  - `openspec/changes/encoder-block-trace-chroma-skip/**`
  - `openspec/changes/archive/2026-06-19-encoder-block-trace-chroma-skip/**`
  - `openspec/specs/encoder-tools/spec.md`
- Exact files/directories forbidden to this PR:
  - all other crates; `crates/splot-encode/src/error.rs` (no new error variants);
    `crates/splot-encode/src/intra_mode_emission.rs`; `crates/splot-encode/src/lib.rs`
  - workspace manifests and `Cargo.lock`; AV2 spec mirror under `docs/spec/av2/**`
- Public APIs/types owned: none
- Matrix rows owned: `ENC-INTRA-BLOCK-TRACE-CHROMA-SKIP`
- Generated files owned: `docs/FEATURE-STATUS.md`, `docs/SPEC-COVERAGE.md`
- Open sibling PRs audited: none open at base commit `0f451031`.
- Changed-file intersection with each sibling PR: none. If a decoder-mission PR
  lands first, sync `main`, regenerate the tracking docs, and re-gate.
- Semantic overlap with each sibling PR: none; private encoder composition.
- Can build/test/merge directly onto main without another open PR: yes.

## Risks / Trade-offs

- [Risk] Adding `VTxbSkip` to `CoefficientCdfRowSelector` touches the merged
  coefficient tokenization module. -> Mitigation: it is additive — the existing
  row router has a catch-all unsupported arm, and `syntax_name` is extended; all
  existing coefficient-tokenization tests still pass.
- [Risk] The trace is still all-zero only (no nonzero coefficients). ->
  Mitigation: this completes the minimal *all-zero* block symbol sequence; nonzero
  coefficient interleaving follows with its own Feature ID.
