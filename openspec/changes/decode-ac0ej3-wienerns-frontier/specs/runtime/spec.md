## ADDED Requirements

### Requirement: Minimal runtime reports ac0ej3 Wiener NS frontier precisely

The minimal decode runtime SHALL map
`FrameHeaderParseStatus::StoppedBeforeWienerNsFilter` to a structured
`decode/unsupported-feature` diagnostic before any tile mode-info symbol decode,
decoded-frame allocation, reference retention, hash, raw, or Y4M output. The
diagnostic SHALL track `DECODE-AC0EJ3-WIENERNS-FRONTIER`, cite AV2 5.18.7.11,
and use a stable unsupported reason distinct from the generic incomplete-header
fallback. The change SHALL NOT parse `read_wienerns_filter()` or claim loop
restoration support.

#### Scenario: ac0ej3 reaches Wiener NS frontier

- **WHEN** `splot decode /Users/bartosztomczyk/Documents/SplotLabs/ac0ej3.ivf`
  runs with default decode limits
- **THEN** it rejects before output with `unsupported_reason =
  "unsupported_wienerns_filter"`
- **AND** the diagnostic tracks `DECODE-AC0EJ3-WIENERNS-FRONTIER`
- **AND** the diagnostic keeps byte offset 74

#### Scenario: other incomplete intra headers keep generic fallback

- **WHEN** an intra frame header stops at a status other than
  `FrameHeaderParseStatus::IntraHeaderComplete` or
  `FrameHeaderParseStatus::StoppedBeforeWienerNsFilter`
- **THEN** the minimal runtime keeps rejecting it with the existing generic
  incomplete-header unsupported diagnostic
