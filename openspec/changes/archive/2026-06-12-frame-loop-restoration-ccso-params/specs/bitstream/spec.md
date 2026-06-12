# bitstream delta: frame-loop-restoration-ccso-params

Advances `AV2-5.18.7-SEGMENTATION-TILING` (lr_params § 5.18.7.11,
ccso_params § 5.18.7.12) and `AV2-5.18.2-FRAME-HEADER-INFO`. The
implementation synced the main spec directly (the stop-point requirement
had to move in the same change), so this delta MODIFIES the requirement
the change itself introduced there.

## MODIFIED Requirements

### Requirement: intra-path loop-restoration and CCSO parsing

The frame-header core parser SHALL parse `lr_params()` (§ 5.18.7.11) and
`ccso_params()` (§ 5.18.7.12) on the intra path, gated on the parsed
sequence restoration/CCSO configuration, and SHALL advance its stop status
to the next unparsed structure of the § 5.18.2 tail (`read_tx_mode()`,
§ 5.18.8.1). When an `lr_params()` plane signals a frame-level Wiener filter,
the parser SHALL stop honestly before the unmodeled `read_wienerns_filter()`
bank decode, naming the missing coverage and preserving the pre-Wiener facts.
An EOF inside the new cluster SHALL preserve the already-parsed frame facts.

#### Scenario: intra frame parses lr and ccso params

- **WHEN** an intra frame header reaches the post-CDEF tail with the
  gating sequence configuration parsed
- **THEN** the loop-restoration and CCSO parameters are parsed and the
  stop status names the next unparsed structure (`read_tx_mode()`)

#### Scenario: frame-level Wiener filter stops honestly

- **WHEN** an `lr_params()` plane signals `frame_filters_on`
- **THEN** the parser stops before `read_wienerns_filter()` with a named
  missing-coverage status, preserving the already-parsed facts

#### Scenario: EOF preserves facts

- **WHEN** the payload ends inside `lr_params()` or `ccso_params()`
- **THEN** the already-parsed frame facts survive and the status reports
  the truncation
