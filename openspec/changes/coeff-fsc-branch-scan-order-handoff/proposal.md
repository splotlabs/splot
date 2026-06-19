## Why

The staged FSC/IDTX coefficient branch now derives `segEob` from a caller-supplied scan extent, but the scan table itself is still a caller fact. Deriving `scan = get_scan(txSz, txClass)` from the transform size and transform class removes another synthetic boundary before the real `coeffs()` loop can call the loaded FSC path.

## What Changes

- Add Feature ID `DECODE-COEFF-FSC-BRANCH-SCAN-ORDER`.
- Add a crate-private loaded-but-unwired FSC/IDTX branch wrapper that derives the AV2 § 5.20.7.30 scan order from caller-resolved `txSz` and `PlaneTxType`, then delegates to the existing scan-extent FSC branch.
- Share the decode-local scan-order derivation between ordinary and FSC coefficient branches so both paths use the same § 5.20.7.30 implementation.
- Add positive equivalence and fail-atomic tests for scan derivation and generated transform-size table validation.
- Update implementation/support/conformance tracking, roadmap notes, and generated status docs.

## Capabilities

### New Capabilities
- `coeff-fsc-branch-scan-order-handoff`: Crate-private loaded-but-unwired FSC/IDTX coefficient branch handoff that derives `scan = get_scan(txSz, txClass)` before the existing scan-extent wrapper.

### Modified Capabilities
- `decoder-support`: Track the new partial decoder-support row and conformance coverage entry for the FSC scan-order handoff.

## Impact

- Affected code: `crates/splot-decode/src/tile_payload/coeff_loop/fsc_quant_pass.rs`, scan-order helper code shared with `ordinary_pass/geometry.rs`, focused coefficient-loop tests, `xtask` decoder conformance coverage, and decoder roadmap/tracking docs.
- APIs: crate-private only; no public API, CLI, output, diagnostic, or dependency changes.
- Non-goals: runtime `useFsc` derivation, runtime `coeffs()` wiring, full `compute_tx_type`, level-config derivation, context geometry derivation, dequantization, inverse transform, residual add, reconstruction, reference refresh, AVM/dav2d invocation, and broad decoder conformance claims.
