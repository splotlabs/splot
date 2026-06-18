## Context

`cdf/coeff_context.rs` holds the position-only contexts and `coeff_br`. The two
IDTX magnitude contexts (08-parsing-process.md lines 1412-1426) are the next
`Level[]`-reading §8.3.2 derivations and the simplest remaining ones — each reads
only the left (`Level[row][col-1]`) and above (`Level[row-1][col]`) neighbour.

## Decisions

- **Shared helper.** `coeff_base_idtx` and `coeff_br_idtx` differ only in the
  per-neighbour clamp (3 vs `MAX_BASE_BR_RANGE - 1`) and the final `Min(mag, 6)`
  (br only). A private `idtx_neighbour_mag(level, row, col, txw, clamp)` does the
  two clamped neighbour reads; the two public `const fn`s wrap it.

- **Caller-provided `Level[]` slice.** Like `coeff_br`, the functions read a
  caller-provided row-major `txw`-wide `u32` slice. No transform-class or
  plane/isLf input is needed (the IDTX contexts don't branch on them), so these
  stay free functions rather than a context struct.

- **Total / panic-free.** The spec reads are always in-block (`col-1`/`row-1`
  guarded by `col > 0`/`row > 0`), so no spec bounds check is required; the only
  totality concern is a short or mismatched caller slice. The flat index uses
  `saturating_mul`/`saturating_add` and a `flat < level.len()` guard, so any read
  past the slice contributes `0`. `const fn` (so the compile-time contract checks
  evaluate it).

- **Result is the spec `mag`.** Both contexts use `mag` directly as the inner
  index of `TileCoeffBaseIdtxCdf[Min(TX_16X16, txSzCtx)]` /
  `TileCoeffBrIdtxCdf[Min(TX_16X16, txSzCtx)]`; the `Min(TX_16X16, txSzCtx)` bank
  index is the caller's concern.

- **Derivation-only, no-output-change.** Not read by any decode path; the
  module-level `const` spec-contract check is the non-test consumer.

## Risks / Trade-offs

- **Clamp/skip fidelity** is the main risk (the 3 vs 5 clamp, the final
  `Min(mag, 6)` on br only, and skipping the missing edge neighbour). Mitigated by
  tests pinning each: clamped left+above sum, the col==0 / row==0 skips, the br
  clamp-then-6 path, and short-slice / pathological-geometry totality.
