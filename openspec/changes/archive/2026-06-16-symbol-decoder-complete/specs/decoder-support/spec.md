## MODIFIED Requirements

### Requirement: AV2 symbol decoder foundation

The decoder support model SHALL provide a bounded `splot-core` AV2 § 8.2 symbol
decoder primitive tracked by Feature ID `AV2-8.2-SYMBOL-DECODER`, and SHALL mark
its `symbol-decoder` support row `supported`. The primitive SHALL implement the
generic symbol-decoder operations over a caller-provided tile payload byte
slice: `init_symbol(sz)` (§ 8.2.2), `read_bool()` (§ 8.2.3), `read_literal(n)`
(§ 8.2.5), `read_symbol(cdf)` arithmetic decoding and CDF adaptation (§ 8.2.6),
and the `exit_symbol()` trailing-bit/zero-padding conformance check (§ 8.2.4).
It SHALL use generated repository-owned § 9.2 conversion tables, SHALL validate
caller-supplied CDF rows before indexing or updating them, and SHALL return
typed `splot-core` errors rather than panicking on any input. The promoted row
SHALL claim only this primitive; the § 8.2.4 CDF copy/averaging and the § 8.2.2
"Tile" CDF-array copy SHALL remain owned by `tile-cdf-save-lifecycle-boundary`,
and § 8.3 syntax-element CDF selection, default § 9.3 CDF banks,
`decode_tile()`, tile syntax traversal, reconstruction, hash output, Y4M output,
reference refresh, AVM/dav2d invocation, and any new dependency/scheduler
surface SHALL remain tracked in their own rows.

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

#### Scenario: Symbol decoding is proven across all arities and update rates

- **WHEN** the symbol decoder primitive is exercised by its test suite
- **THEN** every arity `N = 2..8` is decoded, with a maximal `SymbolValue`
  selecting symbol 0 and a zero `SymbolValue` selecting symbol `N-1`
- **AND** `read_symbol(cdf)` over random valid CDF rows of every arity always
  returns a symbol in `[0, N)`, keeps post-update entries in the valid
  probability range with a count capped at 32, and is deterministic across
  fresh decoders
- **AND** the minimum and maximum § 8.2.6 adaptation rates produce the exact
  hand-verified post-update rows
- **AND** decoding many symbols past the end of a tiny payload drives
  `SymbolMaxBits` deeply negative without panicking and with deterministic
  implicit zero padding

#### Scenario: Broader symbol and CDF work stays in its own rows

- **WHEN** a reader checks decoder support after the symbol decoder primitive is
  marked supported
- **THEN** the `symbol-decoder` row states that the generic AV2 § 8.2
  primitive is complete and proven
- **AND** § 8.3 CDF selection (`tile-cdf-selection-boundary`), the § 8.2.4 CDF
  copy/averaging and Tile/Saved CDF banks (`tile-cdf-save-lifecycle-boundary`),
  default § 9.3 banks, `decode_tile()`/traversal (`tile-payload-decode`), and
  broad reconstruction, decode hashing, Y4M/raw output, and CLI decode beyond
  the already-`supported` minimal tier (plus AVM/dav2d invocation) remain
  tracked in their own rows
