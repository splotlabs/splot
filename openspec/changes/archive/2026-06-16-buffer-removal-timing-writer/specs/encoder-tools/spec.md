# encoder-tools delta: buffer-removal-timing-writer

## ADDED Requirements

### Requirement: buffer removal timing OBU writer

`splot-core` SHALL provide a writer that serializes a parsed `buffer_removal_timing_obu()` (§ 5.12)
back to bytes — the inverse of `parse_buffer_removal_timing` — for both the extended-layer
(`br_ops_dependent_flag == 0`) and the operating-point-set (`br_ops_dependent_flag == 1`) forms, so
the complete-OBU dispatch round-trips this OBU type instead of returning `Unimplemented`. The writer
SHALL be reject-before-write and SHALL never panic on a constructed model: it SHALL reject an
`op_times` length that disagrees with `br_ops_cnt`, a per-operating-point `index` that disagrees with
its position, a `br_time_op` presence that disagrees with `br_decoder_model_present_op_flag`, and any
field value outside its descriptor's domain.

#### Scenario: a parsed buffer removal timing OBU round-trips

- **WHEN** a parsed `buffer_removal_timing_obu()` (either form) is written by the dispatch and the
  bytes are reparsed
- **THEN** the reparsed `BufferRemovalTiming` SHALL equal the original, byte-exact on the canonical
  subset.

#### Scenario: a non-canonical constructed model is rejected, not panicked

- **WHEN** the writer is given a `BufferRemovalTiming` the parser could never produce (an `op_times`
  count, `index`, or gated `br_time_op` inconsistency, or an out-of-range value)
- **THEN** it SHALL return a typed `WriteError` and write no bit, never panicking.
