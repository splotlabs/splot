## ADDED Requirements

### Requirement: Byte planner review regressions stay fixed

The byte-consuming decode stream planner SHALL preserve the diagnostics,
cursor contracts, fuzz smoke coverage, and public documentation promised by
Feature ID `DECODE-BYTE-STREAM-PLANNER`.

#### Scenario: Unsupported prefix wins over later traversal limits

- **WHEN** raw Annex B byte traversal has retained an OBU prefix that the
  existing stream planner classifies as `DecodeError::UnsupportedStructure`
- **THEN** `DecodeContext::plan_bytes` preserves that unsupported-structure
  error while continuing transactional byte traversal
- **AND** a later `max_obus` or `max_frames_to_decode` failure does not mask the
  earlier unsupported prefix

#### Scenario: Malformed suffix wins over earlier unsupported prefix

- **WHEN** raw Annex B byte traversal has retained an OBU prefix that the
  existing stream planner classifies as `DecodeError::UnsupportedStructure`
- **AND** later bytes in the same Annex B payload are malformed
- **THEN** `DecodeContext::plan_bytes` returns `DecodeError::MalformedSource`
- **AND** the malformed parser error is not masked by the earlier unsupported
  prefix

#### Scenario: Malformed IVF frame payload wins over earlier unsupported frame

- **WHEN** IVF byte traversal has retained an earlier frame payload containing
  an OBU that the existing stream planner classifies as
  `DecodeError::UnsupportedStructure`
- **AND** a later IVF frame payload contains malformed Annex B bytes
- **THEN** `DecodeContext::plan_bytes` returns `DecodeError::MalformedSource`
- **AND** the source issue records `IvfFramePayloadError` for the malformed
  later frame

#### Scenario: Parsed IVF OBU limits win before later payload errors

- **WHEN** parsed IVF planning has traversed complete earlier frame payload OBUs
  that exceed `max_obus` or `max_frames_to_decode`
- **AND** a later IVF frame payload contains malformed Annex B bytes
- **THEN** `DecodeContext::plan_stream` returns `DecodeError::Limit` for the
  first exceeded OBU traversal or frame-candidate limit
- **AND** the later payload parse error does not mask that already reached
  limit failure

#### Scenario: IVF frame-record limit stays typed after earlier unsupported frame

- **WHEN** IVF byte traversal has retained an earlier frame payload containing
  an OBU that the existing stream planner classifies as
  `DecodeError::UnsupportedStructure`
- **AND** a later complete IVF frame record exceeds `max_ivf_frame_records`
- **THEN** `DecodeContext::plan_bytes` returns `DecodeError::Limit`
- **AND** the limit source name is `DecodeLimitName::MaxIvfFrameRecords`
- **AND** the unsupported-prefix carve-out remains scoped to `max_obus` and
  `max_frames_to_decode`

#### Scenario: IVF cursor retry preserves fatal frame-header errors

- **WHEN** `IvfFrameCursor::next_frame_record()` returns a fatal IVF
  frame-header error
- **THEN** retrying the same public cursor returns the same fatal error
- **AND** the cursor does not advance to `End` or a warning state before the
  caller observes the retry

#### Scenario: Decode byte planner fuzz seeds exercise valid traversal

- **WHEN** CI seeds fuzz corpora from committed AV2 fixtures
- **THEN** the `decode_plan_bytes` corpus receives flag-prefixed variants
  because that target consumes byte zero as limit flags
- **AND** those variants preserve the original fixture bytes as the bitstream
  payload passed to `DecodeContext::plan_bytes`

#### Scenario: Decode context docs match byte planning API

- **WHEN** generated API docs describe `DecodeContext`
- **THEN** they state that the context owns byte-consuming and parsed-stream
  planning entry points
- **AND** they do not claim the context avoids raw byte traversal
- **AND** they continue to state that filesystem I/O, reconstruction, output
  writing, and external decoder invocation remain unsupported
