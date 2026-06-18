## Context

`DECODE-COEFF-ALL-ZERO-BLOCK-STATE` allocates a zeroed local
`TransformCoeffBlockState` and applies end-of-block context-line writes for
`all_zero == 1`. `DECODE-COEFF-EOB-BRANCH-HANDOFF` dispatches `all_zero == 0`
to the derived EOB reader, but that path still has no local coefficient state
container for future scan traversal. The spec's `coeffs()` process initializes
the local coefficient arrays before either branch-specific work, so the nonzero
path should have the same zeroed block shell before it starts consuming EOB
syntax.

## Goals / Non-Goals

**Goals:**
- Add Feature ID `DECODE-COEFF-NONZERO-BLOCK-STATE`.
- Add crate-private nonzero block-start input/result types.
- Allocate zeroed `TransformCoeffBlockState` from caller-resolved 4x4 geometry
  before reading nonzero EOB syntax.
- Preserve transactional behavior: invalid geometry fails before CDF/symbol
  consumption, and invalid EOB selector facts leave the caller's coefficient
  context state, CDF rows, and symbol counters unchanged.
- Keep `coeff_loop.rs` under the 1000-line soft budget by moving branch-handoff
  code into a child module.

**Non-Goals:**
- No scan traversal, coefficient base/br/sign reads, `Level[]`/`Quant[]` nonzero
  writes, `read_quant`, dequantization, inverse transform, residual add, or
  reconstruction.
- No runtime nonzero block support; the minimal trace still exercises only the
  all-zero arm.
- No public API, CLI, dependency graph, encoder, AVM/dav2d wrapper, or
  diagnostic changes.

## Decisions

1. Allocate before reading EOB syntax.

   Geometry validation is independent of the arithmetic decoder. Allocating the
   local block first keeps malformed transform extents transactional: they fail
   before CDF rows or symbol bits are consumed.

2. Reuse `AllZeroCoeffBlockInput` for geometry.

   The existing type already carries the plane and transform-block coordinates
   that future end-of-`coeffs()` context updates will need. The nonzero helper
   uses only the dimensions today, but carrying the full geometry avoids adding a
   second shape type that would immediately need to converge later.

3. Split branch handoff code into a child module.

   `coeff_loop.rs` is at the soft line budget after the previous brick. Moving
   the branch input/result and dispatcher into `coeff_loop/branch.rs` keeps the
   main module focused on arithmetic/context helpers and leaves room for this
   feature without adding a new source-line advisory.

## Risks / Trade-offs

- The nonzero block-start output is still loaded ahead of real runtime use; the
  matrix/support rows keep the partial status explicit.
- Reusing the all-zero geometry input means the nonzero path receives `plane`,
  `x4`, and `y4` before it needs them. Tests focus on the contract that this
  helper allocates local state and does not mutate tile context state yet.
