## Context

`cargo xtask gen-tables` already parses the committed AV2 section 9
`all_tables.h` attachment and emits numeric tables into `splot-core` and
`splot-tables`. Symbolic values are skipped unless a narrow resolver exists.
The generator currently resolves only the `BLOCK_*` symbols needed by
partition-size tables. The three transform-size tables needed by coefficient
runtime wiring (`Adjusted_Tx_Size`, `Tx_Size_Sqr`, and `Tx_Size_Sqr_Up`) contain
`TX_*` enum tokens, so they remain skipped even though AV2 section 6.19.6.1
defines their integer values.

## Goals / Non-Goals

**Goals:**
- Resolve only the `TX_*` symbols used by the three section 9.2 transform-size
  conversion tables.
- Keep table contents generated from `all_tables.h` and fail loudly on
  unsupported symbols.
- Preserve the existing generated-table drift check and source provenance.
- Add focused tests for the symbol resolver and representative generated values.
- Record decoder-support and implementation-matrix proof.

**Non-Goals:**
- Do not resolve unrelated symbolic tables such as transform-type or reserved
  tile scaling tables.
- Do not introduce public `TxSize` types or change any crate dependency edge.
- Do not wire the tables into the coefficient branch yet.
- Do not derive `txSzCtx`, scan order, `compute_tx_type`, dequantization,
  reconstruction, output, or reference refresh.

## Decisions

- Extend the existing generator symbol resolver rather than adding table copies
  in `splot-decode`. This keeps `all_tables.h` as the single source of truth and
  preserves `gen-tables --check` coverage.
- Keep the numeric output as `[i32; 25]`, matching the existing generated table
  representation. The table values are enum ordinals defined by AV2 section
  6.19.6.1.
- Keep the resolver table-scoped. If a non-TxSize symbol appears in one of the
  three supported tables, generation fails.
- Leave the remaining skipped symbolic tables in the allowlist with their
  existing reasons until their enum domains are modeled.

## Risks / Trade-offs

- [Risk] A TxSize ordinal map could drift from the spec.
  -> Mitigation: cite section 6.19.6.1 in the resolver comment and test edge
  ordinals including `TX_4X4`, `TX_64X64`, `TX_4X64`, and `TX_64X4`.
- [Risk] Generated numeric tables may be mistaken for runtime integration.
  -> Mitigation: tracking and roadmap notes explicitly state that coefficient
  wrappers still do not consume these tables and `txSzCtx` remains deferred.
