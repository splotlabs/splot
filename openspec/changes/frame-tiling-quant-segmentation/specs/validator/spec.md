# Delta spec: validator — frame tiling/quantization/segmentation diagnostics

Mirror citations: `docs/spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-17-6`
and `#s-6-17-7`.

## ADDED Requirements

### Requirement: Frame tile-info conformance diagnostics
The validator SHALL emit structured error diagnostics, with stable `rule_id`,
`severity`, `spec_section`, and byte offsets, for the locally-decidable
§ 6.17.7.2 tile-info constraints on parsed frame headers: `TileCols` greater
than `MAX_TILE_COLS`, `TileRows` greater than `MAX_TILE_ROWS`, and
`context_update_tile_id` not less than `TileCols * TileRows`. Each new rule id
SHALL be registered in `docs/VALIDATOR-DIAGNOSTICS.md`.

#### Scenario: Out-of-range context update tile id
- **WHEN** a frame header parses a `context_update_tile_id` greater than or
  equal to `TileCols * TileRows`
- **THEN** the validator reports an error diagnostic citing § 6.17.7.2 at the
  frame-header OBU offset

#### Scenario: Conforming tile layout is silent
- **WHEN** a frame header parses a tile layout within the MAX_TILE_COLS and
  MAX_TILE_ROWS bounds and a valid `context_update_tile_id`
- **THEN** no tile-info diagnostics are emitted

### Requirement: Frame QM reference diagnostics
For parsed `setup_qm_params()` levels that reference custom quantizer matrices
(`qm_y`/`qm_u`/`qm_v` less than `NUM_CUSTOM_QMS`), the validator SHALL check
the locally-decidable § 6.17.6.2 constraints against its existing quantizer
matrix availability state: the referenced custom QM slot's `QmNumPlanes` SHALL
equal the active sequence's `NumPlanes`, and layer-dependency constraints SHALL
only be checked when the required dependency maps are available, never guessed.
Violations SHALL be error diagnostics citing § 6.17.6.2; unavailable state
SHALL NOT produce false positives.

#### Scenario: Custom QM plane-count mismatch
- **WHEN** a frame header references a custom QM whose recorded plane count
  differs from the active sequence's `NumPlanes`
- **THEN** the validator reports an error diagnostic citing § 6.17.6.2

#### Scenario: Missing QM state stays silent
- **WHEN** a frame header references a custom QM slot for which no quantizer
  matrix OBU state is available
- **THEN** the validator does not emit a § 6.17.6.2 plane-count diagnostic for
  that reference (the existing QM availability diagnostics own that case)

### Requirement: Frame-header parse coverage reporting stays honest
The validator and inspector SHALL report the new stopped-before-deblocking
parse status distinctly, and SHALL NOT claim full § 5.18 frame-header
conformance for frame headers parsed only through the new stop point. Existing
frame-header activation and HLS reference diagnostics SHALL be preserved
unchanged.

#### Scenario: Inspector surfaces new fields and status
- **WHEN** `splot inspect` runs on a stream whose frame header parses through
  quantization/segmentation/tiling
- **THEN** the JSON frame-header summary includes the parsed quantizer, QM,
  segmentation, and tile-layout fields plus the new stop-point status label

#### Scenario: Existing diagnostics regression-safe
- **WHEN** the existing validator test suite runs after this change
- **THEN** all previously emitted diagnostics (rule ids, severities, spec
  sections) are unchanged
