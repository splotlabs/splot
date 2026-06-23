## Why

The ordinary non-FSC coefficient path already has a reverse scan-walk boundary,
but AV2 §5.20.7.27 uses a different scan window for `useFsc`: it computes
`bob = segEob - eob`, expands `eob` to `segEob`, and then visits
`c = bob..segEob` in forward order. The decoder needs that checked window before
the loaded IDTX CDF rows can be consumed by a staged FSC/IDTX symbol pass.

Feature ID: `DECODE-COEFF-FSC-SCAN-WALK`.

## What Changes

- Add a crate-private `FscCoeffScanWalk` boundary over caller-resolved
  `segEob` and `scan[c]`.
- Validate decoded EOB, `segEob`, scan length, and raster positions before any
  future symbol reads or coefficient writes.
- Return checked forward scan entries plus the derived `bob` and `segEob`
  facts for the future `coeff_base_bob` / `coeff_base_idtx` pass.
- Update decoder tracking docs, generated status, and OpenSpec artifacts.

## Capabilities

### New Capabilities

- `coeff-fsc-scan-walk`: checked forward FSC/IDTX coefficient scan-window
  derivation.

### Modified Capabilities

- `decoder-support`: record the new partial decoder boundary and clarify that
  runtime `useFsc` symbol reads remain unsupported.

## Impact

Affected code is limited to `splot-decode` coefficient scan traversal and tests.
There are no public API, dependency, licensing, encoder, or CLI changes. Decode
output for the current minimal fixture remains unchanged because runtime
`coeffs()` still does not call a FSC/IDTX coefficient symbol path.
