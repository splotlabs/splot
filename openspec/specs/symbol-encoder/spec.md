# symbol-encoder Specification

## Purpose
TBD - created by archiving change range-encoder-complete. Update Purpose after archive.
## Requirements
### Requirement: AV2 symbol encoder primitive

`splot-core` SHALL provide an I/O-free AV2 v1.0.0 § 8.2 symbol encoder
primitive that writes tile-payload bytes for the generic entropy operations
inverse to `SymbolDecoder`: `write_bool`, `write_literal(n)`,
`write_symbol(cdf, symbol)`, and `finish`/`exit_symbol` finalization. The
primitive SHALL be safe Rust, panic-free for caller input, deterministic, bounded
by explicit output byte and primitive operation-count limits, and independent of
`splot-decode`, `splot-encode`, external codecs, filesystem I/O, and scheduler
state.

The primitive SHALL use the same generated § 9.2 conversion tables and CDF
adaptation arithmetic as `SymbolDecoder`. It SHALL validate caller-supplied CDF
rows before indexing or mutating them, SHALL reject an out-of-range requested
symbol before changing encoder state or the row, and SHALL update rows exactly as
`SymbolDecoder::read_symbol` would when CDF updates are enabled.

#### Scenario: Boolean and literal operations decode back

- **WHEN** a caller writes a finite sequence of `write_bool` and
  `write_literal(n)` operations with `n <= 32`, finalizes the payload, and then
  drives `SymbolDecoder` through the same operation sequence
- **THEN** every decoded boolean and literal value SHALL equal the value that was
  encoded
- **AND** `SymbolDecoder::finish()` SHALL accept the encoder's final padding.

#### Scenario: CDF symbols decode back with matching updates

- **WHEN** a caller writes symbols for valid CDF rows of arity `N = 2..=8`,
  finalizes the payload, and then drives `SymbolDecoder::read_symbol` through
  the same initial rows
- **THEN** every decoded symbol SHALL equal the encoded symbol
- **AND** every decoder CDF row SHALL equal the encoder CDF row after the
  corresponding operation when CDF updates are enabled
- **AND** both encoder and decoder SHALL leave CDF rows unchanged when CDF
  updates are disabled.

#### Scenario: Malformed encode requests fail before mutation

- **WHEN** a caller passes an invalid CDF row, a symbol outside the CDF arity,
  or a literal width greater than 32
- **THEN** the encoder SHALL return a typed writer error
- **AND** SHALL NOT mutate committed output bytes, arithmetic state, symbol
  count, or the caller's CDF row for the rejected operation
- **AND** SHALL NOT panic.

#### Scenario: Finalization emits valid tile-payload padding

- **WHEN** a caller finalizes an otherwise valid symbol encoder stream
- **THEN** the returned payload SHALL include the required AV2 § 8.2.4 trailing
  one bit and zero padding to byte alignment
- **AND** a fresh `SymbolDecoder` over the payload SHALL accept
  `exit_symbol()`/`finish()`.

#### Scenario: Output and operation log are bounded and deterministic

- **WHEN** the same finite operation sequence and configuration are encoded
  repeatedly
- **THEN** the payload bytes SHALL be byte-identical across runs
- **AND** if encoding would exceed the configured output byte limit, the encoder
  SHALL return a typed writer error before exceeding the limit
- **AND** if encoding would exceed the configured primitive operation-count
  limit, including from valid high-skew CDF rows that produce zero-bit symbols,
  the encoder SHALL return a typed writer error before mutating committed state
  or caller CDF rows.

#### Scenario: Broader entropy and tile syntax remain separate

- **WHEN** the symbol encoder primitive is available
- **THEN** it SHALL NOT claim support for AV2 § 8.3 syntax-element CDF selection,
  default § 9.3 CDF-bank ownership, Tile/Saved/Frame CDF lifecycle,
  `decode_tile()`/`encode_tile()` traversal, coefficient syntax, reconstruction,
  packet production, or public `splot encode` success.
