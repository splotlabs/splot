## Why

Sub-brick 5c-i. The general LF tokenizer capped at eob ≤ 2. This lifts it to eob 3
and 4 — the first eob > 2 case — adding the §5.20.7.27 `eob_extra` CDF flag and
walking 3-4 coefficients. eob 5..=10 (which add `eob_extra_bit` bypass literals) is a
later sub-brick.

A 600-block exhaustive fuzz over the new eob 3-4 window revealed the full reachable
4x4 low-frequency CDF context set — far more than any hand-picked test exercises (two
recent PRs shipped single-context routing holes). So the three 4x4-LF CDF tables are
refactored into context-indexed banks sized to the full generated context dimension,
making the entire 4x4-LF tier free of single-context routing holes.

## What Changes

- Add `ENC-COEFF-GENERAL-WALK-EOB-EXTRA` as a private `splot-encode` encoder-tool
  feature.
- Lift `tokenize_general_lf_luma_block` to eob 3 and 4: `eob_pt_16` symbol = `eobPt-1`
  (fixed from `eob-1`, which coincided for eob ≤ 2), then the `eob_extra` CDF flag =
  `eob-3` (no `eob_extra_bit` literals at eobPt 3). Walk 3-4 coefficients via the
  existing context helpers; reject a nonzero at scan index ≥ 4. Extend
  `recover_quant_from_tokens` to read the `eob_extra` flag (eob = 3 + flag).
- Add an `eob_extra_token` constructor over the existing `EobExtra` selector +
  `DEFAULT_EOB_EXTRA_CDF` row.
- Refactor the 4x4-LF `coeff_base`, `coeff_base_eob`, and `coeff_br` CDF rows into
  context-indexed banks (sized to the generated dimension, TCQ-neutral, q-ctx 0,
  bounds-checked routing) so the whole 4x4-LF tier is routing-hole-free.

## Capabilities

### New Capabilities

- None.

### Modified Capabilities

- `encoder-tools`: extend the general low-frequency coefficient walk to eob 3 and 4
  with the `eob_extra` flag, and make the 4x4-LF CDF routing hole-free.

## Impact

- Affected code: `crates/splot-encode/src/coefficient_tokenization/{general_walk.rs,
  multi_coeff.rs}` (+ tests), `coefficient_tokenization.rs`,
  `block_symbol_trace/{cdf_rows,mod}.rs` (the bank refactor).
- Scope (explicitly NOT claimed): eob > 4 / `eob_extra_bit` literals, magnitudes
  beyond 7 (golomb), high-frequency or chroma coefficients, sizes other than 4x4,
  types other than DCT_DCT, packets, decoder context conformance (the §8.2 roundtrip
  proves self-consistency only; the bank correctness vs a real decoder is the deferred
  cross-check sub-brick).
- Affected docs/tracking: `docs/IMPLEMENTATION-MATRIX.toml`, generated feature status
  / spec coverage.
