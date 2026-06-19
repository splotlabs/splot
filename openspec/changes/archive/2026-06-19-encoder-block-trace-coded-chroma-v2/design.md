## Context

This re-does the closed PR #319 (coded chroma U DC) correctly on top of the merged
§8.2.5 bypass-literal token (`ENC-INTRA-BLOCK-TRACE-BYPASS-LITERAL`). AV2 §5.20.7.27
codes a chroma coefficient with the same CDF token shape as luma for `txb_skip` /
`eob_pt_16` / `coeff_base_eob`, but two facts differ and were missed in #319:

1. **The chroma DC sign is a `sign_bit L(1)` bypass literal, not a CDF symbol.**
   §5.20.7.27 (lines 15694-15726) reads `dc_sign S()` only for `row==0 && col==0 &&
   plane==0` (luma DC) and `dc_sign_horz_vert S()` for the directional luma axis
   coefficients (`TX_CLASS_HORIZ && col==0 && plane==0` / `TX_CLASS_VERT && row==0
   && plane==0`); every other sign — including a chroma DC — is `sign_bit L(1)`.
2. **The V `txb_skip` context gains `+6` once U is coded.** §8.3.2 (`all_zero`,
   lines 1257-1262): `if (plane==2) { if (bw*bh>w*h) ctx+=3; if (EobU!=0) ctx+=6 }`,
   and §5.20.5.3 sets `EobU = eob` for the coded U plane. With empty neighbours and
   `bw*bh==w*h`, the V context is `0 + 6 = 6`.

The remaining chroma CDF contexts (verified vs the decoder): U `txb_skip` reuses
the U all-zero row (`is_inter||fsc_mode` bank 0, ctx 6); `eob_pt_16` uses eob ctx 2
(`eobCtx=(plane>0)?2:is_inter`); `coeff_base_eob` uses the chroma
`TileCoeffBaseLfEobUvCdf` at DC ctx 0 (`coeff_base_eob_ctx` for c==0 =
`SIG_COEF_CONTEXTS_EOB - 4 = 0`).

The minimal coded chroma block codes a luma DC and a U DC (each magnitude `+1`)
with an all-zero V, giving the twelve-token trace `[0,0,0, 0,0,0,0, 0,0,0, 0(bypass),
1]` (modes; luma `txb_skip`/`eob_pt`/`base_eob`/`dc_sign`; U `txb_skip`/`eob_pt`/
`base_eob`; U `sign_bit` bypass; V `all_zero`).

Normative AV2 v1.0.0 sections:

- §5.20.7.27 `coeffs()` (`docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-27`).
- §8.3.2 chroma coefficient CDF contexts and the `all_zero` V EobU term.

## Goals / Non-Goals

**Goals:**

- Add `chroma_u_dc_coded_coeff_tokens` (3 CDF tokens, typed magnitude rejection)
  and `compose_minimal_intra_dc_coded_chroma_block_trace`.
- Code the U DC sign as a `Bypass` `sign_bit` literal; the V `txb_skip` at EobU
  ctx 6.
- Prove the twelve-token trace through one §8.2 coder with shared CDF state.
- Preserve the no-packet invariant.

**Non-Goals:**

- No chroma base-range (`coeff_br`) / golomb tiers, V-plane coded coefficients,
  multi-coefficient blocks, CfL/CCTX, partition syntax, tile CDF lifecycle,
  tile-body emission, packet output, CLI success, or Baseline Encoder Profile v1
  claim.
- No dependency graph change and no AVM/dav2d evidence.

## Decisions

1. **The chroma sign is a trace-level `Bypass` token, not a coefficient token.**
   `chroma_u_dc_coded_coeff_tokens` returns only the three CDF coefficient tokens;
   the compose appends `BlockSymbolToken::bypass(1, sign)` for the `sign_bit`
   literal. This matches §5.20.7.27 and reuses the merged bypass mechanism.

2. **Typed magnitude rejection.** `chroma_u_dc_coded_coeff_tokens` returns a
   `CoefficientTokenizationUnsupportedChromaMagnitude` error for magnitude 0 or
   `> MAX_BASE_EOB_MAGNITUDE` (a block-free variant, since the accessor has no input
   rect), per the #319 review.

3. **V `txb_skip` at EobU ctx 6.** The coded U sets `EobU != 0`, so the V all-zero
   `txb_skip` uses `DEFAULT_V_TXB_SKIP_CDF[q][6]`, not the all-zero-U neutral ctx 0.

## Flight Manifest

- Change ID: `encoder-block-trace-coded-chroma-v2`
- Feature IDs: `ENC-INTRA-BLOCK-TRACE-CODED-CHROMA-DC`
- Base commit: `f8d81edd` (`feat(encode): bypass-literal block-symbol token (§8.2.5 L(n)) (#321)`)
- Depends on merged changes: `encoder-block-trace-bypass-literal`,
  `encoder-block-trace-coded-dc`, `encoder-block-trace-chroma-skip`,
  `encoder-coefficient-tokenization-minimal`
- Supersedes: the closed (not merged) PR #319 / `encoder-block-trace-coded-chroma`.
- Exact files/directories owned by this PR:
  - `crates/splot-encode/src/coefficient_tokenization.rs`
  - `crates/splot-encode/src/coefficient_tokenization_tests.rs`
  - `crates/splot-encode/src/block_symbol_trace.rs`
  - `crates/splot-encode/src/error.rs` (one new `CoefficientTokenizationUnsupportedChromaMagnitude` variant)
  - `docs/IMPLEMENTATION-MATRIX.toml`
  - `docs/FEATURE-STATUS.md`
  - `docs/SPEC-COVERAGE.md`
  - `docs/ENCODER-ROADMAP.md`
  - `docs/ENCODER-GAP-AUDIT.md`
  - `openspec/changes/encoder-block-trace-coded-chroma-v2/**`
  - `openspec/changes/archive/2026-06-19-encoder-block-trace-coded-chroma-v2/**`
  - `openspec/specs/encoder-tools/spec.md`
- Exact files/directories forbidden to this PR:
  - all other crates; `crates/splot-encode/src/intra_mode_emission.rs`;
    `crates/splot-encode/src/lib.rs`; `crates/splot-encode/src/closed_loop.rs`
  - workspace manifests and `Cargo.lock`; AV2 spec mirror under `docs/spec/av2/**`
- Public APIs/types owned: none
- Matrix rows owned: `ENC-INTRA-BLOCK-TRACE-CODED-CHROMA-DC`
- Generated files owned: `docs/FEATURE-STATUS.md`, `docs/SPEC-COVERAGE.md`
- Open sibling PRs audited: none open at base commit `f8d81edd`.
- Changed-file intersection with each sibling PR: none. If a decoder-mission PR
  lands first, sync `main`, regenerate the tracking docs, and re-gate.
- Semantic overlap with each sibling PR: none; private encoder composition.
- Can build/test/merge directly onto main without another open PR: yes.

## Risks / Trade-offs

- [Risk] Chroma context derivation is error-prone (the #313/#319 lessons). ->
  Mitigation: all contexts (U txb_skip bank 0/ctx 6, eob ctx 2, base-eob-uv ctx 0,
  V EobU ctx 6) and the sign-literal path are verified against the spec mirror and
  the decoder, and proven by the §8.2 roundtrip; the magnitude rejection is tested.
- [Risk] No equivalence-test reference (the tokenizer is luma-only). -> Mitigation:
  the block-trace roundtrip plus the chroma-context unit test are the proof, as
  with the merged chroma all-zero tokens.
