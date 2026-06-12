# bitstream delta: frame-loop-restoration-ccso-params

Advances `AV2-5.18.7-SEGMENTATION-TILING` (lr_params § 5.18.7.11,
ccso_params § 5.18.7.12) and `AV2-5.18.2-FRAME-HEADER-INFO`.

## ADDED Requirements

### Requirement: intra-path loop-restoration and CCSO parsing

The frame-header core parser SHALL parse `lr_params()` (§ 5.18.7.11) and
`ccso_params()` (§ 5.18.7.12) on the intra path, gated on the parsed
sequence restoration/CCSO configuration, and SHALL advance its stop status
to the next unparsed structure of the § 5.18.2 tail. An EOF inside the new
cluster SHALL preserve the already-parsed frame facts.

#### Scenario: intra frame parses lr and ccso params

- **WHEN** an intra frame header reaches the post-CDEF tail with the
  gating sequence configuration parsed
- **THEN** the loop-restoration and CCSO parameters are parsed and the
  stop status names the next unparsed structure

#### Scenario: EOF preserves facts

- **WHEN** the payload ends inside `lr_params()` or `ccso_params()`
- **THEN** the already-parsed frame facts survive and the status reports
  the truncation
