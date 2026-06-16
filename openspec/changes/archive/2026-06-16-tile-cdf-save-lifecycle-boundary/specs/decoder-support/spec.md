## ADDED Requirements

### Requirement: Decoder support matrix tracks tile CDF lifecycle boundaries

The decoder support matrix SHALL include a row for
`tile-cdf-save-lifecycle-boundary`, tracked by Feature ID
`DECODE-TILE-CDF-SAVE-LIFECYCLE-BOUNDARY`, covering the crate-private
Tile-to-Saved and Saved-to-Frame CDF lifecycle behavior for the currently
supported subset only.

#### Scenario: lifecycle row is scoped and test-backed

- **GIVEN** the generated decoder support status
- **WHEN** the `tile-cdf-save-lifecycle-boundary` row is rendered
- **THEN** it lists AV2 § 5.20.1, § 6.19.1, § 7.5, § 8.2.2, § 8.2.4,
  § 8.2.6, § 8.3.1, and § 8.3.2 as scoped references
- **AND** it records tests for copy, average, frame-end count scaling,
  transaction rollback, and minimal runtime hash/Y4M identity
- **AND** it does not mark broad § 8.3 CDF selection, full § 9.3 CDF banks,
  multi-tile scheduling, or full `decode_tile()` traversal supported
