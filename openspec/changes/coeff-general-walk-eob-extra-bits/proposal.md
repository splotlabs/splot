## Why

Sub-brick 5c-ii. The general LF tokenizer reached eob 4. This lifts it to **eob 10** —
the full 4x4 low-frequency region — adding the §8.2.5 `eob_extra_bit` bypass literals
for eobPt 4 and 5.

The `eob_extra_bit` order is load-bearing and the §8.2 self-consistency roundtrip
CANNOT catch a reversal (the encoder and its own recovery would agree while
mis-decoding against a real decoder). So the order is mirrored from the spec loop and
the decoder `read_literal` (both MSB-first), and a 5-agent adversarial review
confirmed it (eob=10's asymmetric `[0,1]` would mis-decode to 11 if reversed).

## What Changes

- Add `ENC-COEFF-GENERAL-WALK-EOB-EXTRA-BITS` as a private `splot-encode`
  encoder-tool feature.
- Lift `tokenize_general_lf_luma_block` to eob 5..=10: eob 5-8 → eobPt 4 (1
  `eob_extra_bit`), eob 9-10 → eobPt 5 (2 `eob_extra_bit`s), emitted MSB-first via the
  bypass token; extend `recover_quant_from_tokens` to read them back. Reject a nonzero
  at scan index ≥ 10 (the high-frequency region, a later sub-brick).
- Verify the LF boundary (`row + col < 4`, not scan index) so eob 1-10 are entirely
  low-frequency and reuse the existing hole-free `[q][ctx]` banks (no new CDF rows).
- Split the test file by responsibility (eob ≤ 2 vs eob 3-10) to stay under the
  1000-line budget.

## Capabilities

### New Capabilities

- None.

### Modified Capabilities

- `encoder-tools`: extend the general low-frequency coefficient walk to eob 5-10 with
  the `eob_extra_bit` bypass literals.

## Impact

- Affected code: `crates/splot-encode/src/coefficient_tokenization/general_walk.rs`
  (+ the split tests `general_walk_tests.rs` / `general_walk_eob_extra_tests.rs`).
- Scope (explicitly NOT claimed): high-frequency coefficients (scan ≥ 10), magnitudes
  beyond 7 (golomb), chroma coefficient coding, non-4x4 sizes, non-DCT_DCT, packets,
  decoder context/bit-order conformance (the §8.2 roundtrip proves self-consistency
  only; both are confirmed at the deferred splot-decode cross-check).
- Affected docs/tracking: `docs/IMPLEMENTATION-MATRIX.toml`, generated feature status
  / spec coverage.
