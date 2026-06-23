## ADDED Requirements

### Requirement: Resolve grouped-IVF inter frame units by OBU offset
For the verified minimal multi-reference runtime subset, the decoder SHALL treat
IVF frame records as non-normative container groups rather than decoded-frame
boundaries. After the leading `[OBU_TEMPORAL_DELIMITER, OBU_SEQUENCE_HEADER,
OBU_CLOSED_LOOP_KEY]` IVF record, each following planned
`OBU_REGULAR_TILE_GROUP` frame candidate SHALL be resolved by its planned OBU byte
offset within the parsed IVF payloads, not by assuming IVF record index equals
decoded frame-candidate index.

The decoder SHALL keep the existing verified subset guard: each following
`OBU_REGULAR_TILE_GROUP` candidate must be immediately preceded, in the same IVF
payload, by `OBU_TEMPORAL_DELIMITER`. This change SHALL NOT increase the
three-frame runtime cap, add support for additional AV2 tools, or relax the
required OBU order for the leading key frame.

#### Scenario: two following inter frame units share one IVF record
- **WHEN** the committed three-frame multi-reference fixture is repacked so the
  second and third AV2 frame units share one positive-sized IVF frame record while
  preserving the original Annex B OBU bytes and order
- **THEN** the decoder resolves both following `OBU_REGULAR_TILE_GROUP`
  candidates by planned OBU offset and emits the same three decoded frames as the
  original fixture

#### Scenario: a following inter candidate without an immediate temporal delimiter is rejected
- **WHEN** a following planned `OBU_REGULAR_TILE_GROUP` candidate is not
  immediately preceded by `OBU_TEMPORAL_DELIMITER` in its IVF payload
- **THEN** the decoder emits `decode/unsupported-feature` and produces no output
