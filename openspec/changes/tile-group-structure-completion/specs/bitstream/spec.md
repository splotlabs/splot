# bitstream delta: tile-group-structure-completion

Advances `AV2-5.19-TILE-GROUP` (the post-frame-header remainder) on
intra-complete paths.

## ADDED Requirements

### Requirement: tile-group structure parsing

The tile-group parser SHALL parse the § 5.19 remainder after the frame
header on intra-complete paths — `tile_start_and_end_present_flag`
(gated on `NumTiles > 1` from the parsed tile layout),
`tg_start`/`tg_end` at `TileColsLog2 + TileRowsLog2` bits,
`byte_alignment()`, and the `headerBytes` payload-boundary handoff — and
SHALL validate the locally-decidable tile-group range semantics with
their governing citations. Frames whose `use_bru`/`bru_inactive` cannot
be derived SHALL stop honestly before the BRU arms; the § 5.20 payload
itself stays unparsed.

#### Scenario: intra tile group parses its structure

- **WHEN** an intra-complete first tile group's frame header is followed
  by the § 5.19 remainder
- **THEN** the tg range and payload boundary are parsed and surfaced

#### Scenario: tg range violation is flagged

- **WHEN** the parsed tg range violates a governing § 6 clause
- **THEN** a diagnostic with that citation is emitted

#### Scenario: EOF preserves facts

- **WHEN** the payload ends inside the new region
- **THEN** the already-parsed facts survive and the truncation surfaces
  per the established pattern
