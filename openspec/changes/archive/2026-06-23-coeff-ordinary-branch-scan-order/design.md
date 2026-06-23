## Context

The staged ordinary coefficient branch has been removing caller-resolved facts
from the AV2 section 5.20.7.27 `coeffs()` boundary. It now derives raw
transform dimensions, adjusted base-context dimensions, and `txSzCtx` from
generated section 9.2 tables. The next remaining table/function-derived fact is
`scan`, which section 5.20.7.27 computes as `get_scan(txSz, txClass)` after
`PlaneTxType` is resolved and `txClass = get_tx_class(PlaneTxType)`.

`splot-recon` already contains a scheduler-free `coefficient_scan_order`
primitive for reconstruction, but the repository dependency policy keeps
entropy/CDF-selection code off the `splot-decode -> splot-recon` runtime handoff
edge. This change therefore adds a decode-local scan derivation at the ordinary
branch boundary instead of importing reconstruction code into coefficient
entropy traversal.

## Goals / Non-Goals

**Goals:**

- Derive AV2 section 5.20.7.30 `get_scan(txSz, txClass)` inside the ordinary
  branch `txSz` wrapper.
- Use `txClass` derived from caller-resolved `PlaneTxType` through the existing
  decode-local `CoeffTransformClass::from_plane_tx_type` helper.
- Use `Min(Tx_Width[txSz], 32)` and `Min(Tx_Height[txSz], 32)` dimensions for
  scan generation while preserving raw and adjusted dimension uses already
  implemented.
- Keep scan derivation failures before coefficient context state, CDF row, or
  symbol-decoder mutation.
- Keep the lower explicit branch APIs accepting caller-provided scan slices for
  existing staged tests.

**Non-Goals:**

- Do not implement AV2 section 5.20.7.29 `compute_tx_type`.
- Do not wire runtime `coeffs()` into tile/block syntax traversal.
- Do not dequantize, inverse transform, residual-add, reconstruct, output, or
  refresh references.
- Do not add a new dependency edge or use `splot-recon` from the entropy path.

## Decisions

- Add a decode-local `tx_size_scan` helper beside the other `txSz` table helpers.
  It consumes the already-derived raw dimensions and `CoeffTransformClass`, then
  returns a `Vec<u16>` sized to the adjusted scan extent.
- Validate the scan shape before allocation using the spec's
  `Min(Tx_Width[txSz], 32)` and `Min(Tx_Height[txSz], 32)` outputs. Generated
  `Tx_Width` / `Tx_Height` table validation already rejects invalid `txSz`
  before this helper, but a local error keeps test-table fault injection typed.
- Remove `scan` only from `CoeffOrdinaryBranchTxSizeDimensionsNonZeroInput`.
  Lower handoff structs keep caller-supplied scans so existing direct-comparison
  tests can remain explicit and future staged wrappers can still be tested in
  isolation.

## Risks / Trade-offs

- [Risk] Duplicating scan-order logic from `splot-recon` can drift.
  -> Mitigation: keep the helper narrowly spec-cited, add 2D/horizontal/vertical
  goldens that match section 5.20.7.30, and track the duplication honestly in
  the feature notes.
- [Risk] This can be mistaken for runtime coefficient-loop integration.
  -> Mitigation: tracking and roadmap notes continue to list `compute_tx_type`
  and runtime `coeffs()` wiring as deferred.
