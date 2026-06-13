# bitstream delta: tile-payload-boundary-validation

Advances `AV2-5.20-TILE-GROUP-PAYLOAD` (the § 5.20.1 framing slice).

## ADDED Requirements

### Requirement: tile-payload framing validation

The validator SHALL parse the § 5.20.1 per-tile framing for tile groups
whose § 5.19 structure parsed to completion —
`tile_size_minus_1 le(TileSizeBytes)` for non-last non-bridge tiles,
the `sz` bookkeeping, and the last-tile/bridge arms — and SHALL flag
provable framing defects with their governing citations. Tile groups
with incomplete structure, ambiguous boundaries, or unparsed tile
layout SHALL produce no framing judgments; `decode_tile()` and the
block syntax stay out of scope.

#### Scenario: conformant framing parses

- **WHEN** a completed multi-tile tile group frames its tiles per
  § 5.20.1
- **THEN** the per-tile byte ranges are recorded and surfaced

#### Scenario: overflowing tile size is flagged

- **WHEN** a non-last tile's size plus its length field exceeds the
  remaining payload
- **THEN** the framing diagnostic fires with its citation

#### Scenario: incomplete structure stays silent

- **WHEN** the § 5.19 structure did not complete
- **THEN** no framing judgment is made
