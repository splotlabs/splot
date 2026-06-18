## Context

`cdf/coeff_context.rs` holds the position-only contexts, `coeff_br`, and the IDTX
variants. `coeff_base` (08-parsing-process.md lines 1295-1370) is the main
significant-coefficient context and the most intricate: a neighbour-magnitude sum
feeds a base context that selects one of five CDF banks.

## Decisions

- **Return an enum, not a bare ctx.** `coeff_base` selects one of five banks
  (`TileCoeffBasePhCdf` / `TileCoeffBaseLfUvCdf` / `TileCoeffBaseUvCdf` /
  `TileCoeffBaseLfCdf` / `TileCoeffBaseCdf`), each with a different context offset.
  `select` returns `CoeffBaseSelection::{Ph,LfUv,Uv,Lf,Hf}{ctx}` so the caller
  maps the variant to the bank and supplies the `txSzCtx` / `tcqState` dimensions
  the `Lf` / `Hf` banks carry. This keeps the bank-array indexing (which needs
  state this module doesn't model) at the caller while the §8.3.2 context math
  lives here.

- **Use the generated `SIG_REF_DIFF_OFFSET`.** Unlike `Mag_Ref_Offset_With_Tx_Class`
  (spec-inline, hand-written for `coeff_br`), `Sig_Ref_Diff_Offset` is in
  `all_tables.h` and is generated into `splot_core::tables::conversion`. The
  function reads the generated static (no duplicate), so it is a regular `fn`, not
  a `const fn` (a `const fn` cannot read a `static`); the unit tests are its
  consumer (verified no `dead_code` under the gating clippy on the pinned
  toolchain).

- **Caller-resolved scalars, no `splot-recon` import.** `txClass` is a scalar
  index (0/1/2, out-of-range treated as 2D), consistent with `coeff_br`; the
  geometry (`bwl`/`txw`/`txh`) is caller-resolved. The entropy layer stays off the
  `splot-decode` → `splot-recon` edge.

- **Total / panic-free.** `row`/`col` use checked shifts; the flat neighbour index
  uses `saturating_mul`/`saturating_add` and a `flat < level.len()` guard, exactly
  matching the spec's `refRow < height && refCol < width` guard.

- **Parity-hidden override.** `isHidden && c == 0` selects `Ph{Min(ctx,4)}`
  regardless of plane/frequency, and forces `magLimit` to 3 (the `!(isHidden && c
  == 0)` gate), so it is checked first after the sum.

## Risks / Trade-offs

- **Branch fidelity** is the main risk: the magLimit conditional, the five-bank
  selection, the chroma U-vs-V and 2D-vs-non-2D offsets, the low-frequency
  c==0 / row+col / horiz-col-vert-row sub-branches, and the high-frequency
  row+col buckets. Mitigated by tests pinning every branch, plus a clamped-sum
  test, a magLimit-raise test, a parity-hidden test, a `num`-distinguishing test
  (chroma reads 3 not 5), and short-slice / pathological-geometry totality.
