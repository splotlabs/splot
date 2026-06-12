# Tasks: § 9 tables codegen

## 1. Bookkeeping

- [x] 1.1 Matrix rows confirmed/created; `openspec_change` set; study
  the mirror §9 layout, the all_tables.h attachment URL
  (docs/SPEC-MAPPING.md), provenance.toml, the check-spec-mirror gate,
  and the THIRD-PARTY-NOTICES quarantine wording.

## 2. Attachment + provenance

- [x] 2.1 Fetch and commit all_tables.h under the mirror's
  attachments/ path; extend provenance.toml + the regenerate script +
  the check-spec-mirror gate; update THIRD-PARTY-NOTICES factually.

## 3. Codegen

- [x] 3.1 xtask gen-tables: C-header parser + Rust emitter with
  provenance headers; whole-header coverage or loud failure on
  unhandled constructs.
- [x] 3.2 Generate §9.2 + §9.4 (and what is cheap); drift check wired
  into ci; tables.rs re-exports; stub TODO cleared.

## 4. Verification

- [x] 4.1 Spot tests against the mirror §9 text (cited); regeneration
  byte-identity test; proptests not needed (constants).
- [x] 4.2 `check-feature-status` + `check-diagnostic-registry` pass.
- [x] 4.3 `cargo xtask ci` (bare, exit checked) passes.
