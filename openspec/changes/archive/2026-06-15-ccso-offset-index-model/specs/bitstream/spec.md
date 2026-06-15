# bitstream delta: ccso-offset-index-model

## MODIFIED Requirements

### Requirement: intra-path loop-restoration and CCSO parsing

The frame-header core parser SHALL parse `lr_params()` (§ 5.18.7.11) and
`ccso_params()` (§ 5.18.7.12) on the intra path, gated on the parsed
sequence restoration/CCSO configuration, and SHALL continue into the § 5.18.2
tail (`read_tx_mode()`, § 5.18.8.1, and beyond — see the *complete intra
frame-header parsing* requirement). When an `lr_params()` plane signals a
frame-level Wiener filter, the parser SHALL stop honestly before the unmodeled
`read_wienerns_filter()` bank decode, naming the missing coverage and preserving
the pre-Wiener facts. An EOF inside the new cluster SHALL preserve the
already-parsed frame facts.

`ccso_params()` SHALL surface the per-plane `ccso_offset_idx` values (§ 5.18.7.12) in the
parsed model — a `(d0, d1, band)`-ordered list of the `tu(7)` values, length
`maxEdgeInterval * maxEdgeInterval * maxBand`, empty when the plane codes no offsets — rather
than discarding them, so a § 5.18.7.12 writer can reproduce the structure byte-exactly. The
parse SHALL otherwise be unchanged: the same bits are read in the same order and the
consumed-bit count is identical.

#### Scenario: intra frame parses lr and ccso params

- **WHEN** an intra frame header reaches the post-CDEF tail with the
  gating sequence configuration parsed
- **THEN** the loop-restoration and CCSO parameters are parsed and parsing
  continues into the § 5.18.2 tail

#### Scenario: frame-level Wiener filter stops honestly

- **WHEN** an `lr_params()` plane signals `frame_filters_on`
- **THEN** the parser stops before `read_wienerns_filter()` with a named
  missing-coverage status, and the partial `lr_params()` facts parsed before
  the stop (per-plane restoration types, `frame_filters_on`, the luma
  `NumFilterClasses`, `UsesLr`, and `LoopRestorationSize`) are surfaced on a
  dedicated partial field — distinct from the complete-parse field so a
  partial parse is never mistaken for a complete one

#### Scenario: ccso offset indices are surfaced in read order

- **WHEN** a `ccso_params()` plane has `ccso_planes == 1`
- **THEN** the `ccso_offset_idx` `tu(7)` values are surfaced on the parsed plane in
  `(d0, d1, band)` read order, with length `maxEdgeInterval * maxEdgeInterval * maxBand`
- **AND** a plane with `ccso_planes == 0` surfaces an empty list

#### Scenario: EOF preserves facts

- **WHEN** the payload ends inside `lr_params()` or `ccso_params()`
- **THEN** the already-parsed frame facts survive and the status reports
  the truncation
