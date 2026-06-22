## Why

Sub-brick 5e-ii completes the §5.20.7.28 `read_quant` golomb tier. 5e supported ONE
golomb coefficient per block (the `m=1` case, since a single golomb coefficient has
`hrLevelAvg=0`). This lifts that restriction to an arbitrary number of golomb
coefficients per 4x4 luma block by threading the running `hrLevelAvg` predictor, which
makes the golomb parameter `m` vary per coefficient. With this, the 4x4 DCT_DCT luma
coefficient tokenizer is complete: eob 1-16, every position, every magnitude.

## What Changes

- Add `ENC-COEFF-GENERAL-WALK-GOLOMB-MULTI` as a private `splot-encode` encoder-tool
  feature.
- Generalize the golomb tail helper to a `GolombParams` (m/k/cMax/bias) derived from
  `hrLevelAvg`; the finite-q `coeff_rem` becomes an `L(m)` literal (was `L(1)`).
- Thread `hrLevelAvg` (init 0, `next = (x + hrLevelAvg) >> 1`) identically across
  `compose_sign_pass` (reserve + emit), `validate_general_lf_scope` (per-`m` cap), and
  `recover_quant_from_tokens`.
- Remove the multiple-golomb-coefficient rejection (and the now-unused error variant).

## Capabilities

### New Capabilities

- None.

### Modified Capabilities

- `encoder-tools`: support an arbitrary number of `read_quant` golomb coefficients per
  block via `hrLevelAvg` threading.

## Impact

- Affected code: `crates/splot-encode/src/coefficient_tokenization/{general_walk.rs,
  general_walk_golomb.rs, general_walk_recover.rs}`, `error.rs` (+ tests).
- Scope (explicitly NOT claimed): chroma, non-4x4 sizes, non-DCT_DCT, packets, decoder
  context/table conformance (§8.2 self-consistency only; golomb bit values checked by
  exact bypass-stream assertions mirrored from the decoder).
- Affected docs/tracking: `docs/IMPLEMENTATION-MATRIX.toml`, generated feature status /
  spec coverage.
