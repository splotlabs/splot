## Context

The ordinary coefficient stack has staged helpers through the branch-level
handoff, but the nonzero path still requires callers to provide `txClass`
directly. AV2 § 5.20.7.27 computes `PlaneTxType`, immediately derives
`txClass = get_tx_class(PlaneTxType)`, and then uses that class for scan order,
low-frequency coefficient selection, sign-source selection, and TCQ eligibility.
`splot-recon` already has a pure `tx_class` companion for scan/reconstruction
work, but the decode entropy loop currently avoids importing `splot-recon`.

## Goals / Non-Goals

**Goals:**
- Add a decode-local, crate-private `PlaneTxType -> CoeffTransformClass` helper
  that matches AV2 § 8.3.2.
- Add a max-level wrapper that accepts `PlaneTxType` and delegates through the
  existing `CoeffTransformClass` path.
- Record the row in implementation and decoder support tracking with focused
  proof.

**Non-Goals:**
- Do not implement AV2 § 5.20.7.29 `compute_tx_type`.
- Do not derive scan order or import `splot-recon` into coefficient entropy
  code.
- Do not wire runtime `coeffs()`, dequantization, inverse transform, residual
  add, output, or reference refresh.
- Do not change public APIs, CLI behavior, dependencies, or decode output.

## Decisions

- Keep the mapping in `coeff_loop/max_level.rs` next to
  `CoeffTransformClass`, because the existing max-level, base-level, sign-source,
  and ordinary-pass configs already use that decode-local enum.
- Make the helper total: the AV2 § 8.3.2 function has an `else` branch that maps
  every non-horizontal/non-vertical value to `TX_CLASS_2D`, so malformed or
  future values do not need a new runtime error here.
- Add a wrapper rather than replacing existing configs. Existing staged tests can
  keep using explicit `CoeffTransformClass`, while future runtime wiring can
  supply `PlaneTxType` and get the derived class in one place.

## Risks / Trade-offs

- [Risk] Duplicating the mapping already present in `splot-recon` could drift.
  -> Mitigation: keep the mapping tiny, cite § 8.3.2, and test every horizontal
  and vertical value plus fallback cases.
- [Risk] The helper may look like broad transform-type support.
  -> Mitigation: matrix, support row, docs, and OpenSpec explicitly exclude
  `compute_tx_type`, scan derivation, runtime `coeffs()`, and reconstruction.
