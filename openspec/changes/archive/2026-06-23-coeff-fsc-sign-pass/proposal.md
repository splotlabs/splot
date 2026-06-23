## Why

The FSC/IDTX coefficient path now reads level symbols and writes local
`Level[]`, but it still cannot execute the following AV2 section 5.20.7.27
`idtx_sign` pass. The next small decoder step is a loaded-but-unwired helper
that reads `idtx_sign` in forward scan order, updates local `QuantSign[]` for
evolving sign contexts, and keeps `read_quant` and `Quant[]` production deferred.

Feature ID: `DECODE-COEFF-FSC-SIGN-PASS`.

## What Changes

- Add a crate-private FSC/IDTX sign pass after `NonZeroCoeffFscLevelPass`.
- Derive `IdtxSign` selectors from the current `QuantSign[]` and `Level[]`
  state using the existing AV2 section 8.3.2 context helper.
- Read `idtx_sign` only for nonzero `Level[]` entries over `c = 0..segEob` and
  update local `QuantSign[]` with `-1` or `1` for later sign contexts.
- Update decoder tracking docs, generated status, and OpenSpec artifacts.

## Capabilities

### New Capabilities

- `coeff-fsc-sign-pass`: loaded-but-unwired FSC/IDTX sign pass.

### Modified Capabilities

- `decoder-support`: record that FSC/IDTX sign symbol reads and local
  `QuantSign[]` writes exist, while `read_quant`, nonzero `Quant[]`, runtime
  `coeffs()`, and reconstruction remain unsupported.

## Impact

Affected code is limited to `splot-decode` coefficient-loop internals and
tests. There are no public API, CLI, dependency, licensing, encoder, runtime
output, or oracle-invocation changes.
