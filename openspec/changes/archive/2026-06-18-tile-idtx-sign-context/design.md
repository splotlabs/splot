## Context

`cdf/coeff_context.rs` now holds every §8.3.2 coefficient context except
`idtx_sign` (08-parsing-process.md lines 1429-1444). It is the sign context for
the identity transform and the last one needed before the coefficient layer is
complete.

## Decisions

- **Caller-provided `QuantSign[]` and `Level[]` slices.** Like `dc_sign` /
  `coeff_base`, the function takes the per-transform-block row-major `txw`-wide
  slices. `QuantSign[]` is signed (`-1` / `0` / `+1`, hence `&[i32]`); `Level[]` is
  the magnitude buffer (`&[u32]`). The edge neighbours are gated by `col > 0` /
  `row > 0` (the spec's own guards), and reads past either slice contribute `0`.

- **Total / panic-free `const fn`.** The flat index uses `saturating_mul` /
  `saturating_add` and a slice-length guard; `signc` is `i32` (it ranges `-3..=3`);
  the returned `ctx` is `usize` (`0..=8`). A module-level `const` spec-contract
  check is the non-test consumer (so no `#[allow(dead_code)]`).

- **Two-stage context.** The base context comes from `signc`; the `+2` level
  adjustment only applies when the base is non-zero and `Level[row][col] >
  COEFF_BASE_RANGE`, exactly per the spec.

## Risks / Trade-offs

- **Sign-sum / threshold fidelity** is the main risk (the five-way `signc`
  mapping, the `> COEFF_BASE_RANGE` (strict) threshold, and the non-zero-base gate
  on the `+2`). Mitigated by tests pinning each `signc` bucket, the level-threshold
  raise (including the `== COEFF_BASE_RANGE` no-raise boundary and the
  zero-context no-raise), the missing-edge-neighbour skips, and short-slice /
  pathological-geometry totality.
