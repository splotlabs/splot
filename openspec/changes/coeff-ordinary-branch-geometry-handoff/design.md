## Context

The ordinary coefficient branch now has staged wrappers for caller-resolved
`PlaneTxType -> txClass` and `plane -> plane_type`. The nonzero branch still
passes transform-block 4x4 geometry through both `NonZeroCoeffBlockStartInput.block`
and `CoeffOrdinaryPlaneTypeStateContextConfig`, so callers can still provide
contradictory state-context coordinates.

## Goals / Non-Goals

**Goals:**
- Add a crate-private ordinary branch wrapper that derives state-context `x4`,
  `y4`, `w4`, and `h4` from `NonZeroCoeffBlockStartInput.block`.
- Keep all-zero behavior unchanged and preserve the current explicit geometry
  configs for staged lower-level tests.
- Record the row in implementation and decoder support tracking with focused
  proof.

**Non-Goals:**
- Do not derive raw AV2 `x4 = startX >> 2`, `y4 = startY >> 2`,
  `w4 = Tx_Width[txSz] >> 2`, or `h4 = Tx_Height[txSz] >> 2` from `coeffs()`
  arguments in this change.
- Do not implement AV2 section 5.20.7.29 `compute_tx_type`.
- Do not derive scan order, adjusted transform geometry, coefficient CDF q
  context, parity/TCQ facts, or lossless state.
- Do not wire runtime `coeffs()`, selector derivation from real syntax,
  dequantization, inverse transform, residual add, output, or reference refresh.
- Do not change public APIs, CLI behavior, dependencies, or decode output.

## Decisions

- Keep the wrapper in crate-visible `coeff_loop/ordinary_pass/geometry.rs`,
  because it adapts the same branch boundary while keeping the parent
  ordinary-pass module below the source-line soft limit.
- Add a reduced state-context config that contains only `coeff_cdf_q_ctx`.
  Geometry comes from `input.start.block`; plane type still comes from
  `input.base_config.plane` through the prior wrapper.
- Add a wrapper input enum rather than replacing existing configs. Existing
  staged tests can keep using explicit geometry, while future runtime wiring can
  supply the single block-start geometry fact at the branch boundary.

## Risks / Trade-offs

- [Risk] The wrapper can be mistaken for raw AV2 transform-block geometry
  derivation from `startX`, `startY`, and `txSz`.
  -> Mitigation: docs, specs, and matrix notes explicitly limit this to the
  already staged block-start geometry.
- [Risk] Another crate-private staged API increases temporary surface area.
  -> Mitigation: keep it as a thin adapter that immediately delegates to the
  existing `plane_type` handoff and prove equivalence with the explicit config.
