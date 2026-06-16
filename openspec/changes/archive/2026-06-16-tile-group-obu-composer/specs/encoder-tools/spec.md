# encoder-tools delta: tile-group-obu-composer

## ADDED Requirements

### Requirement: tile-group OBU composer (first tile group)

`splot-core` SHALL provide a composing writer that emits a whole **first** intra `tile_group_obu()`
payload (`is_first_tile_group == 1`): `is_first_tile_group` `f(1) = 1` (with the inferred
`frame_header_present_flag == 1`), the embedded `frame_header()` via the existing frame-header-core
writer, the § 5.19 structure writer, and the § 5.20.1 payload framing writer, in § 5.19 read order.
The composer SHALL draft the whole payload into a scratch writer and commit it only on full success,
so any delegated sub-writer reject leaves the caller's writer untouched (reject-before-write for the
whole composition). For every model the composer accepts, reparsing the emitted payload stage by
stage (`parse_tile_group_prefix`, the frame-header core, `parse_tile_group_structure`,
`parse_tile_group_framing`) SHALL round-trip each stage's syntax fields. The composer SHALL be
additive (no model or parser-error change) and SHALL never panic.

#### Scenario: a first-tile-group OBU payload round-trips

- **WHEN** a valid first-tile-group model (frame-header core + views, § 5.19 structure, § 5.20.1
  framing + tile data) is composed and the emitted payload is reparsed stage by stage
- **THEN** each stage's syntax fields SHALL round-trip and `is_first_tile_group` SHALL reparse as `1`.

#### Scenario: an out-of-scope or non-reproducible composition is rejected before any bit

- **WHEN** the non-first (`frame_header_copy()`) continuation form is requested, or any delegated
  sub-writer rejects its model
- **THEN** the composer SHALL return a typed `WriteError` and write no bit.
