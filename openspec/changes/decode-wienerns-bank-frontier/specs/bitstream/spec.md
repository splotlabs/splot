## MODIFIED Requirements

### Requirement: intra-path loop-restoration and CCSO parsing

The frame-header core parser SHALL parse `lr_params()` (§ 5.18.7.11) and
`ccso_params()` (§ 5.18.7.12) on the intra path, gated on the parsed sequence
restoration/CCSO configuration, and SHALL continue into the § 5.18.2 tail
(`read_tx_mode()`, § 5.18.8.1, and beyond — see the *complete intra frame-header
parsing* requirement). When an `lr_params()` plane signals a fixed-coded
frame-level Wiener NS filter (`read_wienerns_filter(plane, 0, 0, 1)`,
§ 5.20.10.6), the parser SHALL consume the frame-filter bank and preserve it on
the completed `LrParams` model. Reserved unsupported Wiener branches MAY still
use a partial `lr_params()` stop, but the fixed-coded frame-level bank SHALL NOT
be reported as an unmodeled parser stop. An EOF inside the cluster SHALL preserve
the already-parsed frame facts.

`ccso_params()` SHALL surface the per-plane `ccso_offset_idx` values
(§ 5.18.7.12) in the parsed model — a `(d0, d1, band)`-ordered list of the
`tu(7)` values, length `maxEdgeInterval * maxEdgeInterval * maxBand`, empty when
the plane codes no offsets — rather than discarding them, so a § 5.18.7.12 writer
can reproduce the structure byte-exactly. The parse SHALL otherwise be unchanged:
the same bits are read in the same order and the consumed-bit count is identical.

#### Scenario: intra frame parses fixed-coded Wiener NS bank

- **WHEN** an intra frame header reaches `lr_params()` with a plane signalling
  `frame_filters_on == 1` on the fixed-coded `readFrameFilters == 1` path
- **THEN** the frame-level Wiener NS bank is parsed and stored on that plane's
  `frame_filter_bank`
- **AND** the frame header can continue to `IntraHeaderComplete` instead of
  `StoppedBeforeWienerNsFilter`

### Requirement: Frame-header parse coverage reporting stays honest

The validator and inspector SHALL report each frame-header parse status distinctly
— the complete intra-path terminal (after the § 5.18.2 tail), the complete
show-existing-frame terminal (after its `film_grain_config()`), any reserved
unsupported Wiener branch stop, the truncation status for a payload that ends
inside the loop-filter / loop-restoration / CCSO cluster, and the truncation
status for a payload that ends inside the § 5.18.2 tail — and SHALL NOT claim
full § 5.18 frame-header conformance (including trailing-bits conformance of the
carrying OBU) from any of these statuses, since the frame header is followed by
the rest of `tile_group_obu()` (§ 5.19). A frame header truncated inside the
cluster or tail SHALL still expose its already-parsed control-region facts to the
state-supported diagnostics (the truncation SHALL NOT silence earlier frame-size
/ output-class checks). Existing frame-header activation and HLS reference
diagnostics SHALL be preserved unchanged.

#### Scenario: fixed-coded bank is complete parse data

- **WHEN** the fixed-coded frame-level Wiener NS bank parses successfully
- **THEN** inspector JSON reports it under complete `lr` data
- **AND** `lr_partial` is not used for that bank
