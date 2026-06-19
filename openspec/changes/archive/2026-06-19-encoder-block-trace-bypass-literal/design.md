## Context

The block-symbol trace models only CDF-coded `S()` symbols (`Mode` / `Coeff`). AV2
codes some syntax as `L(n)` bypass literals with no CDF:

- A non-luma-DC coefficient's sign: §5.20.7.27 reads the `dc_sign` CDF symbol only
  when `row == 0 && col == 0 && plane == 0` (and `dc_sign_horz_vert` for the luma
  H/V classes); every other coefficient — all chroma signs, non-DC luma signs —
  falls to `else { sign_bit L(1) }`.
- The §5.20.7.28 `read_quant` golomb tail (`q_length_bit` / `golomb_length_bit` /
  `coeff_rem`) is all `L(1)` / `L(n)` literals.

So coded chroma DC (its sign is a literal) and large-magnitude coefficients (the
golomb tail) cannot be represented without a bypass-literal token. The `splot-core`
coder already provides the primitives: `SymbolEncoder::write_literal(value, n)`
(MSB-first) and `SymbolDecoder::read_literal(n)` (exact inverse).

## Goals / Non-Goals

**Goals:**

- Add `BlockSymbolToken::Bypass { width, value }` and route it through
  `roundtrip_block_symbol_trace` via `write_literal` / `read_literal`.
- Prove bypass literals interleave with CDF symbols through one §8.2 coder.
- Preserve the no-packet invariant.

**Non-Goals:**

- No consumer yet (coded chroma signs, the golomb tail, multi-coefficient signs);
  those are follow-on bricks that build on this token kind.
- No partition syntax, tile CDF lifecycle, tile-body emission, packet output, CLI
  success, or Baseline Encoder Profile v1 claim.

## Decisions

1. **A new `BlockSymbolToken` variant, not a `CoefficientEntropyToken` change.**
   The bypass literal is added at the trace-token level (`BlockSymbolToken::Bypass`)
   rather than enum-ifying `CoefficientEntropyToken` (which carries a CDF selector +
   u8 symbol). This keeps the coefficient-token model unchanged and lets the
   roundtrip dispatch literals before CDF-row selection. The `width: u32, value:
   u32` shape covers both a 1-bit `sign_bit` and a wider golomb `coeff_rem`.

2. **Dispatch before `row_mut`.** A bypass literal has no CDF row, so the roundtrip
   matches `Bypass` first and calls `write_literal`/`read_literal`; `row_mut` gains
   an unreachable `Bypass` arm only for match exhaustiveness.

3. **No per-token literal mismatch check.** `read_literal` of what `write_literal`
   wrote is deterministic (no CDF state), so the bypass decode returns the value
   directly; the roundtrip tests assert the decoded values equal the encoded ones.
   (`write_literal` already rejects a value that does not fit its width, so a
   divergence is unreachable.)

4. **Foundation-only, with a synthetic proof.** Like the decoder mission's
   loaded-but-unread CDF-row bricks, this lands the mechanism before its first real
   consumer; the proof is a mixed CDF+bypass trace roundtrip. The next bricks (coded
   chroma DC sign, golomb tail) supply the real consumers.

## Flight Manifest

- Change ID: `encoder-block-trace-bypass-literal`
- Feature IDs: `ENC-INTRA-BLOCK-TRACE-BYPASS-LITERAL`
- Base commit: `567e77be` (current `main`)
- Depends on merged changes: `encoder-block-trace-luma-skip` (the unified
  `BlockSymbolToken` + `roundtrip_block_symbol_trace`)
- Exact files/directories owned by this PR:
  - `crates/splot-encode/src/block_symbol_trace.rs`
  - `docs/IMPLEMENTATION-MATRIX.toml`
  - `docs/FEATURE-STATUS.md`
  - `docs/SPEC-COVERAGE.md`
  - `docs/ENCODER-ROADMAP.md`
  - `docs/ENCODER-GAP-AUDIT.md`
  - `openspec/changes/encoder-block-trace-bypass-literal/**`
  - `openspec/changes/archive/2026-06-19-encoder-block-trace-bypass-literal/**`
  - `openspec/specs/encoder-tools/spec.md`
- Exact files/directories forbidden to this PR:
  - all other crates; `crates/splot-encode/src/coefficient_tokenization.rs`;
    `crates/splot-encode/src/intra_mode_emission.rs`; `crates/splot-encode/src/lib.rs`;
    `crates/splot-encode/src/closed_loop.rs`
  - workspace manifests and `Cargo.lock`; AV2 spec mirror under `docs/spec/av2/**`
  - NOTE: `crates/splot-encode/src/error.rs` is touched only to reuse existing
    variants (no new variant); the bypass path reuses `BlockSymbolTraceSymbolWrite`
    / `BlockSymbolTraceSymbolRead`.
- Public APIs/types owned: none
- Matrix rows owned: `ENC-INTRA-BLOCK-TRACE-BYPASS-LITERAL`
- Generated files owned: `docs/FEATURE-STATUS.md`, `docs/SPEC-COVERAGE.md`
- Open sibling PRs audited: none open at base commit `567e77be`.
- Changed-file intersection with each sibling PR: none. If a decoder-mission PR
  lands first, sync `main`, regenerate the tracking docs, and re-gate.
- Semantic overlap with each sibling PR: none; private encoder composition.
- Can build/test/merge directly onto main without another open PR: yes.

## Risks / Trade-offs

- [Risk] A foundation with no consumer can read as dead code. -> Mitigation: it is
  proven by a real §8.2 roundtrip test, the established "land the mechanism first"
  pattern, and the next bricks (coded chroma sign, golomb) consume it directly.
- [Risk] The `symbol()` view returns `value as u8` for a wide literal. ->
  Mitigation: `symbol()` is the CDF-symbol view; the bypass roundtrip uses
  `width`/`value` directly, not `symbol()`, and the tests use small literals.
