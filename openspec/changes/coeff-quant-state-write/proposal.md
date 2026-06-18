## Why

After ordinary non-FSC levels and signs are locally available, AV2
§5.20.7.27 applies `read_quant` results to `Quant[]`, updates `culLevel` and
`dcCategory`, and applies sign/TCQ adjustments. The decoder needs a small
state-application boundary for those writes before implementing the
`read_quant` syntax reader or wiring runtime `coeffs()`.

## What Changes

- Add Feature ID `DECODE-COEFF-QUANT-STATE-WRITE` for the post-sign ordinary
  non-FSC quantized-coefficient state application step.
- Add a crate-private helper that consumes checked scan entries, local
  `Level[]` state, sign summaries, and caller-provided `read_quant` outputs.
- Apply the spec's hidden-parity, `culLevel`, `dcCategory`, optional TCQ, sign,
  and `Quant[pos]` state effects while leaving `QuantSign[]` untouched.
- Preflight read/sign/quant counts and scan-entry identity before mutating the
  local transform block.
- Add focused tests for positive state writes, hidden-parity and TCQ adjustment,
  zero-level sign behavior, and mismatch rejection before mutation.
- Update the implementation matrix, decoder support matrix, decoder conformance
  coverage metadata, roadmap, and generated status/coverage docs.

Non-goals:

- No runtime `coeffs()` integration and no decode-output change.
- No implementation of §5.20.7.28 `read_quant` bit parsing; callers provide the
  already-decoded quant and updated `hrLevelAvg` facts.
- No `QuantSign[]` writes; the IDTX sign-state path is separate.
- No derivation of scan tables, transform class, LF limits, parity hiding,
  TCQ enablement, or `Lossless` from real block syntax; callers provide those
  facts.
- No dequantization, inverse transform, residual add, reconstruction, reference
  refresh, public API, AVM/dav2d invocation, dependency change, or scheduler
  change.

## Capabilities

### New Capabilities

- `coeff-quant-state-write`: crate-private ordinary non-FSC coefficient
  quantized-state write boundary after local levels, signs, and caller-provided
  `read_quant` results exist.

### Modified Capabilities

- `decoder-support`: records `DECODE-COEFF-QUANT-STATE-WRITE` as partial
  coefficient-decode support while §5.20.7.28 `read_quant`, runtime `coeffs()`,
  and reconstruction remain incomplete.

## Impact

- `crates/splot-decode/src/tile_payload/coeff_loop.rs`
- `crates/splot-decode/src/tile_payload/coeff_loop/quant_state.rs`
- `docs/IMPLEMENTATION-MATRIX.toml`, `docs/DECODER-SUPPORT-MATRIX.toml`
- `xtask/src/decoder_conformance_coverage.rs`
- generated status/coverage docs
- `openspec/specs/decoder-support/spec.md`
