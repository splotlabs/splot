## Context

The ordinary coefficient branch has staged wrappers for caller-resolved
`PlaneTxType -> txClass`, `plane -> plane_type`, and block-start geometry to
state-context geometry. The latest wrapper removes duplicate state-context
geometry, but callers still construct `AllZeroCoeffBlockInput` directly even
though AV2 § 5.20.7.27 derives those fields at the start of `coeffs()`.

## Goals / Non-Goals

**Goals:**
- Add a crate-private ordinary branch wrapper that derives
  `AllZeroCoeffBlockInput` from `plane`, `startX`, `startY`, `Tx_Width[txSz]`,
  and `Tx_Height[txSz]`-style caller facts.
- Use AV2 § 5.20.7.27 shifts exactly: `x4 = startX >> 2`,
  `y4 = startY >> 2`, `w4 = tx_width >> 2`, and `h4 = tx_height >> 2`.
- Preserve the existing all-zero and nonzero branch behavior by immediately
  delegating to the existing geometry handoff.
- Record focused implementation-matrix and decoder-support proof.

**Non-Goals:**
- Do not derive `Tx_Width[txSz]`, `Tx_Height[txSz]`, `Tx_Width_Log2[txSz]`,
  `Tx_Height_Log2[txSz]`, `Tx_Size_Sqr`, or `txSzCtx` from a `txSz` enum.
- Do not implement AV2 section 5.20.7.29 `compute_tx_type`.
- Do not derive scan order, adjusted transform geometry, coefficient CDF q
  context, parity/TCQ facts, or lossless state.
- Do not wire runtime `coeffs()`, selector derivation from real syntax,
  dequantization, inverse transform, residual add, output, or reference refresh.
- Do not change public APIs, CLI behavior, dependencies, or decode output.

## Decisions

- Keep the wrapper in `coeff_loop/ordinary_pass/geometry.rs` with the prior
  geometry handoff. Both wrappers adapt the same branch boundary and can share
  the staged delegation path without growing the parent `ordinary_pass.rs`.
- Model the raw geometry facts with a small `CoeffOrdinaryCoeffsGeometryConfig`.
  The name keeps this distinct from the state-context geometry config and from
  future `txSz` table lookup.
- Let existing downstream validation handle zero extents, invalid planes,
  context-range failures, and arithmetic overflow. This wrapper only performs
  the spec shifts and does not introduce a second geometry policy.
- Add wrapper input enums instead of replacing existing lower-level inputs.
  Existing staged tests keep explicit geometry where useful, while future
  runtime wiring can move upward one boundary at a time.

## Risks / Trade-offs

- [Risk] The wrapper can be mistaken for full `txSz` table lookup.
  -> Mitigation: names, docs, specs, and matrix notes explicitly say callers
  still provide `Tx_Width[txSz]` and `Tx_Height[txSz]`-style dimensions.
- [Risk] Right-shifting non-multiple-of-four caller facts silently floors values,
  matching the spec expression but possibly hiding bad staged callers.
  -> Mitigation: tests focus on AV2-shaped multiples of four; broader syntax
  validation remains with the future runtime `coeffs()` integration.
- [Risk] Another crate-private staged API increases temporary surface area.
  -> Mitigation: keep the adapter thin, loaded-but-unwired, and proven by
  equivalence with the existing geometry handoff.
