## Why

After decoded ordinary non-FSC levels are written into local `Level[]`, AV2
§5.20.7.27 performs a second reverse scan pass that reads coefficient signs
before `read_quant` produces `Quant[]` and `QuantSign[]`. The decoder now needs a
focused sign-read boundary that consumes caller-resolved sign sources without
claiming quantization or reconstruction support.

## What Changes

- Add Feature ID `DECODE-COEFF-SIGN-SYMBOL-READ` for ordinary non-FSC
  coefficient sign symbol/literal reads.
- Add a crate-private helper that accepts local `Level[]` state, checked scan
  entries, and caller-resolved per-entry sign sources.
- Support caller-selected `dc_sign` / `dc_sign_horz_vert` CDF reads through the
  existing `TileDcSignCdf` selector and generic `sign_bit` literal reads.
- Preflight read counts, scan-entry identity, local level coordinates, and
  required signs for nonzero levels before consuming any sign syntax.
- Add focused tests for mixed CDF/literal/skip reads, invalid selector
  no-consumption behavior, input-count mismatch rejection, missing required sign
  rejection, and scan mismatch rejection.
- Update the implementation matrix, decoder support matrix, decoder conformance
  coverage metadata, roadmap, and generated status/coverage docs.

Non-goals:

- No runtime `coeffs()` integration and no decode-output change.
- No derivation of sign-source policy from real transform class, plane, hidden
  parity, or DC-context state; callers provide those facts.
- No `QuantSign[]` or `Quant[]` writes, no `read_quant`, dequantization, inverse
  transform, residual add, reconstruction, reference refresh, public API,
  AVM/dav2d invocation, dependency change, or scheduler change.

## Capabilities

### New Capabilities

- `coeff-sign-symbol-read`: crate-private ordinary non-FSC coefficient
  sign-symbol/literal read boundary after local `Level[]` state exists.

### Modified Capabilities

- `decoder-support`: records `DECODE-COEFF-SIGN-SYMBOL-READ` as partial
  coefficient-decode support while `QuantSign[]`, `Quant[]`, `read_quant`, and
  reconstruction remain incomplete.

## Impact

- `crates/splot-decode/src/tile_payload/coeff_loop.rs`
- `crates/splot-decode/src/tile_payload/coeff_loop/sign_symbol.rs`
- `docs/IMPLEMENTATION-MATRIX.toml`, `docs/DECODER-SUPPORT-MATRIX.toml`
- `xtask/src/decoder_conformance_coverage.rs`
- generated status/coverage docs
- `openspec/specs/decoder-support/spec.md`
