## Why

The ordinary coefficient branch `txSz` wrapper now derives raw dimensions and
adjusted base-context dimensions, but it still accepts `txSzCtx` from the caller.
AV2 section 5.20.7.27 derives that context directly from the generated
`Tx_Size_Sqr[txSz]` and `Tx_Size_Sqr_Up[txSz]` conversion tables. Those tables
are now available after `DECODE-TX-SIZE-SYMBOLIC-TABLES`, so the wrapper can
remove another stale caller-resolved fact without changing runtime decode output.

## What Changes

- Add Feature ID `DECODE-COEFF-ORDINARY-BRANCH-TX-SIZE-CONTEXT`.
- Extend the crate-private ordinary branch `txSz` wrapper to derive
  `txSzCtx = (Tx_Size_Sqr[txSz] + Tx_Size_Sqr_Up[txSz] + 1) >> 1`.
- Feed the derived `txSzCtx` into the ordinary base-context pass while preserving
  the existing raw and adjusted dimension split.
- Add focused tests proving rectangular transform-size context derivation,
  all-zero preservation, and fail-atomic invalid table handling.
- Update decoder tracking, roadmap, generated status/coverage docs, and this
  OpenSpec change.

## Capabilities

### New Capabilities

- `coeff-ordinary-branch-tx-size-context`: The loaded-but-unwired ordinary
  coefficient branch derives `txSzCtx` from generated AV2 section 9.2 tables for
  section 5.20.7.27 base-row selection.

### Modified Capabilities

- `decoder-support`: Record the `txSzCtx` ordinary branch row and proof while
  keeping runtime coefficient-loop integration partial.

## Impact

- Affected code: `crates/splot-decode/src/tile_payload/coeff_loop/ordinary_pass/geometry.rs`
  and focused ordinary branch tests.
- Affected docs/tracking: implementation matrix, decoder support matrix,
  decoder conformance coverage, decoder roadmap, generated status docs, and this
  OpenSpec change.
- No public API, dependency graph, CLI behavior, broad `decode_block()` /
  `decode_tile()` behavior, runtime `coeffs()` call site, `compute_tx_type`,
  scan derivation, dequantization, reconstruction, output, or reference refresh
  support is added.
