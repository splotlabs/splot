# active-intra-tool-frontier Specification

## Purpose
Track the active intra/tool-use frontier for the local decoder mission decoder mission
without claiming decoded samples, loop-restoration output, reference refresh, or
byte equality.

## Requirements
### Requirement: local decoder mission Active Intra Tool Frontier

The decoder SHALL track `DECODE-ACTIVE-INTRA-TOOL-FRONTIER` as a partial
runtime prerequisite for the local decoder mission Wiener NS LR path. When selectable
transform-record derivation reaches AV2 §5.20.5.5 luma mode syntax with
`enable_mrls` enabled, the runtime SHALL consume `mrl_index` and, when
`mrl_index > 0`, `mrl_sec_index` using generated AV2 §9.3 CDF rows exposed
through the tile CDF subset. The runtime SHALL derive and retain AV2 §5.20.5.3
`UsesMrls` state for luma/shared leaves and SHALL use neighbouring `UsesMrls`
for AV2 §8.3.2 MRL CDF context selection. Active nonzero MRL SHALL be admitted
only as syntax metadata for LR tx-skip record derivation and SHALL remain
unsupported for decoded sample prediction, loop-restoration output, and
reference state.

#### Scenario: MRL zero syntax advances selectable records

- **WHEN** the local decoder mission stream reaches active Wiener NS LR
  selectable transform-record derivation
- **AND** a directional intra luma block is parsed with `enable_mrls == 1`
- **AND** the block decodes `mrl_index == 0`
- **THEN** the runtime consumes the MRL symbol in spec order
- **AND** it records `UsesMrls == 0` for the luma/shared leaf
- **AND** it continues to the next transform-record or residual frontier without
  emitting the broad sequence-level intra-tool diagnostic

#### Scenario: Active MRL metadata is retained for LR records

- **WHEN** a supported selectable transform-record path decodes `mrl_index > 0`
- **THEN** the runtime consumes required active MRL syntax for synchronization
- **AND** it derives `UsesMrls == 1` when `mrl_sec_index == 0`
- **AND** it derives `UsesMrls == 2` when `mrl_sec_index != 0`
- **AND** it continues only through LR tx-skip record derivation that does not
  require decoded sample prediction

#### Scenario: MRL neighbour contexts use retained state

- **WHEN** a later directional luma/shared leaf reads `mrl_index` or
  `mrl_sec_index`
- **THEN** the runtime selects the CDF context from the left and above
  neighbours' retained `UsesMrls` values according to AV2 §8.3.2
- **AND** out-of-frame or not-yet-decoded neighbours contribute zero

#### Scenario: MRL prediction remains fail-closed

- **WHEN** a runtime path would need §7.13.2 MRL edge preparation or prediction
  to populate decoded samples
- **THEN** it returns a structured `decode/unsupported-feature` diagnostic
- **AND** it does not populate fabricated decoded samples, `LrTxSkip`,
  loop-restoration output, or reference state

#### Scenario: MRL CDF rows are lifecycle-managed

- **WHEN** tile CDF defaults are created, selected, copied, averaged, or
  frame-end scaled
- **THEN** the MRL index and secondary-index CDF rows use the generated AV2 §9.3
  defaults
- **AND** they use the same checked selector and transactional lifecycle paths
  as other supported tile CDF rows

#### Scenario: Nonzero transform-tool residuals stay unsupported

- **WHEN** selectable transform-record derivation reaches a nonzero residual
  branch while local decoder mission transform tools requiring unsupported `transform_type()`,
  CCTX, or IST syntax are enabled
- **THEN** the runtime returns a structured unsupported-feature diagnostic
  before reading coefficient syntax that would skip those active branches
- **AND** it does not claim broad transform-type parsing, decoded sample output,
  loop-restoration filtering, reference refresh, AVM/dav2d byte equality, or
  successful local decoder mission decode

#### Scenario: Parsed CCSO filter state does not block transform-record derivation

- **WHEN** the local decoder mission stream has parsed CCSO frame state
- **THEN** selectable Wiener NS LR transform-record derivation SHALL continue
  until it reaches an active tile syntax or residual frontier
- **AND** CCSO filtering/output remains outside the row's support claim until
  decoded frame samples and filter application are implemented
