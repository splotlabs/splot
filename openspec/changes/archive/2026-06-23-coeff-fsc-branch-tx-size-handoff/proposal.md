## Why

The staged FSC/IDTX coefficient branch now derives scan order from `txSz` and
`PlaneTxType`, but it still accepts caller-built EOB context, FSC level config,
and context geometry. Those facts are all derived from the same AV2
`coeffs(plane, startX, startY, txSz)` setup. A tx-size handoff removes another
synthetic boundary before runtime `coeffs()` can call the loaded FSC path.

## What Changes

- Add Feature ID `DECODE-COEFF-FSC-BRANCH-TX-SIZE-HANDOFF`.
- Add a crate-private loaded-but-unwired FSC/IDTX branch wrapper that accepts
  caller-resolved block geometry, `txSz`, `PlaneTxType`, `is_inter`, and
  `coeff_cdf_q_ctx`.
- Derive nonzero EOB context, raw/adjusted transform dimensions, `txSzCtx`,
  scan order, FSC level config, and context-commit geometry from generated AV2
  conversion tables before delegating to the existing scan-order FSC branch.
- Add positive equivalence and fail-atomic tests for derived tx-size facts,
  generated table validation, luma-only routing, and block-geometry consistency.
- Update implementation/support/conformance tracking, roadmap notes, and
  generated status docs.

## Capabilities

### New Capabilities

- `coeff-fsc-branch-tx-size-handoff`: Crate-private loaded-but-unwired FSC/IDTX
  coefficient branch handoff that derives tx-size-dependent FSC branch facts
  before the existing scan-order wrapper.

### Modified Capabilities

- `decoder-support`: Track the new partial decoder-support row and conformance
  coverage entry for the FSC tx-size handoff.

## Impact

- Affected code: `crates/splot-decode/src/tile_payload/coeff_loop/fsc_quant_pass.rs`,
  focused coefficient-loop tests, `xtask` decoder conformance coverage, decoder
  roadmap, and tracking docs.
- APIs: crate-private only; no public API, CLI, output, diagnostic, or
  dependency changes.
- Non-goals: runtime `useFsc` derivation, full `compute_tx_type`, `PlaneTxType`
  derivation, runtime `coeffs()` wiring, dequantization, inverse transform,
  residual add, reconstruction, reference refresh, AVM/dav2d invocation, and
  broad decoder conformance claims.
