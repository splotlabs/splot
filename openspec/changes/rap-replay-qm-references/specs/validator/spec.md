# validator delta: rap-replay-qm-references

Advances `AV2-7.3.8-HLS-AVAILABILITY` by wiring quantizer-matrix level references into the
§ 7.3.8.1 random-access-point availability replay — the last RAP-replay residual after
film-grain.

## ADDED Requirements

### Requirement: quantizer-matrix levels participate in the random-access-point replay

The validator SHALL replay quantizer-matrix level references (`using_qmatrix == 1`, the
referenced custom `qm_*` levels) through the § 7.3.8.1 random-access-point availability
tracker (docs/spec/av2/1.0.0/07-decoding-process.md#s-7-3-8-1): a quantizer-matrix OBU send
is recorded as a `RapHlsKey::QmLevel` resend event for each level it makes available
(including every level on a `qm_bit_map == 0` reset-to-defaults), and a frame's
linearly-available referenced levels are buffered and resolved at temporal-unit completion,
firing `hls/unavailable-at-random-access-point` when no qualifying resend is visible from a
start point. The replay records the OBU send and is disjoint from the linear
`frame-header/qm-level-unavailable` check and from the quantizer matrix's reset/poison
discipline (only linearly-available, non-poisoned references are buffered). The
quantizer-matrix family is inexpressible by `ExternalHlsSet`, so any Provided external-HLS
mode suppresses the replay.

#### Scenario: level dropped at a random access point

- **WHEN** a quantizer-matrix OBU defines custom level `L` before a random access point, `L`
  survives that random access point's reset linearly and is not resent in or after it, and a
  later frame references `L` via `using_qmatrix`
- **THEN** an error diagnostic `hls/unavailable-at-random-access-point` naming the quantizer
  matrix level family (§ 7.3.8.1) is produced

#### Scenario: level resent at the random access point stays silent

- **WHEN** the referenced level is resent in or after the governing random access point's
  temporal unit (including via a `qm_bit_map == 0` reset-to-defaults)
- **THEN** no `hls/unavailable-at-random-access-point` diagnostic is produced for it

#### Scenario: replay is disjoint from the linear check

- **WHEN** a frame references a custom level no quantizer-matrix OBU has ever defined
- **THEN** only the linear `frame-header/qm-level-unavailable` fires and the replay stays
  silent

#### Scenario: external-HLS suppression

- **WHEN** validation runs under any Provided external-HLS mode
- **THEN** the quantizer-matrix random-access-point replay does not fire

## MODIFIED Requirements

(none)

## REMOVED Requirements

(none)
