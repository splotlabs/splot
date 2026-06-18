## Context

`DECODE-COEFF-SCAN-WALK` gives the nonzero coefficient path checked scan
entries after EOB syntax. The next coefficient reads need `TileCoeffBase*` and
`TileCoeffBr*` CDF rows, but the current tile CDF subset exposes only EOB and
DC-sign coefficient banks. The generated AV2 §9.3 defaults already live in
`splot-core`; this change keeps the new boundary in `splot-decode` and does not
add dependencies or runtime decode behavior.

## Goals / Non-Goals

**Goals:**

- Add `DECODE-COEFF-BASE-CDF-ROWS` as a focused feature row and decoder-support
  row.
- Store the ordinary non-IDTX coefficient base, base-EOB, and base-range CDF
  families in `CoeffCdfRows`, owned by `BlockCdfRows`, and load them from
  generated §9.3 defaults.
- Provide typed `CoeffCdfSelector` variants behind `TileCdfSelector::Coeff` for
  bounds-checked immutable and mutable row access.
- Include the rows in supported-subset tile copy, save/average, and frame-end
  count scaling.
- Prove the boundary with self-contained tests and no output change.

**Non-Goals:**

- No runtime `coeffs()` loop reads, base/br symbol consumption, sign reads, or
  state writes.
- No FSC, IDTX, parity-hidden-only, or Quant/Level update behavior.
- No new dependency, crate-graph change, public API, AVM/dav2d invocation,
  reconstruction, or scheduler change.

## Decisions

1. Store all quantization-context rows in the tile subset.

   Rationale: this matches the existing `txb_skip`, `eob_extra`, `eob_pt`, and
   `dc_sign` banks. It keeps row selection explicit at read time through
   `coeff_cdf_q_ctx` and avoids mixing this narrow row-exposure change with the
   broader §6.19.1 single-row `init_coeff_cdfs()` model.

   Alternative considered: collapse the stored banks to one active q-context.
   That would be a larger CDF lifecycle change and is better paired with a real
   coefficient loop consumer.

2. Use selector variants that mirror the generated bank families.

   Rationale: `CoeffCdfSelector::Base`, `BaseUv`, `BaseLf`, `BaseLfUv`,
   `BaseEob`, `BaseEobUv`, `BaseLfEob`, `BaseLfEobUv`, `Br`, `BrUv`, and `BrLf`
   make each selector's axes clear while keeping the large coefficient family in
   its own module. The public handoff remains one `TileCdfSelector::Coeff(...)`
   variant, and typed errors still name the exact CDF array. The context
   derivation helpers already produce bank-specific context indexes, so this
   layer should only validate indexes and return rows.

   Alternative considered: one broad enum with a bank discriminator and generic
   context fields. That would reduce match arms but blur error reporting and
   make family-specific bounds harder to review.

3. Keep the implementation loaded-but-unread.

   Rationale: this isolates CDF storage/selection from symbol sequencing,
   rollback, and coefficient-state mutation. The next brick can read base/BR
   symbols over `NonZeroCoeffScanWalk` entries using this tested boundary.

   Alternative considered: wire first base-symbol reads in the same PR. That
   would couple CDF plumbing to coefficient-loop semantics and widen review risk.

## Risks / Trade-offs

- Row-family volume can make `block_rows.rs` larger and harder to scan ->
  mitigate by isolating the coefficient family in `coeff_rows.rs` and running
  `cargo xtask check-source-lines`.
- Selector axes can be mixed up because several families have similar shapes ->
  mitigate with tests that compare selected rows against generated defaults for
  representative cells and assert typed `SelectorOutOfRange` errors per axis.
- Loaded-but-unread rows can be mistaken for runtime coefficient decode support
  -> mitigate with matrix/support notes and OpenSpec scenarios that explicitly
  exclude symbol consumption and output support.
