## Why

The ordinary coefficient branch still accepts caller-resolved `txSet`, even
though AV2 §5.20.8.3 derives it from `txSz`, plane, inter/intra state, and the
reduced-transform-set frame flags. Deriving that value is the next small step
toward wiring runtime `coeffs()` without broadening beyond the already landed
non-lossless intra chroma `Mode_To_Txfm` subset.

## What Changes

- Add Feature ID `DECODE-COEFF-ORDINARY-BRANCH-TX-SET-HANDOFF`.
- Add a crate-private ordinary-branch wrapper that derives AV2 §5.20.8.3
  `txSet` from generated transform-size conversion tables and caller-resolved
  `is_inter`, `reduced_tx_set`, and `enable_chroma_dctonly` facts.
- Delegate successful nonzero branches to the existing
  `Mode_To_Txfm` ordinary-branch wrapper so `PlaneTxType` derivation remains
  centralized.
- Preserve the all-zero branch behavior and reject invalid reduced-transform
  set values or transform-size table domains before mutation.
- Add focused equivalence and fail-atomic tests, matrix/support/conformance
  metadata, generated status docs, and roadmap notes.
- No runtime decode output changes.

## Capabilities

### New Capabilities

- `coeff-ordinary-branch-tx-set-handoff`: ordinary-branch handoff from AV2
  §5.20.8.3 `get_tx_set` to the existing `Mode_To_Txfm` subset wrapper.

### Modified Capabilities

- `decoder-support`: records the new handoff as partial decoder infrastructure
  while keeping full `compute_tx_type`, luma/inter/lossless branches, frame-state
  parsing, runtime `coeffs()`, reconstruction, and output unsupported.

## Impact

- Affected code: `crates/splot-decode/src/tile_payload/coeff_loop/ordinary_pass/geometry.rs`
  and focused coefficient-loop tests.
- Affected docs/status: `docs/IMPLEMENTATION-MATRIX.toml`,
  `docs/DECODER-SUPPORT-MATRIX.toml`, `docs/DECODER-ROADMAP.md`, generated
  feature/support/conformance status docs, and OpenSpec artifacts.
- No public API changes, no new diagnostics, no dependency graph changes, no
  encoder work, and no validator behavior changes.
