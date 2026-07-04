## ADDED Requirements

### Requirement: Minimal runtime advances past local decoder mission frame-level Wiener NS parser stop

The minimal decode runtime SHALL no longer reject the live local decoder mission key frame with
`unsupported_wienerns_filter` once the core parser has consumed the
AV2 5.20.10.6 frame-level luma Wiener NS bank syntax. The runtime SHALL still
reject before tile mode-info symbol decode, decoded-frame allocation, reference
retention, hash, raw, or Y4M output because loop-filter reconstruction and
10-bit output remain unsupported.

#### Scenario: local decoder mission reaches next runtime loop-filter boundary

- **WHEN** `splot decode /Users/bartosztomczyk/Documents/SplotLabs/local-decoder-mission.ivf`
  runs with default decode limits
- **THEN** it rejects before output at byte offset 74 with a structured
  `decode/unsupported-feature` diagnostic
- **AND** the diagnostic uses `unsupported_reason =
  "unsupported_wienerns_filter_bank"`
- **AND** the diagnostic tracks `DECODE-WIENERNS-BANK-FRONTIER`, which
  does not claim successful local decoder mission decode

#### Scenario: frame header parse is complete before runtime rejection

- **WHEN** the local decoder mission leading key frame is parsed during minimal runtime
  preflight
- **THEN** `FrameHeaderParseStatus::IntraHeaderComplete` is reached before the
  runtime emits its unsupported-feature diagnostic
- **AND** no tile mode-info symbol is decoded before that diagnostic
