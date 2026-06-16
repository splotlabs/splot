# tile-cdf-save-lifecycle-boundary Specification

## Purpose
Define the crate-private supported-subset Tile-to-Saved and Saved-to-Frame CDF
lifecycle boundary used by the minimal decoder runtime, including transactional
tile completion, frame-end count scaling, and minimal output identity.

## Requirements
### Requirement: Tile CDF lifecycle boundary is transactional

The decoder SHALL provide a crate-private lifecycle boundary for Feature ID
`DECODE-TILE-CDF-SAVE-LIFECYCLE-BOUNDARY` over the currently supported tile CDF
subset. The boundary SHALL copy frame rows into tile-local rows before symbol
reads, let successful `S()` reads mutate only the tile-local rows selected by
the existing CDF selectors, and apply final tile rows to saved rows only after
the tile's `exit_symbol()` succeeds.

#### Scenario: successful tile completion applies saved rows

- **GIVEN** a supported tile work unit with a copy or average save policy
- **WHEN** the tile syntax frontier succeeds and `exit_symbol()` succeeds
- **THEN** the final tile-local CDF subset is copied or averaged into the saved
  CDF subset according to AV2 § 8.2.4
- **AND** only the supported subset rows are claimed by this boundary

#### Scenario: failure leaves saved rows unchanged

- **GIVEN** a supported tile work unit and an initial saved CDF subset
- **WHEN** a symbol mismatch, CDF/symbol parse failure, resource-limit failure,
  or `exit_symbol()` failure aborts the tile frontier
- **THEN** saved CDF rows remain byte-for-byte equal to their pre-tile state
- **AND** frame CDF rows are not promoted from the failed tile

### Requirement: Frame-end subset update scales CDF counts

The supported subset SHALL provide frame-end CDF update behavior for Feature ID
`DECODE-TILE-CDF-SAVE-LIFECYCLE-BOUNDARY` and AV2 § 7.5: saved rows are copied
into frame rows, and each row count is scaled with `(3 * count) >> 2`.

#### Scenario: frame rows receive scaled saved counts

- **GIVEN** saved CDF rows with nonzero use counts in the supported subset
- **WHEN** the subset frame-end update is applied
- **THEN** the frame CDF subset equals the saved rows except that every row's
  final count entry is `(3 * saved_count) >> 2`
- **AND** row probability and adaptation-rate entries are preserved

### Requirement: Existing minimal runtime output identity is preserved

The lifecycle boundary SHALL NOT change the existing minimal runtime hash or Y4M
output contracts.

#### Scenario: minimal fixture output remains stable

- **GIVEN** the committed `minimal-intra-8bit420-hash-v1` fixture
- **WHEN** `splot-decode` produces the hash report or Y4M bytes for the minimal
  runtime tier
- **THEN** the frame hash and Y4M bytes remain identical to the pre-change
  contract
- **AND** broader AV2 decode support remains explicitly out of scope
