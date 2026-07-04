## MODIFIED Requirements

### Requirement: Multi-frame runtime reaches precise unsupported gates

For the verified minimal multi-frame runtime subset, the decoder SHALL NOT reject
a planned stream solely because it contains more than three frame candidates.
The runtime SHALL preserve the leading key-frame shape requirement
(`[OBU_TEMPORAL_DELIMITER, OBU_SEQUENCE_HEADER, OBU_CLOSED_LOOP_KEY]`), require no
fatal container errors, permit only terminal trailing partial IVF header warnings,
and require each following `OBU_REGULAR_TILE_GROUP` candidate to be immediately
preceded by `OBU_TEMPORAL_DELIMITER` in its IVF payload.

The runtime SHALL continue to reject unsupported later frames before producing
caller-visible output using the existing precise gates, including but not limited
to non-regular-tile-group frame candidates, more than two valid references,
`NumTotalRefs > 2`, neighbour-dependent unproven single-ref contexts, unmodeled
cross-frame CDF loads, temporal MV state, unsupported tools, and unsupported
geometry. This change SHALL NOT claim bit-exact decode for streams beyond the
committed fixtures.

#### Scenario: a fourth frame reaches the reference-state gate

- **WHEN** a stream contains a key frame followed by three otherwise verified
  inter frame candidates
- **THEN** the runtime does not reject the stream at a total frame-candidate-count
  preflight
- **AND** it rejects before output at the existing precise
  `inter_too_many_valid_references` gate once a third valid reference would be
  needed

#### Scenario: local decoder mission reaches a runtime feature gate

- **WHEN** `splot decode /Users/bartosztomczyk/Documents/SplotLabs/local-decoder-mission.ivf`
  runs with default decode limits
- **THEN** it advances past the former total-frame-count preflight
- **AND** it emits the current structured `decode/unsupported-feature`
  diagnostic for the first unsupported runtime feature reached by the stream
