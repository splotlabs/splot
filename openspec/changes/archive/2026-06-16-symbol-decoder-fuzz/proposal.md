## Why

Phase 9 requires every byte-consuming decoder stage to have bounded fuzz
coverage. The public AV2 symbol decoder already has unit and property tests, but
no cargo-fuzz target exercises arbitrary tile-payload bytes, literal widths,
symbol reads, malformed CDF rows, and `exit_symbol()` together.

## What Changes

- Add Feature ID `CONF-SYMBOL-DECODER-FUZZ`.
- Add a cargo-fuzz target named `symbol_decoder_bytes`.
- Feed bounded arbitrary tile-payload bytes into
  `splot_core::symbol::SymbolDecoder`.
- Drive a bounded operation stream over `read_bool`, `read_literal`,
  `read_symbol`, and `exit_symbol`.
- Generate both valid CDF rows and intentionally malformed CDF rows from fuzz
  input while keeping row lengths and operation counts finite.
- Update fuzz target lists, implementation/support matrices, generated status
  docs, and decoder conformance coverage metadata.

## Capabilities

### New Capabilities

### Modified Capabilities

- `conformance`: Add no-panic fuzz coverage for the public AV2 §8.2 symbol
  decoder byte-consuming API.
- `decoder-support`: Add a scoped supported `symbol-decoder-fuzz` row while
  keeping the existing `symbol-decoder` row partial for §8.3 CDF selection and
  runtime tile decode gaps.

## Impact

- Affected code: `fuzz/Cargo.toml` and
  `fuzz/fuzz_targets/symbol_decoder_bytes.rs`.
- Affected docs/status: `docs/IMPLEMENTATION-MATRIX.toml`,
  `docs/DECODER-SUPPORT-MATRIX.toml`, generated status docs,
  `docs/TESTING.md`, `AGENTS.md`, and decoder conformance coverage metadata.
- APIs: no public API changes.
- Diagnostics: no new public diagnostic rule; malformed symbol/CDF input remains
  represented by typed `splot_core::Error` values.
- Dependencies: no new third-party dependency and no AVM/dav2d integration.
- Runtime behavior: no `splot decode` behavior change.
