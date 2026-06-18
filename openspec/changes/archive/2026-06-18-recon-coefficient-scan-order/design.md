## Context

The coefficient decode loop (§5.20.7) iterates transform coefficients in scan
order, and §7.14.4 places the decoded coefficients into `coefs` in that same
order. The order comes from §5.20.7.30 `get_scan(txSz, txClass)`. This is the
first prerequisite (after the coefficient CDF banks) toward that loop.

## Decisions

- **Pure function, caller-resolved shape.** `get_scan` computes
  `w = Min(Tx_Width[txSz], 32)` / `h = Min(Tx_Height[txSz], 32)` internally in the
  spec, but `splot-recon` cannot reach the §9.2 conversion tables, so (consistent
  with the §7.15 transforms) the function takes `w` / `h` directly. The result is
  a pure permutation of `0..w*h`, written into a caller `&mut [u16]` (positions
  are at most `w*h-1 = 1023`, so `u16` suffices and avoids allocation).

- **Faithful 2D anti-diagonal translation.** The spec 2D branch decrements `y`
  and tests `y < 0`, which underflows in `usize`; the implementation uses signed
  `i32` `x`/`y` and casts the `y*w + x` position to `u16`. The VERT and HORIZ
  branches are direct raster / transpose nested loops. The 4x4 2D order was
  hand-traced from the spec — `[0,4,1,8,5,2,12,9,6,3,13,10,7,14,11,15]` — and is
  pinned by a test (note: this is AV2's column-first diagonal, distinct from
  AV1's up-right `default_scan`).

- **`TransformClass` enum.** Names the spec `txClass` (`TwoD`/`Horizontal`/
  `Vertical` = `TX_CLASS_2D`/`HORIZ`/`VERT`). `get_tx_class` (the
  `PlaneTxType -> txClass` mapping) is a separate concern, deferred.

- **Self-contained, no-output-change.** No decode path calls it yet, so this is
  additive with no behavioral change. Correctness is proven by unit tests:
  exact 4x4 2D values, VERT identity, HORIZ transpose, and — the strongest check
  — that the output is a valid permutation of `0..w*h` for all 16 shapes and all
  three classes.

## Risks / Trade-offs

- **2D translation correctness** is the main risk; mitigated by the hand-traced
  4x4 assertion plus the all-shapes permutation-validity test (a wrong diagonal
  step would produce a duplicate or out-of-range index and fail it).
