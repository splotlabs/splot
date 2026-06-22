## Why

Sub-brick 5e adds the AV2 §5.20.7.28 `read_quant` golomb tail to the general 4x4 luma
coefficient walk — coding magnitudes beyond the base-range cap (LF ≥ 8, HF ≥ 6). It is
scoped to ONE golomb coefficient per block, where the golomb predictor `hrLevelAvg = 0`
so the parameter `m = 1` (the case the existing DC golomb composers already implement);
a block with two-or-more golomb coefficients is rejected (the multi-coefficient
`hrLevelAvg` threading is 5e-ii).

A 4-agent investigation corrected a load-bearing assumption: `read_quant` fires in the
§5.20.7.27 sign+quant pass, so the golomb bypass bits follow each coefficient's sign
token (in `compose_sign_pass`), not the base pass. A review also found the magnitude cap
must be on `x = magnitude - maxLevel ≤ 517` (LF ≤ 525, HF ≤ 523), not a
position-independent `magnitude ≤ 525`, to keep the golomb-prefix length ≤ 8.

## What Changes

- Add `ENC-COEFF-GENERAL-WALK-GOLOMB` as a private `splot-encode` encoder-tool feature.
- Split the §8.2 recovery half of `general_walk.rs` into `general_walk_recover.rs`
  (keeping the file under the 1000-line budget); add `general_walk_golomb.rs` for the
  golomb emission/recovery.
- Add the per-coefficient `read_quant` golomb tail (m=1) in `compose_sign_pass` after the
  sign token when `magnitude ≥ maxLevel`; lift `validate_general_lf_scope` to accept a
  single golomb coefficient (cap `x ≤ 517`), reject ≥ 2, and reject `x > 517`.

## Capabilities

### New Capabilities

- None.

### Modified Capabilities

- `encoder-tools`: extend the general coefficient walk with the §5.20.7.28 `read_quant`
  golomb tail for a single golomb-range coefficient.

## Impact

- Affected code: `crates/splot-encode/src/coefficient_tokenization/{general_walk.rs,
  general_walk_recover.rs (new), general_walk_golomb.rs (new)}`, `coefficient_tokenization.rs`,
  `error.rs` (+ tests).
- Scope (explicitly NOT claimed): ≥ 2 golomb coefficients (5e-ii), chroma, non-4x4 sizes,
  non-DCT_DCT, packets, decoder context/table conformance (§8.2 self-consistency only;
  the golomb bit values are checked by exact bypass-stream assertions mirrored from the
  decoder).
- Affected docs/tracking: `docs/IMPLEMENTATION-MATRIX.toml`, generated feature status /
  spec coverage.
