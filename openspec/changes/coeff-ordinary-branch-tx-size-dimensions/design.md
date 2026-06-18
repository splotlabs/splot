## Context

The ordinary coefficient branch has staged wrappers for `PlaneTxType -> txClass`,
`plane -> ptype`, block geometry, and `coeffs()` geometry. The newest wrapper
accepts `startX`, `startY`, and caller-resolved transform dimensions, but a real
AV2 § 5.20.7.27 `coeffs(plane, startX, startY, txSz)` caller provides `txSz`.
Generated `splot-core` § 9.2 conversion tables already expose
`Tx_Width[txSz]`, `Tx_Height[txSz]`, `Tx_Width_Log2[txSz]`, and
`Tx_Height_Log2[txSz]`.

## Goals / Non-Goals

**Goals:**
- Add a crate-private ordinary branch wrapper that accepts `txSz` with
  `plane`, `startX`, and `startY`, derives generated width/height and log2
  transform-size facts, and delegates to the existing `coeffs()` geometry
  handoff.
- Use the generated `splot-core::tables::conversion` arrays rather than
  hand-written dimension tables.
- Reject invalid `txSz` indices before table indexing and before mutating
  coefficient state, CDF rows, or symbol-decoder state.
- Record focused implementation-matrix and decoder-support proof.

**Non-Goals:**
- Do not derive `Tx_Size_Sqr[txSz]`, `Tx_Size_Sqr_Up[txSz]`, or `txSzCtx` until
  the enum-valued conversion tables are modeled as generated Rust tables.
- Do not derive `Adjusted_Tx_Size[txSz]` or adjusted scan-row geometry.
- Do not implement AV2 section 5.20.7.29 `compute_tx_type`.
- Do not derive scan order, coefficient-CDF q context, parity/TCQ facts, or
  lossless state.
- Do not wire runtime `coeffs()`, selector derivation from real syntax,
  dequantization, inverse transform, residual add, output, or reference refresh.
- Do not change public APIs, CLI behavior, dependencies, or decode output.

## Decisions

- Keep the wrapper in `coeff_loop/ordinary_pass/geometry.rs`. The new adapter is
  another geometry-facing handoff and can reuse the existing delegation path
  without growing `ordinary_pass.rs`.
- Add a small `CoeffOrdinaryTxSizeGeometryConfig` that carries only `plane`,
  `start_x`, `start_y`, and `tx_size`. Nonzero inputs carry `is_inter` and one
  remaining base-config struct for the facts that are still not derived.
- Build `NonZeroCoeffEobContextInput`,
  `CoeffOrdinaryCoeffsGeometryConfig`,
  `CoeffOrdinaryBranchPlaneTxTypeBaseConfig`, and
  `CoeffOrdinaryGeometryStateContextConfig` from the same derived dimension
  facts. This prevents callers from supplying contradictory width/log2 facts at
  the new wrapper.
- Return the existing ordinary branch error type with a new invalid-transform-size
  variant. This keeps staged callers on one branch API while preserving
  fail-atomic validation before any delegated mutation.

## Risks / Trade-offs

- [Risk] The wrapper can be mistaken for full `txSz` integration.
  -> Mitigation: docs, specs, and matrix notes explicitly list `txSzCtx`,
  `Adjusted_Tx_Size`, scan derivation, and `compute_tx_type` as deferred.
- [Risk] `txSzCtx` remains caller-resolved, so one transform-size fact can still
  contradict `txSz`.
  -> Mitigation: keep the feature name scoped to dimensions/log2s and leave
  enum-valued table generation as the next boundary before deriving `txSzCtx`.
- [Risk] A generated table value conversion could be assumed infallible.
  -> Mitigation: convert table entries through checked integer conversion and
  return a typed branch error if a generated value is not a non-negative
  dimension.
