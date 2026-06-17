# Change: writer-coverage-doc

## Feature IDs

- `XTASK-FEATURE-STATUS` (extends the matrix-driven coverage tooling with a writer view)
- `ENC-BITSTREAM-WRITER` (the deferred `docs/spec-coverage-writer.md` per-structure coverage matrix)

## Why

The writer surface (`splot-core::write`) is complete for all 14 OBU payload types plus the tile-group
continuation, but the only writer-wide coverage record is prose in `docs/IMPLEMENTATION-MATRIX.toml`
notes and a forward reference in `crates/splot-core/src/write/mod.rs` ("see
`docs/spec-coverage-writer.md` (once landed)"). Its three sibling coverage documents
(`docs/FEATURE-STATUS.md`, `docs/SPEC-COVERAGE.md`, `docs/DECODER-SPEC-COVERAGE.md`) are all generated
from the matrix and drift-guarded. This change adds the matching generated, drift-guarded writer
coverage document so the writer surface has a first-class, never-stale index.

## What changes

- **xtask** (`xtask/src/feature_status.rs`, additive): add `run_writer_coverage` (text + markdown) that
  renders, from `docs/IMPLEMENTATION-MATRIX.toml`, one row per writable feature — every
  `bitstream-syntax` feature plus every feature with a landed writer (`write` `done` / `partial`) — with
  its spec section(s), feature id, name, `write` maturity, and module, plus a `write`-status count
  summary. A new `cargo xtask writer-coverage [--format text|markdown] [--output PATH]` subcommand
  (`xtask/src/main.rs`).
- **Drift guard**: `check-feature-status` regenerates and compares `docs/spec-coverage-writer.md`
  (`check_writer_coverage_doc`), exactly as it already guards `FEATURE-STATUS.md` and
  `SPEC-COVERAGE.md`, so the doc can never drift from the matrix.
- **Doc** (`docs/spec-coverage-writer.md`, new, generated — "Do not edit by hand").
- **Reference**: update `crates/splot-core/src/write/mod.rs` to point at the now-landed doc.

## Validator impact

None.

## Non-goals

- No new structured matrix field for the per-writer round-trip guarantee (byte-exact vs.
  canonicalizing) — that nuance stays in each row's notes; the doc links to the matrix.

## Impact

- Crate: `xtask` (additive `writer-coverage` subcommand + drift guard); `crates/splot-core` (the
  `write/mod.rs` reference only).
- Docs: new generated `docs/spec-coverage-writer.md`; `docs/IMPLEMENTATION-MATRIX.toml`
  (`XTASK-FEATURE-STATUS` + `ENC-BITSTREAM-WRITER` notes) + regenerated `docs/FEATURE-STATUS.md` /
  `docs/SPEC-COVERAGE.md` if a status field changes.
