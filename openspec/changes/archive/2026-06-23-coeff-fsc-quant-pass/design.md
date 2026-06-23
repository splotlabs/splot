## Context

`DECODE-COEFF-FSC-LEVEL-PASS` and `DECODE-COEFF-FSC-SIGN-PASS` implement the
loaded-but-unwired FSC/IDTX level and sign portions of AV2 §5.20.7.27. The
second FSC loop still stops before the spec's `read_quant(level, pos, 0,
NUM_BASE_LEVELS + COEFF_BASE_RANGE + 1, hrLevelAvg, 0)` call and the signed
`Quant[pos]` write.

The ordinary non-FSC path already has total, typed helpers for §5.20.7.28
`read_quant` and for signed `Quant[]`, `culLevel`, and `dcCategory` state
updates. The FSC path should reuse those mechanics where the spec facts are the
same, while keeping the runtime `coeffs()` caller unwired until the broader
coefficient loop integration can commit all block effects in syntax order.

## Goals / Non-Goals

**Goals:**

- Add a crate-private FSC sign/quant pass that consumes the completed FSC level
  pass and steps the second-loop sign and quant operations in spec order.
- Read `idtx_sign` and then `read_quant` over checked `0..segEob` entries with
  FSC constants:
  `isHidden = 0`, `allowTcq = 0`, and `maxLevel =
  NUM_BASE_LEVELS + COEFF_BASE_RANGE + 1`.
- Compute final signed `Quant[pos]`, `culLevel`, and `dcCategory` while writing
  `QuantSign[]` before later sign contexts can observe it.
- Keep static validation fail-atomic before any literal consumption or block
  mutation.

**Non-Goals:**

- Runtime `coeffs()` wiring.
- Tile context-line commits for the FSC path.
- Dequantization, inverse transform, residual add, reconstruction/output, or
  reference refresh.
- New dependencies, public APIs, AVM/dav2d integration, or broad `decode_tile()`
  support.

## Decisions

- Reuse `CoeffReadQuantState` instead of duplicating §5.20.7.28 parsing.
  Alternative considered: add an FSC-only parser. Rejected because the syntax is
  identical once the FSC constants are supplied.
- Reuse the quant-state accumulator for signed `Quant[]` arithmetic after making
  its no-write per-entry step available crate-private. Alternative considered:
  duplicate the small FSC arithmetic. Rejected because the ordinary helper
  already centralizes overflow-checked signed quant, `culLevel`, and
  `dcCategory` behavior.
- Reuse the FSC sign-pass derivation and symbol-read helpers inside the quant
  pass rather than consuming a completed sign pass. Alternative considered: run
  the sign pass first and then the quant pass. Rejected because AV2 §5.20.7.27
  alternates `idtx_sign` and `read_quant` within the same `c = 0..segEob` loop
  over a shared symbol decoder.
- Keep the FSC quant pass crate-private and loaded-but-unwired. This avoids
  exposing partial symbol-stream behavior until the real runtime `coeffs()` loop
  can call the level, sign, and quant steps in the full block context.

## Risks / Trade-offs

- Duplicate intermediate storage for reads/writes before mutation -> bounded by
  `segEob` and accepted to preserve block fail-atomicity for write preflight.
- The helper still consumes literals if a later decoded quant overflows the local
  signed representation -> this mirrors existing staged parser behavior; block
  mutation is deferred until all writes are computed.
- The helper writes local `QuantSign[]` before later entries are known -> required
  by §5.20.7.27 because later `idtx_sign` contexts use prior local signs. The
  state remains local to the returned pass.
- The pass does not commit above/left context lines -> tracked as an explicit
  follow-on so this brick stays narrow and reviewable.
