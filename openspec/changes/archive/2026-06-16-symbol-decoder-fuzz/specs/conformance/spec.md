## ADDED Requirements

### Requirement: symbol decoder fuzz target
The repository SHALL provide a cargo-fuzz target named `symbol_decoder_bytes`,
tracked by Feature ID `CONF-SYMBOL-DECODER-FUZZ`, that drives the public AV2
§8.2 `splot_core::symbol::SymbolDecoder` byte-consuming API with bounded
payload bytes, bounded operation streams, and bounded CDF rows.

#### Scenario: arbitrary symbol inputs return typed results
- **WHEN** the fuzz target receives arbitrary bytes
- **THEN** it uses a bounded prefix as the tile payload for
  `SymbolDecoder::with_config`
- **AND** it drives a bounded sequence of `read_bool`, `read_literal`,
  `read_symbol`, and `exit_symbol` operations
- **AND** successful operation results satisfy local public-API invariants
- **AND** malformed CDF rows, invalid literal widths, exhausted payload bytes, or
  invalid symbol decoder states are represented by typed `splot_core::Error`
  returns without panicking

#### Scenario: symbol decoder fuzzing remains bounded
- **WHEN** fuzz input requests larger payloads, operation counts, literal
  widths, CDF row arities, CDF values, or CDF mutation counts than the target
  permits
- **THEN** the target clamps those values to fixed CI-safe limits before
  invoking the symbol decoder

#### Scenario: symbol decoder fuzzing does not claim tile decode
- **WHEN** the target is recorded in implementation and support status
- **THEN** the status text states that the target covers only the public §8.2
  symbol decoder primitive
- **AND** it does not claim §8.3 syntax-element CDF selection, default Tile or Saved
  CDF-bank initialization, tile traversal, partition decoding, block syntax,
  reconstruction, runtime hash output, runtime Y4M output, reference refresh,
  AVM evidence, dav2d evidence, filesystem I/O, network I/O, subprocesses, or new
  dependencies

#### Scenario: smoke automation enumerates the target
- **WHEN** `cargo xtask check-fuzz-targets` or `cargo xtask fuzz` enumerates
  cargo-fuzz targets
- **THEN** `symbol_decoder_bytes` is included in target execution without
  hardcoding the executable target list in CI workflow files
