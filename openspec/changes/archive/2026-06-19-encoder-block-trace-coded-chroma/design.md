## Context

The coded DC/base-range traces code only luma coefficients (chroma planes stay
all-zero). A real intra frame codes chroma coefficients, so this change adds the
minimal coded U-plane DC coefficient. AV2 §5.20.7.27 codes a chroma coefficient
with the same token shape as luma (`txb_skip`, `eob_pt_16`, `coeff_base_eob`,
`dc_sign`) but with §8.3.2 chroma contexts:

- **U `txb_skip == 0`** reuses `TileTxbSkipCdf` at the same `is_inter || fsc_mode`
  bank 0 as luma, U context 6 (the row the U all-zero token already uses; only the
  symbol differs).
- **`eob_pt_16`** uses eob context `2` — §5.20.7.27 line 15362:
  `eobCtx = (plane > 0) ? 2 : is_inter`, so intra chroma → 2.
- **`coeff_base_eob`** uses the dedicated chroma `TileCoeffBaseLfEobUvCdf` at the
  DC context 0 (`coeff_base_eob_ctx` returns `SIG_COEF_CONTEXTS_EOB - 4 = 0` for
  `c == 0`, plane-independent).
- **`dc_sign`** uses `TileDcSignCdf[ptype][isHidden][ctx]` with `ptype = (plane > 0)
  = 1` for chroma, `isHidden = 0`, ctx 0.

All four contexts are verified against the decoder (`base_level_pass.rs::base_eob_selector`,
`coeff_context.rs::coeff_base_eob_ctx` / `dc_sign` doc).

The minimal coded chroma block codes a luma DC and a U DC (each magnitude `+1`)
with an all-zero V plane, giving the twelve-symbol trace
`[0,0,0, 0,0,0,0, 0,0,0,0, 1]` (modes; luma `txb_skip`/`eob_pt`/`base_eob`/`dc_sign`;
U same; V `all_zero = 1`).

Normative AV2 v1.0.0 sections:

- §5.20.7.27 `coeffs()` (`docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-27`).
- §8.3.2 chroma coefficient CDF contexts.

## Goals / Non-Goals

**Goals:**

- Add `chroma_u_dc_coded_tokens` and `compose_minimal_intra_dc_coded_chroma_block_trace`
  for `ENC-INTRA-BLOCK-TRACE-CODED-CHROMA-DC`.
- Add the `CoeffBaseLfEobUv` selector and the chroma CDF rows to the unified
  roundtrip.
- Prove the twelve-symbol trace through one §8.2 coder with shared CDF state.
- Preserve the no-packet invariant.

**Non-Goals:**

- No chroma base-range (`coeff_br`) / golomb tiers, V-plane coded coefficients,
  multi-coefficient blocks, CfL/CCTX, partition syntax, tile CDF lifecycle,
  tile-body emission, packet output, CLI success, or Baseline Encoder Profile v1
  claim.
- No dependency graph change and no AVM/dav2d evidence.

## Decisions

1. **Hand-built chroma accessor (no tokenizer reference).** `tokenize_coefficients`
   is luma-only, so — as with the chroma all-zero tokens — the chroma coded-DC
   accessor is hand-built and verified by the §8.2 roundtrip plus the spec/decoder
   context citations (rather than an equivalence test). Base tier only (magnitude
   1..=4); the chroma base-range/golomb tiers are later bricks.

2. **The coded U `txb_skip` reuses the U all-zero row.** A coded U block's
   `txb_skip == 0` and an all-zero U block's `txb_skip == 1` share the same §8.3.2
   row (`TxbSkip` bank 0, ctx 6); only the symbol differs. No new routing arm is
   needed for the coded U `txb_skip` — only `eob_pt_16` (ctx 2),
   `coeff_base_lf_eob_uv`, and chroma `dc_sign` (ptype 1) add rows.

3. **Trace codes both luma and U.** The block codes a luma DC and a U DC with an
   all-zero V, so the trace exercises the per-plane `residual()` ordering Y, U, V
   with two coded planes.

## Flight Manifest

- Change ID: `encoder-block-trace-coded-chroma`
- Feature IDs: `ENC-INTRA-BLOCK-TRACE-CODED-CHROMA-DC`
- Base commit: `1fa35bce` (`feat(encode): coded base-range intra block trace (coeff_br) (#316)`)
- Depends on merged changes: `encoder-block-trace-coded-dc`,
  `encoder-block-trace-chroma-skip`, `encoder-coefficient-tokenization-minimal`
- Exact files/directories owned by this PR:
  - `crates/splot-encode/src/coefficient_tokenization.rs`
  - `crates/splot-encode/src/block_symbol_trace.rs`
  - `docs/IMPLEMENTATION-MATRIX.toml`
  - `docs/FEATURE-STATUS.md`
  - `docs/SPEC-COVERAGE.md`
  - `docs/ENCODER-ROADMAP.md`
  - `docs/ENCODER-GAP-AUDIT.md`
  - `openspec/changes/encoder-block-trace-coded-chroma/**`
  - `openspec/changes/archive/2026-06-19-encoder-block-trace-coded-chroma/**`
  - `openspec/specs/encoder-tools/spec.md`
- Exact files/directories forbidden to this PR:
  - all other crates; `crates/splot-encode/src/error.rs` (no new variants);
    `crates/splot-encode/src/closed_loop.rs`;
    `crates/splot-encode/src/intra_mode_emission.rs`; `crates/splot-encode/src/lib.rs`
  - workspace manifests and `Cargo.lock`; AV2 spec mirror under `docs/spec/av2/**`
- Public APIs/types owned: none
- Matrix rows owned: `ENC-INTRA-BLOCK-TRACE-CODED-CHROMA-DC`
- Generated files owned: `docs/FEATURE-STATUS.md`, `docs/SPEC-COVERAGE.md`
- Open sibling PRs audited: none open at base commit `1fa35bce`.
- Changed-file intersection with each sibling PR: none. If a decoder-mission PR
  lands first, sync `main`, regenerate the tracking docs, and re-gate.
- Semantic overlap with each sibling PR: none; private encoder composition.
- Can build/test/merge directly onto main without another open PR: yes.

## Risks / Trade-offs

- [Risk] Chroma context derivation is easy to get wrong (the #313 `is_inter||fsc_mode`
  lesson). -> Mitigation: all four chroma contexts (eob ctx 2, base-eob-uv ctx 0,
  dc_sign ptype 1, U txb_skip bank 0 / ctx 6) are verified against the spec mirror
  and the decoder, and proven by the §8.2 roundtrip.
- [Risk] No equivalence-test reference for the chroma accessor. -> Mitigation: the
  block-trace roundtrip is the proof, as with the merged chroma all-zero tokens.
