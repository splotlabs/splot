## Why

The ordinary coefficient branch still accepts caller-resolved `PlaneTxType`,
even though the non-lossless intra chroma subset of AV2 §5.20.7.29 can now be
derived from the generated §9.2 `Mode_To_Txfm` table. Removing that staged
caller fact is the next small step toward a runtime `coeffs()` integration.

## What Changes

- Add Feature ID `DECODE-COEFF-ORDINARY-BRANCH-MODE-TO-TXFM-HANDOFF`.
- Add a crate-private ordinary-branch wrapper for the non-lossless intra chroma
  `Mode_To_Txfm` subset of `compute_tx_type`.
- Resolve `PlaneTxType` from caller-resolved `enable_chroma_dctonly`, `UVMode`,
  caller-resolved `txSet`, and the inline AV2 §5.20.7.29
  `Tx_Type_In_Set_Intra` membership table, falling back to `DCT_DCT` when the
  short-circuit or set rejects the mapped transform.
- Preserve the existing caller-resolved `PlaneTxType` wrapper for luma, inter,
  lossless, directional wide-angle, and future full-runtime paths.
- Add focused equivalence and fail-atomic tests, matrix/support/conformance
  metadata, generated status docs, and roadmap notes.
- No runtime decode output changes.

## Capabilities

### New Capabilities

- `coeff-ordinary-branch-mode-to-txfm-handoff`: non-lossless intra chroma
  ordinary-branch handoff from generated `Mode_To_Txfm` to `PlaneTxType`.

### Modified Capabilities

- `decoder-support`: records the new handoff as partial decoder infrastructure
  while keeping full `compute_tx_type`, `get_tx_set`, directional wide-angle,
  runtime `coeffs()`, reconstruction, and output unsupported.

## Impact

- Affected code: `crates/splot-decode/src/tile_payload/coeff_loop/ordinary_pass/geometry.rs`
  and focused coefficient-loop tests.
- Affected docs/status: `docs/IMPLEMENTATION-MATRIX.toml`,
  `docs/DECODER-SUPPORT-MATRIX.toml`, `docs/DECODER-ROADMAP.md`, generated
  feature/support/conformance status docs, and OpenSpec artifacts.
- No public API changes, no new diagnostics, no dependency graph changes, no
  encoder work, and no validator behavior changes.
