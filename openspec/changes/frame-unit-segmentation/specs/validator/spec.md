# validator delta: frame-unit-segmentation

Advances the § 7.3.3–§ 7.3.5 rows from todo and lands the segmentation
consumers.

## ADDED Requirements

### Requirement: coded-frame-unit segmentation and presence order

The validator SHALL segment each (obu_xlayer_id, obu_mlayer_id,
obu_tlayer_id) triple's consecutive OBUs into coded frame units and enforce
the § 7.3.3/§ 7.3.4 presence order — content interpretation (zero or one),
multi-frame headers, the pre-frame region (buffer-removal timing with the
zero-or-one bound in non-output units, quantization matrices, film grain,
prefix metadata), a single coded frame (same-type tile OBUs with
`is_first_tile_group` 1-then-0, or exactly one SEF), and the suffix-metadata
tail — with OBU_PADDING position-free and any unit containing an OBU whose
classification is undecidable treated as Unknown (no diagnostics).

#### Scenario: prefix metadata after the coded frame

- **WHEN** a non-suffix metadata OBU follows the coded frame in its unit
- **THEN** a `frame-unit/` presence-order error citing § 7.3.3 is emitted

#### Scenario: second BRT in a non-output unit

- **WHEN** a coded non-output frame unit carries two buffer-removal-timing
  OBUs
- **THEN** an error citing § 7.3.4 is emitted (an output unit with two is
  conforming)

#### Scenario: first-tile-group flag

- **WHEN** the first tile OBU of a coded frame has `is_first_tile_group = 0`
  or a later one has `is_first_tile_group = 1`
- **THEN** an error citing § 7.3.3/§ 7.3.4 is emitted

#### Scenario: undecidable unit stays silent

- **WHEN** a unit contains a frame OBU whose output classification is
  unavailable (unsupported parse path)
- **THEN** no segmentation diagnostic is emitted for that unit

### Requirement: content interpretation in the first coded frame unit

The validator SHALL enforce § 7.3.8.10: a content-interpretation OBU may
appear only in its layer's first coded frame unit of the temporal unit, and
the § 6.16.5/§ 6.16.6 first-coded-picture indication halves follow the same
segmentation.

#### Scenario: CI in a later frame unit

- **WHEN** a CI OBU appears in the second coded frame unit of its layer's
  temporal unit
- **THEN** an error citing § 7.3.8.10 is emitted

## MODIFIED Requirements

(none)

## REMOVED Requirements

(none)
