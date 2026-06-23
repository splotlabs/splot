## Why

The ordinary coefficient branch now receives generated `txSz` dimensions, but
its base-context handoff still uses the raw transform size where AV2 section
8.3.2 requires `Adjusted_Tx_Size[txSz]`. The generated adjusted-size table is
available after `DECODE-TX-SIZE-SYMBOLIC-TABLES`, so the next wrapper can remove
that caller-resolved/stale split without hand-writing table values.

## What Changes

- Add Feature ID `DECODE-COEFF-ORDINARY-BRANCH-ADJUSTED-TX-SIZE`.
- Extend the crate-private ordinary branch `txSz`-dimensions wrapper to derive
  `Adjusted_Tx_Size[txSz]` from generated AV2 section 9.2 conversion tables.
- Feed adjusted `Tx_Width_Log2[Adjusted_Tx_Size[txSz]]`,
  `Tx_Width[Adjusted_Tx_Size[txSz]]`, and
  `Tx_Height[Adjusted_Tx_Size[txSz]]` into the ordinary base-context pass while
  keeping raw `Tx_Width[txSz]` / `Tx_Height[txSz]` for block geometry and raw
  log2 dimensions for EOB-size context.
- Add focused tests proving adjusted 64-sample-side behavior, all-zero
  preservation, and fail-atomic invalid adjusted-size table handling.
- Update decoder tracking, roadmap, generated status/coverage docs, and this
  OpenSpec change.

## Capabilities

### New Capabilities
- `coeff-ordinary-branch-adjusted-tx-size`: The loaded-but-unwired ordinary
  coefficient branch derives adjusted transform-size dimensions for section 8.3.2
  base contexts from generated section 9.2 tables.

### Modified Capabilities
- `decoder-support`: Record the adjusted transform-size ordinary branch row and
  proof while keeping runtime coefficient-loop integration partial.

## Impact

- Affected code: `crates/splot-decode/src/tile_payload/coeff_loop/ordinary_pass/geometry.rs`
  and focused ordinary branch tests.
- Affected docs/tracking: implementation matrix, decoder support matrix,
  decoder conformance coverage, decoder roadmap, generated status docs, and this
  OpenSpec change.
- No public API, dependency graph, CLI behavior, broad `decode_block()` /
  `decode_tile()` behavior, runtime `coeffs()` call site, `txSzCtx` derivation,
  `compute_tx_type`, scan derivation, dequantization, reconstruction, output, or
  reference refresh support is added.
