## Why

Sub-brick 5d-ii completes the 4x4 luma coefficient walk. 5d-i reached the first
high-frequency coefficient (eob 11, whose single HF coefficient is the EOB). This lifts
the walk to eob 16 — the full 4x4 scan — adding the missing piece: the **non-EOB**
high-frequency `coeff_base` family. The encoder can now tokenize any 4x4 DCT_DCT luma
block from eob 1 to 16, every coefficient position, up to the golomb magnitude tier.

It reuses all the HF infrastructure 5d-i established (the HF `coeff_br`, the
`is_lf_position` per-coefficient selection, `NUM_BASE_LEVELS`, the dual-router pattern)
and adds the `DEFAULT_COEFF_BASE_CDF` family with its distinct context (no low-frequency
near-DC carve-out, no DC special case). A 4-agent adversarial review verified all the
divergences (the build agent's report was lost to a transient API 529).

## What Changes

- Add `ENC-COEFF-GENERAL-WALK-HF-MULTI` as a private `splot-encode` encoder-tool feature.
- Add `coeff_base_hf_luma_context` (HF `coeff_base` context: `magLimit=3` for every
  neighbour, no near-DC carve-out, no `c==0` case, `ctx2 + {0,5,10}` bands).
- Add the `CoeffBase` selector + `coeff_base_hf_token` (level cap `NUM_BASE_LEVELS+1=3`,
  4-symbol `DEFAULT_COEFF_BASE_CDF`) + the HF `coeff_base` bank (full 20 contexts) to
  BOTH §8.2 proof routers.
- `compose_base_pass` selects each non-EOB coefficient's LF vs HF `coeff_base` by
  `is_lf_position`; lift `MAX_GENERAL_SCAN_INDEX` 10 → 15.

## Capabilities

### New Capabilities

- None.

### Modified Capabilities

- `encoder-tools`: complete the general coefficient walk to eob 16 with the non-EOB
  high-frequency `coeff_base` family.

## Impact

- Affected code: `crates/splot-encode/src/coefficient_tokenization{.rs,/general_walk.rs,
  /multi_coeff.rs,/coeff_base_lf.rs,/cdf_rows.rs}`, `block_symbol_trace/cdf_rows.rs`
  (+ a new `general_walk_hf_multi_tests.rs`).
- Scope (explicitly NOT claimed): golomb magnitudes (5e), chroma, non-4x4 sizes,
  non-DCT_DCT, packets, decoder context/table conformance (§8.2 self-consistency only).
- `general_walk.rs` is at the 1000-line limit (compliant); the §8.2 recovery half splits
  into a sibling module in 5e before golomb code is added.
- Affected docs/tracking: `docs/IMPLEMENTATION-MATRIX.toml`, generated feature status /
  spec coverage.
