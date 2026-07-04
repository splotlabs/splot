## Why

The current local decoder mission decoder frontier is the parsed frame-level Wiener NS bank: the
runtime correctly rejects because loop-restoration reconstruction is not applied.
Before that gate can move safely, `splot-recon` needs a small, source-backed
primitive for the AV2 section 7.20.3 non-separable Wiener sample math.

## What Changes

- Add Feature ID `RECON-WIENERNS-FILTER-PRIMITIVE` to the implementation matrix,
  decoder support matrix, and decoder conformance coverage group.
- Add a scheduler-free `splot-recon` helper that applies the AV2 section 7.20.3
  non-separable Wiener tap accumulation over caller-supplied source samples and
  caller-resolved coefficients.
- Keep frame traversal, restoration-unit syntax, pixel-classified Wiener
  classification, chroma luma-sample downsampling, temporal/reference Wiener
  state, and runtime wiring out of scope.

## Capabilities

### New Capabilities

- `recon-wienerns-filter-primitive`: AV2 section 7.20.3 non-separable Wiener
  per-block/per-sample filter primitive over caller-resolved samples and
  coefficients.

### Modified Capabilities

- `decoder-support`: Track `RECON-WIENERNS-FILTER-PRIMITIVE` as partial
  loop-restoration reconstruction progress without claiming full loop
  restoration, local decoder mission decode, or runtime wiring.

## Impact

- Affected code: `crates/splot-recon`, decoder support/implementation tracking
  docs, generated status/coverage docs, and OpenSpec artifacts.
- Public API: one additive `splot-recon` primitive and parameter type.
- Dependencies: no new dependencies and no dependency graph changes.
- Runtime behavior: no decode output change in this brick; the minimal runtime
  still rejects local decoder mission with `unsupported_wienerns_filter_bank`.
