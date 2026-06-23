## Why

The ordinary coefficient branch still treats AV2 §5.20.7.29 lossless
transform-type selection as unsupported even though the first lossless outcome
can be staged cleanly before the existing `txSet` and `Mode_To_Txfm` handoffs.
Adding that branch is the next small step toward runtime `coeffs()` without
claiming full `compute_tx_type` support.

## What Changes

- Add Feature ID `DECODE-COEFF-ORDINARY-BRANCH-LOSSLESS-HANDOFF`.
- Add a crate-private ordinary-branch wrapper that handles the AV2 §5.20.7.29
  nonzero `Lossless` branch which selects `DCT_DCT`.
- Delegate non-lossless nonzero branches to the existing AV2 §5.20.8.3 `txSet`
  ordinary-branch wrapper.
- Preserve all-zero behavior and make the lossless short-circuit happen before
  lower non-lossless `txSet` and `Mode_To_Txfm` validation.
- Add focused equivalence and fail-atomic tests, matrix/support/conformance
  metadata, generated status docs, and roadmap notes.
- No runtime decode output changes.

## Capabilities

### New Capabilities

- `coeff-ordinary-branch-lossless-handoff`: ordinary-branch handoff for the
  AV2 §5.20.7.29 lossless `DCT_DCT` transform-type branch before the existing
  `txSet` wrapper.

### Modified Capabilities

- `decoder-support`: records the new handoff as partial decoder infrastructure
  while keeping full `compute_tx_type`, FSC/IDTX lossless cases, inter/luma
  transform-state lookup, frame-state parsing, runtime `coeffs()`,
  reconstruction, and output unsupported.

## Impact

- Affected code: `crates/splot-decode/src/tile_payload/coeff_loop/ordinary_pass/geometry.rs`
  and focused coefficient-loop tests.
- Affected docs/status: `docs/IMPLEMENTATION-MATRIX.toml`,
  `docs/DECODER-SUPPORT-MATRIX.toml`, `docs/DECODER-ROADMAP.md`, generated
  feature/support/conformance status docs, and OpenSpec artifacts.
- No public API changes, no new diagnostics, no dependency graph changes, no
  encoder work, and no validator behavior changes.
