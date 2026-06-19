## Context

`RECON-DEBLOCK-SAMPLE-FILTER` left `maxWidthNeg` / `maxWidthPos` caller-resolved.
§ 7.17.3 derives them from the § 7.17.4 filter size, the plane, and whether the
edge is at a super-block boundary — a small table-free branching process.

## Goals / Non-Goals

**Goals:** a total `const fn` for the § 7.17.3 width derivation, a companion to
the sample filter in the same module.

**Non-Goals:** the § 7.17.4 filter size (a caller-side `Min`), the § 7.17.5/§
7.17.6 adaptive strength (segment/qindex state), and any runtime wiring.

## Decisions

- **Caller-resolved `filter_size` and `is_chroma`.** § 7.17.4 sets
  `filterSize = Min(Tx_Width/Height[prevTxSz], Tx_Width/Height[txSz])` from the
  § 9.2 conversion tables `splot-recon` cannot reach; the caller computes that
  `Min` and passes the scalar. `is_chroma` is the spec `plane != 0`.
- **`const fn`, no error path.** Every `(filter_size, is_chroma, sb_edge)` maps to
  a defined pair (a `filter_size` of `0` or any non-transform value falls in the
  `<= 4` bucket), so there is nothing to reject; a module-level `const` pins the
  luma `filter_size == 32` case as a compile-time contract.

## Risks / Trade-offs

- It is small and loaded ahead of its runtime caller, matching the established
  pattern of building the deblock derivations before the edge traversal; the
  matrix and roadmap keep those out of scope. The branch-coverage test pins every
  spec bucket and the super-block cap so a transcription error is caught.
