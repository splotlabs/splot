# encoder-tools delta: multi-frame-header-writer

## ADDED Requirements

### Requirement: multi frame header OBU writer

`splot-core` SHALL provide a writer that serializes a parsed `multi_frame_header_obu()` (§ 5.7) back
to bytes — the inverse of `parse_multi_frame_header`, reusing `write_seg_info` for the embedded
`seg_info()` (§ 5.4.9) — so the complete-OBU dispatch round-trips this OBU type instead of returning
`Unimplemented`. The writer SHALL reproduce the parser-tolerated `mfh_seq_header_id` / `mfh_id_minus_1`
values verbatim so a parsed model always round-trips. It SHALL be reject-before-write and SHALL never
panic on a constructed model, rejecting the decidable inconsistencies (a `mfh_apply_deblocking_filter`
array that is non-`false` when `mfh_deblocking_filter_update` is clear, the segment-info `Option`s that
disagree with `mfh_seg_info_present_flag`, an out-of-range frame-size bit width, and out-of-range field
values).

#### Scenario: a parsed multi frame header OBU round-trips

- **WHEN** a parsed `multi_frame_header_obu()` (with or without the frame-size, deblocking-update, and
  segment-info branches) is written by the dispatch and the bytes are reparsed
- **THEN** the reparsed `MultiFrameHeader` SHALL equal the original, byte-exact on the canonical
  subset.

#### Scenario: a non-canonical constructed model is rejected, not panicked

- **WHEN** the writer is given a `MultiFrameHeader` the parser could never produce (a forced-false
  deblocking-flag, segment-info-`Option`-vs-flag, bit-width, or out-of-range inconsistency)
- **THEN** it SHALL return a typed `WriteError` and write no bit, never panicking.
