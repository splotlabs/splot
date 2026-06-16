# tile-partition-symbol-read-boundary Specification

## Purpose
Define the crate-private decoder boundary that performs individual AV2
partition-entry `S()` symbol reads over already-derived tile CDF selectors
without claiming full partition traversal.

## Requirements
### Requirement: Tile partition symbol read boundary

The decoder SHALL provide a crate-private tile partition symbol read boundary tracked by Feature ID `DECODE-TILE-PARTITION-SYMBOL-READ-BOUNDARY` and decoder support matrix row `tile-partition-symbol-read-boundary`. The boundary SHALL route the existing AV2 § 8.3.2 partition-entry CDF selectors for `do_split`, `do_square_split`, `rect_type`, `do_ext_partition`, and `do_uneven_4way_partition` through AV2 § 8.3.1 `S()` parsing by calling `SymbolDecoder::read_symbol(cdf)` on the selected mutable CDF row. The boundary SHALL return raw decoded `Symbol` values and SHALL NOT claim partition decisions, `read_partition()`, `decode_tile()`, `exit_symbol()`, reconstruction, output, reference refresh, public API support, or external decoder invocation.

#### Scenario: Supported selector reads one symbol

- **WHEN** a caller provides a valid supported partition-entry `TileCdfSelector`, a tile-local CDF subset, and a caller-owned `SymbolDecoder` initialized for a bounded tile payload
- **THEN** the boundary validates the selector, passes the selected row to `SymbolDecoder::read_symbol(cdf)`, returns the decoded raw `Symbol`, and advances the caller-owned symbol decoder exactly once

#### Scenario: CDF update mode is honored by the caller-owned symbol decoder

- **WHEN** the caller-owned `SymbolDecoder` is configured with CDF updates enabled
- **THEN** a successful read mutates the selected CDF row according to AV2 § 8.2.6
- **AND** all non-selected supported rows remain unchanged
- **WHEN** the caller-owned `SymbolDecoder` is configured with CDF updates disabled
- **THEN** the same successful read leaves the selected CDF row byte-for-byte unchanged

#### Scenario: Selector failures do not consume symbols

- **WHEN** a caller provides an out-of-range selector context or an invalid `PlaneStart`
- **THEN** the boundary returns the existing crate-private `TileCdfError`
- **AND** it does not call `read_symbol(cdf)`, mutate CDF rows, or advance the caller-owned symbol decoder

#### Scenario: Symbol decoder failures preserve their source

- **WHEN** `SymbolDecoder::read_symbol(cdf)` rejects the selected CDF row or tile payload state
- **THEN** the boundary returns the underlying `splot_core` symbol-decoder error without collapsing it into a selector error
- **AND** CDF rows remain unchanged when `read_symbol(cdf)` rejects the row during pre-read validation

#### Scenario: Partition traversal remains incomplete

- **WHEN** decoder support status is rendered after this boundary is implemented
- **THEN** `tile-partition-symbol-read-boundary` records support only for the five individual partition-entry `S()` symbol reads
- **AND** `tile-payload-decode` and `tile-cdf-selection-boundary` remain partial for broader tile syntax traversal, full CDF-bank coverage, `read_partition()`, `decode_tile()`, and broad `exit_symbol()` after real syntax (the generic AV2 § 8.2 `symbol-decoder` primitive itself is complete and supported)
