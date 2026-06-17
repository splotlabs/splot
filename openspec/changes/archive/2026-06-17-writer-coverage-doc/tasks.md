# Tasks

## xtask (additive)
- [x] `xtask/src/feature_status.rs`: `run_writer_coverage` (text + markdown) + `writer_rows` /
      `writer_coverage_markdown` / `writer_coverage_text`; a `WRITER_COVERAGE_DOC_PATH` const.
- [x] `xtask/src/main.rs`: `WriterCoverage { format, output }` subcommand + dispatch.
- [x] `check_writer_coverage_doc` called from `run_check_feature_status`, mirroring
      `check_coverage_doc` (regenerate + compare `docs/spec-coverage-writer.md`).

## Doc + reference
- [x] Generate `docs/spec-coverage-writer.md` (`cargo xtask writer-coverage --format markdown --output
      docs/spec-coverage-writer.md`).
- [x] `crates/splot-core/src/write/mod.rs`: point the module-doc reference at the landed doc.

## Tests and proof
- [x] An xtask unit test for `writer_coverage_markdown` (header / a known writable row present /
      deterministic). `check-feature-status` exercises the drift guard.

## Matrix and docs
- [x] `XTASK-FEATURE-STATUS` note + commands (the `writer-coverage` subcommand + the generated doc) and
      `ENC-BITSTREAM-WRITER` note (the writer-coverage matrix landed). Regenerate `docs/FEATURE-STATUS.md`
      / `docs/SPEC-COVERAGE.md` if a status field changes.

## Checks
- [x] `cargo xtask ci` and `openspec validate writer-coverage-doc --strict`
