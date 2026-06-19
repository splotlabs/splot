## Context

The unified `block_symbol_trace` composes the all-zero intra block (every plane's
`txb_skip == 1`, no coefficients). A real intra frame codes coefficients, so this
change adds the minimal *coded* block: a single luma DC coefficient. Per AV2
§5.20.7.27 `residual()`/`coeffs()`, a coded luma block reads `txb_skip == 0` then,
for a DC-only end-of-block, `eob_pt_16`, `coeff_base_eob`, and `dc_sign`. The
chroma planes stay all-zero.

`coefficient_tokenization::tokenize_coefficients` already produces exactly these
four tokens for a nonzero DC-only 4x4 luma block and roundtrips them in isolation.
This change exposes that token shape as a `pub(crate)` accessor for the trace and
integrates it into the unified roundtrip.

Coded-DC CDF selection (defaults from `splot-core`, neutral top-left luma):

- `txb_skip` (coded) reuses the existing luma `txb_skip` row
  (`DEFAULT_TXB_SKIP_CDF[0][0][0][0]`); only the symbol differs (0 vs 1).
- `eob_pt_16` → `DEFAULT_EOB_PT_16_CDF[0][0]` (q-ctx 0, luma-intra eob-ctx 0).
- `coeff_base_eob` → `DEFAULT_COEFF_BASE_LF_EOB_CDF[0][0][0]` (q-ctx 0, 4x4, DC
  ctx 0).
- `dc_sign` → `DEFAULT_DC_SIGN_CDF[0][0][0][0]` (q-ctx 0, luma plane-type 0, group
  0, ctx 0).

For a DC coefficient of value `+1` (magnitude 1, positive): `txb_skip=0`,
`eob_pt_16=0`, `coeff_base_eob=magnitude-1=0`, `dc_sign=0` — so the full nine-symbol
trace is `[0,0,0, 0,0,0,0, 1,1]`.

Normative AV2 v1.0.0 sections:

- §5.20.5.3 `intra_frame_mode_info()`
  (`docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-20-5-3`).
- §5.20.7.27 `coeffs()` coded path (`#s-5-20-7-27`).
- §8.3.2 coefficient CDF context derivation.

## Goals / Non-Goals

**Goals:**

- Add `compose_minimal_intra_dc_coded_block_trace` for
  `ENC-INTRA-BLOCK-TRACE-CODED-DC`: the mode prefix, coded luma DC residual, and
  all-zero U/V `txb_skip`.
- Add a `pub(crate)` `luma_dc_coded_tokens` accessor mirroring the tokenizer's
  coded DC path, guarded by an equivalence test against `tokenize_coefficients`.
- Prove the nine-symbol trace through one §8.2 coder with shared CDF state.
- Preserve the no-packet invariant.

**Non-Goals:**

- No multi-coefficient blocks, `coeff_br` base-range / higher-frequency /
  sign-golomb extension, chroma coefficients, CfL/CCTX, partition syntax, tile CDF
  lifecycle, tile-body emission, packet output, CLI success, or Baseline Encoder
  Profile v1 claim.
- No dependency graph change and no AVM/dav2d evidence.

## Decisions

1. **Accessor mirrors the tokenizer, guarded by an equivalence test.** The module
   already exposes hand-built `const fn` token constructors (`luma_all_zero_token`,
   `chroma_u/v_all_zero_token`) rather than driving `tokenize_coefficients`.
   `luma_dc_coded_tokens` follows that pattern for the coded DC tokens, and a new
   `coded_dc_tokens_match_tokenizer` test asserts it equals
   `tokenize_coefficients` across the supported magnitude/sign range so the two
   never drift.

2. **The coded `txb_skip` reuses the luma `txb_skip` row.** A coded block's
   `txb_skip == 0` and an all-zero block's `txb_skip == 1` share the same CDF row
   (`TxbSkip` selector with luma bank, ctx 0); only the symbol differs. No new
   routing arm is needed for the coded `txb_skip` — only `eob_pt_16`,
   `coeff_base_lf_eob`, and `dc_sign` add rows.

3. **Minimal coded coefficient is a positive unit DC.** Magnitude 1, positive
   gives all-zero symbols for the coded luma part, the simplest coded block. The
   accessor is parameterized over magnitude/sign for generality and tested across
   the range; the trace uses the unit case.

## Flight Manifest

- Change ID: `encoder-block-trace-coded-dc`
- Feature IDs: `ENC-INTRA-BLOCK-TRACE-CODED-DC`
- Base commit: `5d50a3fa` (`feat(encode): complete all-zero block trace with chroma txb_skip (#313)`)
- Depends on merged changes: `encoder-block-trace-chroma-skip`,
  `encoder-coefficient-tokenization-minimal`
- Exact files/directories owned by this PR:
  - `crates/splot-encode/src/block_symbol_trace.rs`
  - `crates/splot-encode/src/coefficient_tokenization.rs`
  - `docs/IMPLEMENTATION-MATRIX.toml`
  - `docs/FEATURE-STATUS.md`
  - `docs/SPEC-COVERAGE.md`
  - `docs/ENCODER-ROADMAP.md`
  - `docs/ENCODER-GAP-AUDIT.md`
  - `openspec/changes/encoder-block-trace-coded-dc/**`
  - `openspec/changes/archive/2026-06-19-encoder-block-trace-coded-dc/**`
  - `openspec/specs/encoder-tools/spec.md`
- Exact files/directories forbidden to this PR:
  - all other crates; `crates/splot-encode/src/error.rs`;
    `crates/splot-encode/src/intra_mode_emission.rs`; `crates/splot-encode/src/lib.rs`
  - workspace manifests and `Cargo.lock`; AV2 spec mirror under `docs/spec/av2/**`
- Public APIs/types owned: none
- Matrix rows owned: `ENC-INTRA-BLOCK-TRACE-CODED-DC`
- Generated files owned: `docs/FEATURE-STATUS.md`, `docs/SPEC-COVERAGE.md`
- Open sibling PRs audited: none open at base commit `5d50a3fa`.
- Changed-file intersection with each sibling PR: none. If a decoder-mission PR
  lands first, sync `main`, regenerate the tracking docs, and re-gate.
- Semantic overlap with each sibling PR: none; private encoder composition.
- Can build/test/merge directly onto main without another open PR: yes.

## Risks / Trade-offs

- [Risk] `luma_dc_coded_tokens` duplicates the tokenizer's token shape. ->
  Mitigation: the `coded_dc_tokens_match_tokenizer` equivalence test fails if they
  ever diverge.
- [Risk] The trace is still a single 4x4 block, not a full tile. -> Mitigation:
  this is the minimal *coded* block sub-unit; partition syntax, multi-coefficient
  blocks, and tile-body assembly follow with their own Feature IDs.
