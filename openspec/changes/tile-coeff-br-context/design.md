## Context

`cdf/coeff_context.rs` already holds the position-only coefficient base contexts.
`coeff_br` (08-parsing-process.md lines 1491-1541) is the first coefficient
context that reads the per-transform-block `Level[]` magnitudes, so it is the
natural next §8.3.2 derivation.

## Decisions

- **Struct + method, not a free function.** `coeff_br` needs seven scalar inputs
  (`pos`, `bwl`, `txw`, `txh`, `plane`, `is_lf`, `tx_class`) plus the level slice;
  a free function would trip `clippy::too_many_arguments`. `CoeffBrContext` groups
  the scalars and exposes `ctx(&[u32])`, mirroring the `block_context.rs`
  `YModeIndexContext` idiom.

- **Caller-provided `Level[]` slice.** The full per-transform-block level buffer
  does not exist yet (it is written by the `coeffs()` loop). The context reads a
  caller-provided row-major `txw`-wide `u32` slice (`level[row * txw + col]`),
  consistent with the caller-resolves convention. `Level[]` values are
  non-negative coefficient magnitudes, hence `u32`.

- **Total / panic-free.** The spec guards each neighbour read with
  `refRow < txh && refCol < txw`; the implementation adds `flat < level.len()` so
  a short or mismatched slice cannot panic. The body is a `const fn` (a `while`
  loop, not `for`, since iterators are not const), so the compile-time contract
  checks evaluate it.

- **`MAG_REF_OFFSET_WITH_TX_CLASS` inline table.** The spec
  `Mag_Ref_Offset_With_Tx_Class[3][3][2]` is given inline in §8.3.2 (not a §9
  table), so it is hand-written and spec-cited, indexed by the spec `txClass`
  value (`TX_CLASS_2D` = 0, `HORIZ` = 1, `VERT` = 2) via a `TransformClass` match.
  The non-2D-chroma `num = 2` branch reads only the first two offsets.

- **Derivation-only, no-output-change.** Not read by any decode path; the
  module-level `const` spec-contract check is the non-test consumer (the lesson
  from the position-context brick), so no `#[allow(dead_code)]`.

## Risks / Trade-offs

- **Branch/offset fidelity** is the main risk (the plane/DC/LF `+7` offsets, the
  chroma `Min(mag, 3)` clamp, the `num = 2` non-2D-chroma case, and the table
  rows). Mitigated by tests that pin each branch, the halving-and-clamp-to-6 path,
  a test that distinguishes `num = 2` from `num = 3` (a value only the third
  offset would change), and an out-of-bounds/short-slice totality test.
