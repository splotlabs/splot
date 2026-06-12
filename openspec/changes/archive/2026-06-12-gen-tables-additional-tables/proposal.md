# Proposal: generate the § 9 additional tables (gen-tables codegen)

## Feature IDs

- `AV2-9-ADDITIONAL-TABLES` (tables.rs stops being an 11-line stub; the
  recorded plan: `cargo xtask gen-tables` from `all_tables.h`)
- `XTASK-GEN-TABLES` (create the row if the schema wants a tooling id —
  follow the XTASK-* precedent)

## Why

`crates/splot-core/src/tables.rs` is a stub and the planned codegen does
not exist. The § 9 tables are a hard prerequisite for § 5.20 symbol
decoding (CDFs, scan orders) and feed quantizer-matrix checks. The
licensing rule is absolute: never hand-transcribe table contents —
generate from the spec's `all_tables.h` attachment with provenance
recorded.

## What Changes

1. Commit the spec's `all_tables.h` attachment under the existing
   quarantined mirror path (`docs/spec/av2/1.0.0/attachments/`), with
   `provenance.toml` extended (URL + sha256) and the
   `check-spec-mirror` gate covering it; update
   `docs/references/THIRD-PARTY-NOTICES.md` factually (same AOMedia
   quarantine, same version). This stays within the maintainer-approved
   mirror exception — surfaced explicitly in the PR for veto.
2. `cargo xtask gen-tables`: parse the C header and emit
   `crates/splot-core/src/tables/` generated Rust (typed constants with
   a generated-from provenance header and the standard SPDX header per
   the annex_a transcribed-constants precedent), plus a drift check
   (`gen-tables --check`) wired into `cargo xtask ci`.
3. Generate at least the § 9.2 conversion tables and the § 9.4
   quantizer-matrix tables now (near-term consumers); the generator must
   cover the whole header or fail loudly on unhandled constructs (no
   silent truncation) — CDF groups may land as opt-in modules if size
   demands, with what-was-skipped recorded.
4. tables.rs re-exports the generated modules; the matrix row advances
   with proof; the stub TODO clears.

## Non-goals

- Consuming the tables (§ 5.20 work is item 26).
- Hand-editing any generated content.

## Acceptance criteria

- [ ] gen-tables regenerates byte-identically (drift check in CI);
  provenance recorded end to end; spot tests assert known table values
  against the mirror's § 9 text (cited); no hand-transcription anywhere;
  `cargo xtask ci` green.
