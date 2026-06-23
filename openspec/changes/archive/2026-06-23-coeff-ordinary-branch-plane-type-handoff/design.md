## Context

The ordinary coefficient stack now has a state-backed all-zero/nonzero branch
handoff plus a `PlaneTxType -> txClass` branch wrapper. The top-level nonzero
ordinary branch still requires callers to provide `CoeffOrdinaryStateContextConfig.plane_type`
directly even though AV2 section 5.20.7.27 derives `ptype` inside `coeffs()` as
`plane > 0`.

## Goals / Non-Goals

**Goals:**
- Add a crate-private ordinary branch wrapper that accepts caller-resolved
  `plane` and derives `plane_type` before delegating to the existing
  `PlaneTxType` ordinary branch handoff.
- Keep all-zero behavior unchanged and preserve the current explicit
  `CoeffOrdinaryBranchInput` and `CoeffOrdinaryBranchPlaneTxTypeInput` APIs for
  staged tests.
- Record the row in implementation and decoder support tracking with focused
  proof.

**Non-Goals:**
- Do not implement AV2 section 5.20.7.29 `compute_tx_type`.
- Do not derive scan order, transform block geometry, coefficient CDF q context,
  parity/TCQ facts, or lossless state.
- Do not wire runtime `coeffs()`, selector derivation from real syntax,
  dequantization, inverse transform, residual add, output, or reference refresh.
- Do not change public APIs, CLI behavior, dependencies, or decode output.

## Decisions

- Keep the wrapper in `coeff_loop/ordinary_pass.rs` next to
  `apply_coeff_ordinary_branch_from_plane_tx_type`, because it adapts that exact
  staged boundary without changing lower-level pass contracts.
- Add a reduced state-context config that omits `plane_type` and derives
  `usize::from(plane > 0)` from the same `plane` already carried by the
  `PlaneTxType` base config. This models AV2 section 5.20.7.27 directly and
  avoids accepting contradictory `plane` and `plane_type` values at the wrapper.
- Add a wrapper input enum rather than replacing existing configs. Existing
  staged tests can keep using explicit `plane_type`, while future runtime wiring
  can supply `plane` at the branch boundary.

## Risks / Trade-offs

- [Risk] The wrapper can look like full `coeffs()` integration.
  -> Mitigation: docs, matrix rows, and tests explicitly exclude runtime
  `coeffs()` wiring, scan derivation, and transform-type computation.
- [Risk] Another crate-private staged API increases temporary surface area.
  -> Mitigation: keep it as a thin adapter that immediately delegates to the
  existing branch path and prove equivalence with the explicit config.
