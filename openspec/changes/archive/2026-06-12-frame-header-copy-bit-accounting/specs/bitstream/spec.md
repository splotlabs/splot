# bitstream delta: frame-header-copy-bit-accounting

Advances `AV2-5.18.1-FRAME-HEADER-GENERAL` (NumFrameHeaderBits,
frame_header_copy) on completed intra first headers.

## ADDED Requirements

### Requirement: frame header copy accounting

The frame-header parser SHALL record `NumFrameHeaderBits` when a first
frame header's `frame_header_info()` parses to completion, and a
non-first tile group of the same coded frame SHALL have its
`frame_header_copy()` region parsed as exactly that many bits and
compared bit-for-bit against the first header. A first header that did
not parse to completion SHALL leave the copy region unparsed (Unknown
routing).

#### Scenario: copy region parses and matches

- **WHEN** a non-first tile group follows a completed intra first header
- **THEN** its header-copy bits are consumed and verified bit-identical

#### Scenario: copy mismatch is flagged

- **WHEN** the copy region differs from the first header's bits
- **THEN** a diagnostic with the governing citation is emitted

#### Scenario: incomplete first header keeps Unknown routing

- **WHEN** the first header's parse stopped before completion
- **THEN** the non-first tile group's copy region is left unparsed as
  today
