## Why

The loaded FSC/IDTX coefficient branch still accepts `segEob` as an independent
caller fact even though AV2 derives it from the same capped transform extent that
`get_scan(txSz, txClass)` uses. Removing that duplicated fact is the next small
step toward runtime `coeffs()` wiring without changing decode output yet.

Feature ID: `DECODE-COEFF-FSC-BRANCH-SEG-EOB-HANDOFF`.

## What Changes

- Add a crate-private loaded-but-unwired FSC branch handoff that derives
  `segEob` from the caller-resolved scan window length before delegating to
  `apply_coeff_fsc_branch`.
- Preserve the existing all-zero, non-luma, and invalid-scan mutation boundaries.
- Add focused positive and failure tests proving equivalence with the explicit
  `segEob` path and fail-atomic behavior.
- Update implementation/support/conformance tracking, roadmap text, and
  generated status documents.

Non-goals:

- Do not wire runtime `coeffs()` or change decode output.
- Do not derive `useFsc`, `PlaneTxType`, `txClass`, or scan order from frame
  state.
- Do not implement dequantization, inverse transform, residual add,
  reconstruction, reference refresh, inter prediction, filters, or public APIs.

## Capabilities

### New Capabilities

- `coeff-fsc-branch-seg-eob-handoff`: covers the loaded-but-unwired FSC/IDTX
  branch wrapper that derives `segEob` from scan extent before running the
  staged FSC branch.

### Modified Capabilities

- `decoder-support`: tracks decoder support status for
  `DECODE-COEFF-FSC-BRANCH-SEG-EOB-HANDOFF`.

## Impact

- Affected code: `crates/splot-decode/src/tile_payload/coeff_loop/fsc_quant_pass.rs`
  and its focused tests.
- Affected tracking: `docs/IMPLEMENTATION-MATRIX.toml`,
  `docs/DECODER-SUPPORT-MATRIX.toml`,
  `xtask/src/decoder_conformance_coverage.rs`, roadmap/status documents, and
  OpenSpec artifacts.
- No new dependencies, public APIs, diagnostics, or crate dependency changes.
