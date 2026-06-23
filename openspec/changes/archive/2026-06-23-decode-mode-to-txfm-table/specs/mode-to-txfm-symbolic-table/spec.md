## ADDED Requirements

### Requirement: Generate Mode_To_Txfm conversion table

The repository SHALL generate the AV2 §9.2 `Mode_To_Txfm` conversion table from
the committed `docs/spec/av2/1.0.0/attachments/all_tables.h` attachment by
resolving AV2 `TxType` symbols to their §3 Table 3.1 integer values. The
generated table SHALL be exposed in `splot-core::tables::conversion` and SHALL
remain covered by `cargo xtask gen-tables --check`.

#### Scenario: TxType symbols are resolved

- **WHEN** `cargo xtask gen-tables --check` runs
- **THEN** `Mode_To_Txfm` is emitted as a generated `MODE_TO_TXFM` integer array
  instead of appearing in the generator skip report

#### Scenario: Generated values match the mirror

- **WHEN** the core table spot tests run
- **THEN** `MODE_TO_TXFM` has `UV_INTRA_MODES_CFL_ALLOWED` entries matching the
  AV2 §9.2 mirror text and the AV2 §3 `TxType` integer assignments

#### Scenario: Scope remains table-only

- **WHEN** decoder support status is generated
- **THEN** the feature is recorded as generated table infrastructure only
- **AND** no runtime `compute_tx_type()`, coefficient-loop, reconstruction,
  output, reference-refresh, AVM/dav2d, or public decoder API behavior is
  claimed
