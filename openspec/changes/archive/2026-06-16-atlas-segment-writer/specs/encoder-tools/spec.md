# encoder-tools delta: atlas-segment-writer

## ADDED Requirements

### Requirement: atlas segment OBU writer

`splot-core` SHALL provide a writer that serializes a parsed `atlas_segment_info_obu()` (§ 5.9) and
its § 5.9.1–5.9.5 sub-structures back to bytes — the inverse of `parse_atlas_segment` — so the
complete-OBU dispatch round-trips this OBU type instead of returning `Unimplemented`. The writer SHALL
reproduce the § 6.9.2 descriptive segment-id assignment values verbatim (they carry no
bitstream-conformance requirement), so a parsed model always round-trips. It SHALL be
reject-before-write and SHALL never panic on a constructed model, rejecting the decidable
inconsistencies (a `mode_info` variant that disagrees with the `mode`, a `num_segments` that disagrees
with the value re-derived from the mode, gated-`Option` and count-vs-length mismatches, and
out-of-range field values).

#### Scenario: a parsed atlas segment OBU round-trips

- **WHEN** a parsed `atlas_segment_info_obu()` of any mode (single / enhanced / basic / multistream /
  multistream-with-alpha) is written by the dispatch and the bytes are reparsed
- **THEN** the reparsed `AtlasSegment` SHALL equal the original, byte-exact on the canonical subset.

#### Scenario: a non-canonical constructed model is rejected, not panicked

- **WHEN** the writer is given an `AtlasSegment` the parser could never produce (a mode / mode_info,
  derived-num_segments, gated-`Option`, count, or out-of-range inconsistency)
- **THEN** it SHALL return a typed `WriteError` and write no bit, never panicking.
