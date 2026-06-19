## Context

The coded luma DC trace supports magnitude 1..7. Magnitude 8 reaches the §5.20.7.27
`maxLevel = LF_NUM_BASE_LEVELS + COEFF_BASE_RANGE + 1 = 8` (the level from
`coeff_base_eob + 1 = 5` plus `coeff_br = 3`), at which §5.20.7.28 `read_quant`
emits the golomb tail. The merged §8.2.5 bypass-literal token now lets the trace
carry the golomb `coeff_rem` bits.

`read_quant` golomb derivation for the first/only DC coefficient (verified vs the
decoder `read_quant.rs` and the spec):

- `hrLevelAvg = 0` at block entry (§5.20.7.27 init), so `predLevel = 0`,
  `m = Clip3(1, 6, GetMsb(0)) = 1`, `k = m + 1 = 2`, `cMax = Min(m + 4, 6) = 5`.
- Finite-q path (`q < cMax`): the `q_length_bit` loop reads `q` zeros then a `1`;
  `length = m = 1`; `xBase = q << 1`; `coeff_rem` is `L(1)`; `x = xBase + coeff_rem
  = 2q + coeff_rem`. So `q = x >> 1`, `coeff_rem = x & 1`, covering `x` in `0..=9`
  (`q` in `0..=4`), i.e. magnitude `maxLevel..=maxLevel + 9 = 8..=17`.
- The golomb-prefix path (`q == cMax`, x ≥ 10, magnitude 18+) is a later brick.

The level tokens before the golomb tail are fixed for the golomb tier:
`all_zero = 0`, `eob_pt_16 = 0`, `coeff_base_eob = LF_NUM_BASE_LEVELS (4)` (level 5,
its max), `coeff_br = COEFF_BASE_RANGE (3)` (level reaches `maxLevel = 8`). The
sign+quant pass reads the sign before calling `read_quant`, so the luma DC
`dc_sign` CDF token precedes the golomb bits.

For magnitude `+10`: `x = 2`, `q = 1`, `coeff_rem = 0` → golomb bypass bits `0`
(one `q_length_bit` zero), `1` (terminating `q_length_bit`), `0` (`coeff_rem`),
emitted after the `dc_sign` token. The thirteen-token trace is
`[0,0,0, 0,0,4,3, 0, 0,1,0, 1,1]`.

Normative AV2 v1.0.0 sections:

- §5.20.7.27 `coeffs()` `maxLevel` (`docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-27`).
- §5.20.7.28 `read_quant()` golomb tail (`#s-5-20-7-28`).
- §8.2.5 bypass `L(n)` literals.

## Goals / Non-Goals

**Goals:**

- Add `luma_dc_golomb_level_tokens` + `luma_dc_sign_token` and
  `compose_minimal_intra_dc_golomb_block_trace` for the finite-q golomb tail
  (magnitude 8..17).
- Prove the trace through one §8.2 coder, and that the decoded golomb bits
  reconstruct the encoded magnitude via the `read_quant` finite-q arithmetic.
- Preserve the no-packet invariant.

**Non-Goals:**

- No golomb-prefix tail (magnitude 18+), multi-coefficient blocks, higher-frequency
  coefficients, chroma golomb, partition syntax, tile CDF lifecycle, tile-body
  emission, packet output, CLI success, or Baseline Encoder Profile v1 claim.
- No dependency graph change and no AVM/dav2d evidence.

## Decisions

1. **Fixed-magnitude compose with a conformance reconstruction test.** The compose
   uses a const magnitude (`+10`), so no runtime magnitude validation is needed
   (a `debug_assert` documents the finite-q bound). Because the §8.2 roundtrip
   alone only proves the bits are self-consistent, a test reconstructs `x` from the
   decoded bypass bits via the decoder's finite-q algorithm and asserts it yields
   the encoded magnitude — the conformance proof.

2. **The golomb bits are individual 1-bit `Bypass` tokens.** The `q_length` unary
   (q zeros + a 1) and the `coeff_rem` bit are emitted as `BlockSymbolToken::bypass(1,
   bit)` tokens, avoiding bit-packing arithmetic and reusing the merged bypass path.

3. **`luma_dc_sign_token` factored out.** The luma DC `dc_sign` CDF token is
   extracted so both `luma_dc_coded_tokens` and the golomb compose share it; the
   golomb tail appends the `coeff_rem` bypass bits after the `dc_sign` token,
   because §5.20.7.27's sign+quant pass reads the sign before calling
   §5.20.7.28 `read_quant` (the level tokens, then `dc_sign`, then the golomb bits).

## Flight Manifest

- Change ID: `encoder-block-trace-golomb-finite`
- Feature IDs: `ENC-INTRA-BLOCK-TRACE-GOLOMB-FINITE`
- Base commit: `34c1929d` (`feat(encode): coded chroma intra block trace, redone on bypass literals (#323)`)
- Depends on merged changes: `encoder-block-trace-bypass-literal`,
  `encoder-block-trace-coded-br`, `encoder-block-trace-coded-dc`
- Exact files/directories owned by this PR:
  - `crates/splot-encode/src/coefficient_tokenization.rs`
  - `crates/splot-encode/src/block_symbol_trace.rs`
  - `docs/IMPLEMENTATION-MATRIX.toml`
  - `docs/FEATURE-STATUS.md`
  - `docs/SPEC-COVERAGE.md`
  - `docs/ENCODER-ROADMAP.md`
  - `docs/ENCODER-GAP-AUDIT.md`
  - `openspec/changes/encoder-block-trace-golomb-finite/**`
  - `openspec/changes/archive/2026-06-19-encoder-block-trace-golomb-finite/**`
  - `openspec/specs/encoder-tools/spec.md`
- Exact files/directories forbidden to this PR:
  - all other crates; `crates/splot-encode/src/error.rs` (no new variant);
    `crates/splot-encode/src/intra_mode_emission.rs`; `crates/splot-encode/src/lib.rs`;
    `crates/splot-encode/src/closed_loop.rs`
  - workspace manifests and `Cargo.lock`; AV2 spec mirror under `docs/spec/av2/**`
- Public APIs/types owned: none
- Matrix rows owned: `ENC-INTRA-BLOCK-TRACE-GOLOMB-FINITE`
- Generated files owned: `docs/FEATURE-STATUS.md`, `docs/SPEC-COVERAGE.md`
- Open sibling PRs audited: none open at base commit `34c1929d`.
- Changed-file intersection with each sibling PR: none. If a decoder-mission PR
  lands first, sync `main`, regenerate the tracking docs, and re-gate.
- Semantic overlap with each sibling PR: none; private encoder composition.
- Can build/test/merge directly onto main without another open PR: yes.

## Risks / Trade-offs

- [Risk] The golomb encoding is intricate and stateful (`hrLevelAvg`/`m`/`k`). ->
  Mitigation: `hrLevelAvg = 0` for the single DC gives a fixed `m = 1`; the
  derivation is verified against the decoder's `read_quant`, and the conformance
  test decodes the bits back to the magnitude.
- [Risk] The §8.2 roundtrip only proves the bits are self-consistent. ->
  Mitigation: the reconstruction test runs the decoder's finite-q arithmetic on
  the decoded bits and asserts the encoded magnitude.
