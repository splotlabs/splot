## ADDED Requirements

### Requirement: AV2 symbol decoder foundation

The decoder support model SHALL provide a bounded `splot-core` AV2 § 8.2 symbol
decoder foundation tracked by Feature ID `AV2-8.2-SYMBOL-DECODER`. The
foundation SHALL implement only generic symbol-decoder primitives over a
caller-provided tile payload byte slice: `init_symbol(sz)`, `read_bool()`,
`read_literal(n)`, `read_symbol(cdf)`, CDF update, and `exit_symbol()`
validation. It SHALL use generated repository-owned § 9 conversion tables, SHALL
validate caller-supplied CDF rows before indexing or updating them, and SHALL
return typed `splot-core` errors rather than panicking. It SHALL NOT implement
§ 8.3 syntax-element CDF selection, Tile/Saved CDF banks, default CDF
initialization, CDF copy/averaging, `decode_tile()`, tile syntax traversal,
runtime `splot decode` success, reconstruction, hash output, Y4M output,
reference refresh, AVM/dav2d invocation, or any new dependency/scheduler
surface.

#### Scenario: Initialization is tile-slice bounded

- **WHEN** a caller creates a symbol decoder over a finite tile payload byte
  slice
- **THEN** initialization follows AV2 § 8.2.2 using at most the first 15 coded
  bits, `SymbolRange = 1 << 15`, and signed `SymbolMaxBits = 8 * sz - 15`
- **AND** empty, one-byte, multi-byte, and large synthetic `sz` cases do not
  overflow or panic
- **AND** the decoder reads only from the provided tile payload slice, not from a
  parent OBU, IVF, or Annex B reader

#### Scenario: Boolean and literal reads are deterministic

- **WHEN** a caller reads pseudo-raw bits with `read_bool()` or `read_literal(n)`
- **THEN** boolean renormalization follows AV2 § 8.2.3, including implicit zero
  padding when `SymbolMaxBits` is negative
- **AND** literal reads follow AV2 § 8.2.5 by composing exactly `n`
  `read_bool()` values in MSB-first order
- **AND** literal widths outside the bounded implementation range are rejected
  with a typed error before unbounded work occurs

#### Scenario: CDF rows are validated before symbol decoding

- **WHEN** a caller passes a mutable CDF row to `read_symbol(cdf)`
- **THEN** the row is checked for a supported AV2 § 8.2.6 length, monotonic
  cumulative values, valid probability range, valid adaptation-rate index, and
  valid capped use count before any generated-table indexing occurs
- **AND** invalid rows return typed CDF errors without changing decoder state or
  mutating the row

#### Scenario: Symbol reads update CDFs only when enabled

- **WHEN** `read_symbol(cdf)` decodes a symbol from a valid row
- **THEN** it follows AV2 § 8.2.6 arithmetic renormalization using the generated
  `Prob_Inc` table
- **AND** it increments the frame-symbol count by one
- **AND** it updates CDF cumulative values and caps the row count at 32 when CDF
  update is enabled
- **AND** it leaves the CDF row byte-for-byte unchanged when CDF update is
  disabled

#### Scenario: Exit validates tile padding

- **WHEN** a caller finishes symbol decoding for a tile payload
- **THEN** `exit_symbol()` enforces the AV2 § 8.2.4 `SymbolMaxBits >= -14`
  requirement
- **AND** it validates the required trailing one bit and every zero padding bit
  up to byte alignment inside the tile payload
- **AND** malformed exit state, missing trailing one bit, and nonzero padding
  return typed errors without panicking

#### Scenario: Runtime decode remains unsupported

- **WHEN** a reader checks decoder support after the symbol decoder foundation is
  implemented
- **THEN** the `symbol-decoder` row states that only generic AV2 § 8.2
  primitives are available
- **AND** `tile-payload-decode`, § 8.3 CDF selection, `decode_tile()`,
  reconstruction, runtime hashes, runtime Y4M output, AVM/dav2d invocation, and
  CLI decode success remain unsupported or planned in their own rows
