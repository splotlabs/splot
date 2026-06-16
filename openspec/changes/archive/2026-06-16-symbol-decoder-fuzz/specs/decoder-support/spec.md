## ADDED Requirements

### Requirement: Decoder support matrix tracks symbol decoder fuzz coverage
The decoder support matrix SHALL include a supported row named
`symbol-decoder-fuzz`, tracked by Feature ID `CONF-SYMBOL-DECODER-FUZZ`, for
scoped no-panic fuzz coverage of the existing public
`splot_core::symbol::SymbolDecoder` API, without changing the existing
`symbol-decoder` row's partial status for unimplemented §8.3 and runtime
tile-decode behavior.

#### Scenario: symbol decoder fuzz evidence is scoped and test-backed
- **GIVEN** the generated decoder support status
- **WHEN** the `symbol-decoder-fuzz` row is rendered
- **THEN** it records `fuzz/fuzz_targets/symbol_decoder_bytes.rs` as evidence
- **AND** it records fuzz target enumeration, fuzz crate compilation, focused
  symbol decoder tests, and a local nightly fuzz smoke command
- **AND** it keeps broad §8.3 CDF selection, default Tile or Saved CDF banks,
  runtime tile payload traversal, reconstruction, output, reference refresh,
  AVM differential testing, dav2d differential testing, and support beyond the
  public §8.2 symbol decoder primitive out of scope
