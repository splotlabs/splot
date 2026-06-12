# bitstream delta: frame-global-motion-params

Advances `AV2-5.18.9-GLOBAL-MOTION` (the inter arm).

## ADDED Requirements

### Requirement: inter global-motion parsing

The frame-header parser SHALL parse the § 5.18.9 inter arm —
`use_global_motion`, the base-parameter selection (including the
SWITCH_FRAME inference and the `our_ref` `ns(NumTotalRefs + 1)` read),
and the per-reference warp-parameter loop through `global_param()` and
the § 5.18.9.3-.6 subexp decode chain — on grounded inter frames,
stopping honestly where the selection consumes unmodeled cross-frame
state. An EOF inside the modeled arm SHALL preserve the already-parsed
facts and surface as truncation.

#### Scenario: inter frame parses global motion

- **WHEN** a grounded inter frame signals `use_global_motion == 1`
- **THEN** the warp parameters parse through the subexp chain

#### Scenario: unmodeled base selection stops honestly

- **WHEN** the base-parameter selection consumes cross-frame state the
  model lacks
- **THEN** the parse stops at that branch with earlier facts preserved

#### Scenario: EOF surfaces as truncation

- **WHEN** the payload ends inside the global-motion arm
- **THEN** the facts survive and the truncation diagnostic fires
