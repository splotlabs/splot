# encoder-tools delta: msdo-writer

## ADDED Requirements

### Requirement: multistream decoder operation OBU writer

`splot-core` SHALL provide a writer that serializes a parsed `multistream_decoder_operation_obu()`
(§ 5.6) back to bytes — the inverse of `parse_msdo` — so the complete-OBU dispatch round-trips this
OBU type instead of returning `Unimplemented`. The writer SHALL be reject-before-write and SHALL
never panic on a constructed model: it SHALL reject a `multistream_large_picture_idc` presence that
disagrees with `multistream_even_allocation_flag`, a `sub_stream_count` that disagrees with
`num_streams_minus_2 + 2`, a non-zero unused sub-stream slot, and any field value outside its
descriptor's domain.

#### Scenario: a parsed MSDO OBU round-trips

- **WHEN** a parsed `multistream_decoder_operation_obu()` (even- or uneven-allocation form) is written
  by the dispatch and the bytes are reparsed
- **THEN** the reparsed `MultistreamDecoderOperation` SHALL equal the original, byte-exact on the
  canonical subset.

#### Scenario: a non-canonical constructed model is rejected, not panicked

- **WHEN** the writer is given a `MultistreamDecoderOperation` the parser could never produce (a gated
  `multistream_large_picture_idc`, sub-stream count, unused-slot, or out-of-range inconsistency)
- **THEN** it SHALL return a typed `WriteError` and write no bit, never panicking.
