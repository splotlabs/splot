## ADDED Requirements

### Requirement: Shared dependency-free spec-tables crate

The repository SHALL provide a `splot-tables` crate that is the single home for
the AV2 § 9 transform-kernel tables (the § 9.6 1D transform and § 9.7 secondary
transform tables), tracked by `INFRA-SHARED-SPEC-TABLES`. The crate SHALL depend
on no other `splot-*` crate and no external crate, so any crate may depend on it
without affecting the one-way dependency direction. The tables SHALL be generated
verbatim by `cargo xtask gen-tables` from the committed § 9 attachment and SHALL
NOT be hand-edited; the generator SHALL route the § 9.6/§ 9.7 transform-kernel
modules into this crate and the remaining § 9 modules into `splot-core::tables`,
and SHALL write and drift-check every output directory. Moving the transform
modules SHALL NOT change any generated table value.

#### Scenario: Transform-kernel tables build and cross-check in the shared crate

- **WHEN** `cargo test -p splot-tables --locked` runs
- **THEN** the crate builds with no `splot-*` and no external dependency
- **AND** a mirror cross-check spot test asserts a generated transform-kernel
  table against the committed § 9 Markdown mirror

#### Scenario: The generator drift-checks both output directories

- **WHEN** `cargo xtask gen-tables --check` runs in `cargo xtask ci`
- **THEN** it regenerates the § 9 tables into both `crates/splot-core/src/tables/`
  and `crates/splot-tables/src/tables/` and fails on any drift, missing file, or
  stray generated file in either directory
- **AND** the generated-table count is unchanged, proving the relocation changed
  no table content

#### Scenario: The crate is a dependency-free leaf

- **WHEN** `cargo xtask check-dependency-direction` runs
- **THEN** `splot-tables` is recorded as depending on no internal crate
- **AND** the consumer `splot-recon -> splot-tables` edge is introduced only with
  the § 7.15 inverse-transform work, not by this change
