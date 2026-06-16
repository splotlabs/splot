# encoder-tools delta: operating-point-set-writer

## ADDED Requirements

### Requirement: operating point set OBU writer

`splot-core` SHALL provide a writer that serializes a parsed `operating_point_set_obu()` (§ 5.10) and
its `operating_point_payload()` sub-structs (§ 5.11, § 5.11.1–5.11.5) back to bytes — the inverse of
`parse_operating_point_set` — threading the OBU header's `obu_xlayer_id` to select the global-vs-local
branch, so the complete-OBU dispatch round-trips this OBU type instead of returning `Unimplemented`.
The writer SHALL be reject-before-write and SHALL never panic on a constructed model: it SHALL reject
a `payloads` length that disagrees with `ops_cnt`, a per-element index that disagrees with its
position, any gated `Option` whose presence disagrees with its gate, and any field value outside its
descriptor's domain.

#### Scenario: a parsed operating point set OBU round-trips

- **WHEN** a parsed `operating_point_set_obu()` (reset, global, or local form) is written by the
  dispatch and the bytes are reparsed
- **THEN** the reparsed `OperatingPointSet` SHALL equal the original, byte-exact on the canonical
  subset.

#### Scenario: a non-canonical constructed model is rejected, not panicked

- **WHEN** the writer is given an `OperatingPointSet` the parser could never produce (a payload count,
  index, gated-`Option`, or out-of-range inconsistency)
- **THEN** it SHALL return a typed `WriteError` and write no bit, never panicking.
