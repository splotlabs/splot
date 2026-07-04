# Tasks

## Implementation

- [x] Record AVM decoder oracle hashes over the reused `CONF-AVM-VALID-STREAMS`
      corpus into `tests/conformance/decoder-oracle.toml`.
- [x] Add the capability taxonomy `tests/conformance/decoder-oracle-coverage.toml`.
- [x] Add the CI runner `crates/splot-cli/tests/decoder_oracle.rs` (in-process
      decode; `must_pass` oracle compare + `xfail_splot` fail-closed assertions;
      non-blocking XPASS with strict local mode; orphan gate).
- [x] Add `cargo xtask decoder-fixtures {verify,report,coverage}` and wire
      `verify` + `coverage --check` into `cargo xtask ci`.
- [x] Add local-only regeneration tooling under `tools/decoder-fixtures/`.
- [x] Document the system in `docs/CONFORMANCE.md` and keep the decoder-oracle
      coverage report available on demand through
      `cargo xtask decoder-fixtures coverage`.

## Tests and proof

- [x] Record proof in the `CONF-AVM-DECODE-ORACLE` row.

## Checks

- [x] `cargo xtask ci`
