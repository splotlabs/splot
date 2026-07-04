## Why

The nonzero coefficient path now initializes local block state and reads EOB, but
it still has no checked boundary for the § 5.20.7.27 `scan[c]` loops that drive
base, BR, sign, and `read_quant` reads. Adding a decode-local scan walk over
caller-resolved scan positions is the next small step toward real coefficient
entropy decode without importing `splot-recon` into CDF/symbol code.

## What Changes

- Add Feature ID `DECODE-COEFF-SCAN-WALK` for the decode-side coefficient scan
  traversal boundary.
- Add a crate-private `splot-decode` helper that walks the non-FSC coefficient
  loop's reverse `c = eob - 1 .. 0` order over a caller-provided scan slice.
- Validate that decoded EOB fits the supplied scan table and that every visited
  scan position fits the initialized `TransformCoeffBlockState`.
- Expose only checked scan entries to later base/BR/sign symbol readers; this
  change does not derive `get_scan`, read coefficient symbols, write nonzero
  coefficients, or change decode output.
- Update decoder roadmap, implementation matrix, support matrix, conformance
  coverage metadata, generated status docs, and focused tests.

## Capabilities

### New Capabilities
- `coeff-scan-walk`: Decode-side, caller-supplied coefficient scan traversal for
  the non-FSC § 5.20.7.27 nonzero coefficient path.

### Modified Capabilities
- `decoder-support`: Records the scan-walk boundary as a tracked decoder support
  brick while keeping runtime nonzero coefficient decode unsupported.

## Impact

- Affected code: `crates/splot-decode/src/tile_payload/coeff_loop*`,
  coefficient-loop tests, and decoder support/conformance documentation.
- APIs: crate-private only; no public API, CLI, fixture, or output change.
- Dependencies: no dependency-graph change; entropy/CDF code remains independent
  of `splot-recon`.
- Non-goals: no AV2 scan-table derivation, no transform-type computation, no
  coefficient base/BR/sign reads, no `read_quant`, no dequant/inverse transform,
  no residual add, no `local-decoder-mission.ivf` stream-planner widening, and no encoder work.
