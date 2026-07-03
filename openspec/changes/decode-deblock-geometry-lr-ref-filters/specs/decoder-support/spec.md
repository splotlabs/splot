## MODIFIED Requirements

### Requirement: Inter-frame filter chain geometry and LR reference filters
The decoder SHALL expose every executed luma transform unit to the
§ 7.17.2 deblock edge walk with the § 7.17 coding-block origin carried
separately from the transform origin, SHALL keep skipped inter blocks'
internal transform-tile seams from qualifying as coding-block edges, and
SHALL resolve § 5.18 loop-restoration filter-match indices against the
reference frames' retained frame-level Wiener-NS taps in
`search_frame_filters` order, offsetting the PC-Wiener translation by
both the frame-class and reference-filter group sizes.

#### Scenario: Mission-stream post-filter luma matches AVM
- **GIVEN** the ac0ej3 mission stream's first three coded frames
- **WHEN** each frame runs the full § 7.2 filter chain
- **THEN** every post-filter luma sample is byte-identical to the AVM
  oracle, including the frame that selects reference-frame filter
  matches in its frame-level Wiener-NS bank
