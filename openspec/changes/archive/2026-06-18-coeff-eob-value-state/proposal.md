## Why

The coefficient-loop state now models the all-zero branch, but the next
nonzero path still has no checked representation of the § 5.20.7.27 EOB value
derived from `eobPt`, `eob_extra`, and the following refinement bits. Adding
that narrow value/state helper is the next safe step before wiring real
`eob_pt_*` symbol reads into the minimal decoder trace.

Feature ID: `DECODE-COEFF-EOB-VALUE-STATE`.

## What Changes

- Add crate-private `splot-decode` coefficient-loop helpers that compute a
  nonzero `eob` value from caller-decoded `eobPt`, `eob_extra`, and packed
  `eob_extra_bit` refinements according to AV2 § 5.20.7.27.
- Keep the helper total and panic-free with typed errors for impossible
  caller-provided EOB parts.
- Track the new partial decoder-support and implementation-matrix rows, with
  generated status and coverage docs refreshed.
- Do not read new coefficient symbols, walk the scan order, fill nonzero
  `Quant[]`, run dequantization, or change decoded output in this change.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `decoder-support`: add the partial `coeff-eob-value-state` row for the
  checked § 5.20.7.27 nonzero-EOB value helper.

## Impact

- Affected code: `crates/splot-decode/src/tile_payload/coeff_loop.rs`.
- Affected tracking/docs: `docs/IMPLEMENTATION-MATRIX.toml`,
  `docs/DECODER-SUPPORT-MATRIX.toml`, generated decoder support/status/coverage
  docs, and decoder-conformance coverage grouping.
- APIs/dependencies: no public API changes and no new dependencies.
- Diagnostics: no new user-facing diagnostics; misuse remains crate-private and
  maps through existing unsupported/minimal trace boundaries when eventually
  consumed.
