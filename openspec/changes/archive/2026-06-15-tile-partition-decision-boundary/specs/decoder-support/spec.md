## ADDED Requirements

### Requirement: Decoder support tracks tile partition decision boundary

The decoder support model SHALL track `DECODE-TILE-PARTITION-DECISION-BOUNDARY` as a distinct crate-private row named `tile-partition-decision-boundary`. The row SHALL mark only the AV2 §5.20.3.2 partition decision boundary over caller-provided facts as supported, SHALL link it to the existing `tile-partition-symbol-read-boundary` and `tile-cdf-selection-boundary` rows, and SHALL keep broader `tile-payload-decode`, `tile-cdf-selection-boundary`, `symbol-decoder`, and traversal/output rows honest when they remain partial.

#### Scenario: Support matrix records narrow partition decision support
- **WHEN** `cargo xtask check-decoder-support` renders decoder support status
- **THEN** `tile-partition-decision-boundary` appears as its own row with Feature ID `DECODE-TILE-PARTITION-DECISION-BOUNDARY`
- **AND** its notes state that support is limited to one partition decision from caller-provided allowed/implied facts
- **AND** it does not claim allowed-partition derivation, recursive `read_partition()`, `decode_tile()`, reconstruction, output, reference refresh, or external decoder use

#### Scenario: Existing broader rows remain honest
- **WHEN** decoder support status is rendered after the decision boundary lands
- **THEN** `tile-payload-decode` and `tile-cdf-selection-boundary` remain partial until their broader residual work is implemented
- **AND** `tile-partition-symbol-read-boundary` remains limited to individual `S()` reads
- **AND** the new row is cited from those notes as the separate decision consumer rather than broadening their claims
