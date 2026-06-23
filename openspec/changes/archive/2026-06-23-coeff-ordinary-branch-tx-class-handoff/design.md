## Context

The ordinary coefficient stack now has state-backed all-zero/nonzero branch
handoff, derived first-pass base selectors, derived sign sources, derived
`maxLevel`, and signed `Quant[]` writes. The top-level ordinary branch still
requires callers to provide `CoeffBaseDerivedLevelPassConfig.tx_class` directly.
The previous `DECODE-COEFF-TX-CLASS-DERIVE` change added a decode-local,
crate-private `PlaneTxType -> CoeffTransformClass` helper that matches AV2
section 8.3.2, but only the max-level layer has a `PlaneTxType` handoff.

## Goals / Non-Goals

**Goals:**
- Add a crate-private ordinary branch wrapper that accepts caller-resolved
  `PlaneTxType` and derives `CoeffTransformClass` before delegating to the
  existing state-backed ordinary branch.
- Keep all-zero behavior unchanged and preserve the current explicit
  `CoeffOrdinaryBranchInput` API for staged tests.
- Record the row in implementation and decoder support tracking with focused
  proof.

**Non-Goals:**
- Do not implement AV2 section 5.20.7.29 `compute_tx_type`.
- Do not derive scan order or import `splot-recon` into coefficient entropy
  code.
- Do not wire runtime `coeffs()`, selector derivation from real syntax,
  dequantization, inverse transform, residual add, output, or reference refresh.
- Do not change public APIs, CLI behavior, dependencies, or decode output.

## Decisions

- Keep the wrapper in `coeff_loop/ordinary_pass.rs` next to
  `apply_coeff_ordinary_branch`, because it adapts that exact boundary without
  changing lower-level pass contracts.
- Reuse `CoeffTransformClass::from_plane_tx_type` rather than duplicating the
  match in the ordinary branch module. This keeps the § 8.3.2 mapping in one
  decode-local helper and avoids a `splot-recon` dependency in entropy code.
- Add a wrapper input enum rather than replacing existing configs. Existing
  staged tests and lower-level helpers can keep using explicit
  `CoeffTransformClass`, while future runtime wiring can supply `PlaneTxType`
  at the branch boundary.

## Risks / Trade-offs

- [Risk] The wrapper can look like broad transform-type support.
  -> Mitigation: docs, matrix rows, and tests explicitly exclude
  `compute_tx_type`, scan derivation, and runtime `coeffs()` wiring.
- [Risk] Adding another staged API increases temporary surface area.
  -> Mitigation: keep it crate-private, delegate immediately to the existing
  branch path, and test equivalence with the direct config.
