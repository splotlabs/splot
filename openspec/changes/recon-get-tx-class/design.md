## Context

The coefficient decode loop (§5.20.7) scans coefficients in the order returned by
§5.20.7.30 `get_scan(txSz, txClass)`. The `txClass` argument is derived from the
block's `PlaneTxType` by §8.3.2 `get_tx_class(txType)`. With the scan order itself
already landed (`RECON-COEFFICIENT-SCAN-ORDER`), `get_tx_class` is the small
remaining selector that connects a transform type to its scan.

## Decisions

- **`const fn`, total over all inputs.** The spec `get_tx_class` is a three-way
  branch ending in an unconditional `else -> TX_CLASS_2D`, so the mapping is total
  and panic-free for any `usize` `PlaneTxType`. It needs no new error variant. It
  is a `const fn` so a fixed transform type resolves at compile time (a `const`
  assertion pins this).

- **Reuse `TransformClass`.** The scan-order brick already defined the
  `TransformClass` enum (`TwoD`/`Horizontal`/`Vertical` = `TX_CLASS_2D`/`HORIZ`/
  `VERT`). `tx_class` returns that same enum, so the two functions compose directly:
  `coefficient_scan_order(w, h, tx_class(plane_tx_type), out)`.

- **Spec-cited `TX_TYPE` literals.** The vertical types are `V_DCT` (10),
  `V_ADST` (12), `V_FLIPADST` (14); the horizontal types are `H_DCT` (11),
  `H_ADST` (13), `H_FLIPADST` (15) (`03-symbols.md` `TX_TYPE` values). All other
  values — `DCT_DCT`..`IDTX` (0..9) and any out-of-range input — take the spec
  `else` branch to `TX_CLASS_2D`. The literals are written inline with the spec
  symbol names in comments rather than introducing a `TX_TYPE` enum, which belongs
  with the §5.20.7.29 `compute_tx_type` producer not yet modeled.

- **Self-contained, no-output-change.** No decode path calls it yet, so this is
  additive with no behavioral change. Correctness is proven by a unit test that
  checks every vertical, horizontal, and 0..=9 value plus two out-of-range inputs.

## Risks / Trade-offs

- **Literal/spec-name drift** is the only real risk (a transposed value would
  misclassify a transform); mitigated by the exhaustive small-domain test that
  pins each named value to its class and the `else` fallback for out-of-range
  inputs.
