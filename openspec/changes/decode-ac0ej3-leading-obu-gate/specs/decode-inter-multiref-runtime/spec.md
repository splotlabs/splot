## MODIFIED Requirements

### Requirement: Multi-frame runtime reaches precise unsupported gates

For the verified minimal multi-frame runtime subset, the decoder SHALL NOT reject
a planned stream solely because it contains more than three frame candidates.
The runtime SHALL require the leading key-frame payload to start with
`[OBU_TEMPORAL_DELIMITER, OBU_SEQUENCE_HEADER, OBU_CLOSED_LOOP_KEY]`, parse and
validate that sequence header, require no fatal container errors, permit only
terminal trailing partial IVF header warnings, and reject any additional OBU
after the leading key OBU for otherwise supported sequences before
caller-visible output.

The runtime SHALL continue to reject unsupported later frames before producing
caller-visible output using the existing precise gates, including but not limited
to extra OBUs in the leading key IVF payload, non-regular-tile-group frame
candidates, more than two valid references, `NumTotalRefs > 2`,
neighbour-dependent unproven single-ref contexts, unmodeled cross-frame CDF
loads, temporal MV state, unsupported tools, and unsupported geometry. This
change SHALL NOT claim bit-exact decode for streams beyond the committed
fixtures.

#### Scenario: ac0ej3 reaches the sequence bit-depth gate

- **WHEN** `splot decode /Users/bartosztomczyk/Documents/SplotLabs/ac0ej3.ivf`
  runs with default decode limits
- **THEN** it advances past the former leading
  `[TD, SEQ, CLK, OBU_REGULAR_TILE_GROUP]` shape gate
- **AND** it emits the structured `decode/unsupported-feature` diagnostic with
  `unsupported_reason = "unsupported_bit_depth"`

#### Scenario: extra leading-payload OBU still fails closed

- **WHEN** an otherwise supported stream carries an additional OBU after the
  leading `[TD, SEQ, CLK]` key payload
- **THEN** the runtime rejects it before caller-visible output with
  `unsupported_reason = "unexpected_leading_obu_after_key"`
