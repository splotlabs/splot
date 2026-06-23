## Why

The FSC/IDTX coefficient path now has loaded CDF rows and a checked forward
scan window, but nothing consumes `coeff_base_bob`, `coeff_base_idtx`, or
`coeff_br_idtx` in the AV2 section 5.20.7.27 `useFsc` first pass. The next
small decoder step is a loaded-but-unwired helper that reads those symbols in
spec order and writes local `Level[]` state before later IDTX sign/quant work.

Feature ID: `DECODE-COEFF-FSC-LEVEL-PASS`.

## What Changes

- Add a crate-private `NonZeroCoeffFscLevelPass` over `FscCoeffScanWalk`.
- Derive `BaseBob`, later `BaseIdtx`, and conditional `BrIdtx` selectors from
  caller-resolved q/tx-size facts plus current local `Level[]`.
- Read the corresponding symbol rows in forward `bob..segEob` order and write
  `Level[row][col]` after each coefficient.
- Update decoder tracking docs, generated status, and OpenSpec artifacts.

## Capabilities

### New Capabilities

- `coeff-fsc-level-pass`: loaded-but-unwired FSC/IDTX first level pass.

### Modified Capabilities

- `decoder-support`: record that FSC/IDTX level symbol reads and local
  `Level[]` writes exist, while IDTX signs, `read_quant`, runtime `coeffs()`,
  and reconstruction remain unsupported.

## Impact

Affected code is limited to `splot-decode` coefficient-loop internals and
tests. There are no public API, CLI, dependency, licensing, encoder, runtime
output, or oracle-invocation changes.
