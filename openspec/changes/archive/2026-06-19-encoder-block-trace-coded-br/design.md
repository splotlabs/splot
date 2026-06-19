## Context

`ENC-INTRA-BLOCK-TRACE-CODED-DC` tokenizes a single nonzero luma DC coefficient of
magnitude 1..=4 (`coeff_base_eob` base tier). AV2 § 5.20.7.27 codes a low-frequency
EOB coefficient whose base level exceeds `LF_NUM_BASE_LEVELS` with an additional
`coeff_br` (base-range) symbol:

```text
coeff_base_eob        S()        level = coeff_base_eob + 1
baseLevels = isLf ? LF_NUM_BASE_LEVELS : NUM_BASE_LEVELS    // LF luma → 4
if ( level > baseLevels && !(isLf && plane > 0) ) {
    coeff_br          S()        level += coeff_br
}
```

`coeff_base_eob` is 5-ary (symbols 0..=4 → level 1..=5; the `TileCoeffBaseLfEobCdf`
row has 6 entries → 5 symbols). So for the DC EOB coefficient, `coeff_br` is read
iff `coeff_base_eob == 4` (level 5), and `coeff_br ∈ 0..=COEFF_BASE_RANGE (3)` adds
to the level, giving final magnitude 5..=7 (the cap; see below).

CDF selection (verified against the decoder, `base_level_pass.rs:base_range_selector`
+ `CoeffBrContext::ctx`): for a luma (plane 0) low-frequency coefficient the
selector is `BrLf { coeff_cdf_q_ctx, ctx }` → `TileCoeffBrLfCdf[q][ctx]`. For the
DC (`pos == 0`, empty `Level[]`, 2D), `CoeffBrContext::ctx` returns mag 0 → **ctx 0**
(`pos == 0` non-IDTX branch returns `mag`). So the row is `DEFAULT_COEFF_BR_LF_CDF[q][0]`
(5-entry row → 4 symbols 0..=3).

Concrete magnitudes (single luma DC EOB, neutral ctx):

- magnitude 4: `coeff_base_eob = 3` (level 4, not > 4) → no `coeff_br`.
- magnitude 5: `coeff_base_eob = 4` (level 5) → `coeff_br = 0`.
- magnitude 6: `coeff_base_eob = 4`, `coeff_br = 1`.
- magnitude 8: `coeff_base_eob = 4`, `coeff_br = 3` → level 8 == `maxLevel`
  (`LF_NUM_BASE_LEVELS + COEFF_BASE_RANGE + 1`), so §5.20.7.28 `read_quant` emits the
  golomb tail (TCQ off → `quant >= maxLevel`); magnitude 8 is REJECTED until that
  brick. The cap is therefore `LF_NUM_BASE_LEVELS + COEFF_BASE_RANGE = 7`.

Normative AV2 v1.0.0 sections:

- §5.20.7.27 `coeffs()` (`docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-27`).
- §8.3.2 `coeff_br` CDF context (the low-frequency `BrLf` bank; DC ctx 0).

## Goals / Non-Goals

**Goals:**

- Add `coeff_br` tokenization for the coded luma DC base-range tier (magnitude
  5..=7) and `compose_minimal_intra_dc_br_block_trace` for the trace.
- Unify the coded DC token shape in `luma_dc_coded_tokens` (the tokenizer
  delegates to it), guarded by the equivalence test over 1..=7.
- Prove the ten-symbol base-range trace through one §8.2 coder with shared CDF
  state.
- Preserve the no-packet invariant.

**Non-Goals:**

- No golomb (exp-golomb) tail for magnitudes > 8, multi-coefficient blocks,
  higher-frequency coefficients, chroma coefficients, partition syntax, tile CDF
  lifecycle, tile-body emission, packet output, CLI success, or Baseline Encoder
  Profile v1 claim.
- No dependency graph change and no AVM/dav2d evidence.

## Decisions

1. **`luma_dc_coded_tokens` becomes the single source.** It now returns a
   variable-length `Result<Vec<CoefficientEntropyToken>>` (4 tokens base tier, 5
   with `coeff_br`), and `tokenize_coefficients` delegates to it after validating
   the magnitude cap. The `coded_dc_tokens_match_tokenizer` test (now over 1..=7)
   guarantees the trace accessor and the tokenizer cannot drift.

2. **`coeff_br` uses the low-frequency `BrLf` CDF at ctx 0.** Verified against the
   decoder: a luma LF coefficient uses `TileCoeffBrLfCdf`, and the DC's
   `CoeffBrContext::ctx` is 0 (empty `Level[]`, `pos == 0`). The minimal trace
   uses magnitude 6 (`coeff_br = 1`) to exercise a non-zero base-range symbol.

3. **`closed_loop` DC recovery accumulates `coeff_br`.** The new `CoeffBr` syntax
   variant forces the `closed_loop` test helper's exhaustive match to handle it;
   the spec-correct behavior is `level += coeff_br`.

## Flight Manifest

- Change ID: `encoder-block-trace-coded-br`
- Feature IDs: `ENC-INTRA-BLOCK-TRACE-CODED-BR`
- Base commit: `2ea06196` (`feat(encode): minimal coded intra block trace (DC coefficient) (#314)`)
- Depends on merged changes: `encoder-block-trace-coded-dc`,
  `encoder-coefficient-tokenization-minimal`
- Exact files/directories owned by this PR:
  - `crates/splot-encode/src/coefficient_tokenization.rs`
  - `crates/splot-encode/src/block_symbol_trace.rs`
  - `crates/splot-encode/src/closed_loop.rs` (one-line exhaustive-match arm only)
  - `docs/IMPLEMENTATION-MATRIX.toml`
  - `docs/FEATURE-STATUS.md`
  - `docs/SPEC-COVERAGE.md`
  - `docs/ENCODER-ROADMAP.md`
  - `docs/ENCODER-GAP-AUDIT.md`
  - `openspec/changes/encoder-block-trace-coded-br/**`
  - `openspec/changes/archive/2026-06-19-encoder-block-trace-coded-br/**`
  - `openspec/specs/encoder-tools/spec.md`
- Exact files/directories forbidden to this PR:
  - all other crates; `crates/splot-encode/src/error.rs` (no new variants);
    `crates/splot-encode/src/intra_mode_emission.rs`; `crates/splot-encode/src/lib.rs`
  - workspace manifests and `Cargo.lock`; AV2 spec mirror under `docs/spec/av2/**`
- Public APIs/types owned: none
- Matrix rows owned: `ENC-INTRA-BLOCK-TRACE-CODED-BR`
- Generated files owned: `docs/FEATURE-STATUS.md`, `docs/SPEC-COVERAGE.md`
- Open sibling PRs audited: none open at base commit `2ea06196`.
- Changed-file intersection with each sibling PR: none. If a decoder-mission PR
  lands first, sync `main`, regenerate the tracking docs, and re-gate.
- Semantic overlap with each sibling PR: none; private encoder composition.
- Can build/test/merge directly onto main without another open PR: yes.

## Risks / Trade-offs

- [Risk] Raising the tokenizer magnitude cap changes which quantized inputs are
  accepted (5..=7 now tokenize instead of erroring). -> Mitigation: a strict
  extension; the rejection test now asserts the new cap (8), and all existing
  base-tier tests still pass.
- [Risk] `luma_dc_coded_tokens` and `tokenize_coefficients` duplicate the br
  logic. -> Mitigation: `tokenize_coefficients` delegates to the accessor, and the
  equivalence test over 1..=7 fails on any divergence.
