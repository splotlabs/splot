# ac0ej3-active-intra-tool-frontier Specification

## Purpose
Track the active intra/tool-use frontier for the local ac0ej3 decoder mission
without claiming decoded samples, loop-restoration output, reference refresh, or
byte equality.

## Requirements
### Requirement: ac0ej3 Active Intra Tool Frontier

The decoder SHALL track `DECODE-AC0EJ3-ACTIVE-INTRA-TOOL-FRONTIER` as a partial
runtime prerequisite for the ac0ej3 Wiener NS LR path. When selectable
transform-record derivation reaches AV2 §5.20.5.5 luma mode syntax with
`enable_mrls` enabled, the runtime SHALL consume `mrl_index` and, when
`mrl_index > 0`, `mrl_sec_index` using generated AV2 §9.3 CDF rows exposed
through the tile CDF subset. The runtime SHALL allow `mrl_index == 0` to
continue and SHALL reject active nonzero MRL with a structured
unsupported-feature diagnostic before prediction or transform semantics are
claimed.

#### Scenario: MRL zero syntax advances selectable records

- **WHEN** the local ac0ej3 mission stream reaches active Wiener NS LR
  selectable transform-record derivation
- **AND** a directional intra luma block is parsed with `enable_mrls == 1`
- **AND** the block decodes `mrl_index == 0`
- **THEN** the runtime consumes the MRL symbol in spec order
- **AND** it continues to the next transform-record or residual frontier without
  emitting the broad sequence-level intra-tool diagnostic

#### Scenario: Active MRL remains fail-closed

- **WHEN** a supported selectable transform-record path decodes `mrl_index > 0`
- **THEN** the runtime consumes required active MRL syntax for synchronization
- **AND** it returns a structured `decode/unsupported-feature` diagnostic for
  active nonzero MRL
- **AND** it does not populate fabricated `LrTxSkip`, decoded samples,
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
  branch while ac0ej3 transform tools requiring unsupported `transform_type()`,
  CCTX, or IST syntax are enabled
- **THEN** the runtime returns a structured unsupported-feature diagnostic
  before reading coefficient syntax that would skip those active branches
- **AND** it does not claim broad transform-type parsing, decoded sample output,
  loop-restoration filtering, reference refresh, AVM/dav2d byte equality, or
  successful ac0ej3 decode

#### Scenario: Parsed CCSO filter state does not block transform-record derivation

- **WHEN** the local ac0ej3 mission stream has parsed CCSO frame state
- **THEN** selectable Wiener NS LR transform-record derivation SHALL continue
  until it reaches an active tile syntax or residual frontier
- **AND** CCSO filtering/output remains outside the row's support claim until
  decoded frame samples and filter application are implemented
