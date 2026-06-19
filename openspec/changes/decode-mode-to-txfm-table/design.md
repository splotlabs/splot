## Context

`cargo xtask gen-tables` parses the committed AV2 §9 `all_tables.h`
attachment, groups declarations by the spec mirror, and emits deterministic
Rust arrays. It already resolves a small set of symbolic §9.2 table domains:
`BLOCK_*` for partition-size tables and `TX_*` for three transform-size tables.

`Mode_To_Txfm[UV_INTRA_MODES_CFL_ALLOWED]` is also a §9.2 conversion table, but
it is skipped because its elements are `TxType` symbols such as `DCT_DCT` and
`ADST_DCT`. AV2 §5.20.7.29 `compute_tx_type()` needs this table for intra
chroma transform fallback after directional wide-angle remapping. The ordinary
coefficient branch is already staged up to caller-resolved `PlaneTxType`, so
this change removes a concrete table-generation blocker without wiring runtime
state.

## Goals / Non-Goals

**Goals:**

- Generate `MODE_TO_TXFM` from the committed AV2 §9 attachment.
- Resolve only the AV2 `TxType` symbol domain needed by `Mode_To_Txfm`.
- Prove the generated values against the mirror text and symbol constants from
  AV2 §3.
- Update status docs so `compute_tx_type()` remains honestly incomplete.

**Non-Goals:**

- No implementation of AV2 §5.20.7.29 `compute_tx_type()`.
- No transform-set membership, `TxTypes` tile state, luma tx-size state, mode
  syntax, wide-angle mapping, or runtime `coeffs()` wiring.
- No generated Rust enums or public table API redesign.
- No dependency graph or licensing change.

## Decisions

1. Extend `block_symbols.rs` instead of hand-writing `MODE_TO_TXFM`.

   The table is already present in the pinned attachment. Resolving symbols in
   the generator preserves the existing source-of-truth model and keeps
   `gen-tables --check` as the drift gate.

2. Add a narrow `TxType` resolver.

   The resolver maps the 16 AV2 `TxType` symbols from §3 Table 3.1 to their
   integer values and is activated only for `Mode_To_Txfm`. This mirrors the
   existing narrow block-size and transform-size resolvers and avoids pretending
   the generator understands every symbolic AV2 table.

3. Keep other skipped symbolic tables skipped.

   `Max_Tx_Size_Rect`, the two `Size_To_Tx_Type_Group_*` tables, and the
   `reserved` tile-scaling tables have different symbolic domains or placeholder
   values. Resolving them is separate work.

## Risks / Trade-offs

- Wrong TxType ordinal map -> Mitigation: derive the mapping from AV2 §3 Table
  3.1, add direct unit coverage for representative symbols, and add mirror
  spot checks for every `Mode_To_Txfm` entry.
- Overclaiming decoder progress -> Mitigation: matrix/support notes explicitly
  keep `compute_tx_type()` and runtime coefficient decoding as residual work.
- Generator scope creep -> Mitigation: route only `Mode_To_Txfm` through the new
  resolver and leave unrelated skip-allowlist entries intact.
