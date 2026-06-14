## Why

Codex review `4492663492` on PR #113 found four follow-up issues in the
merged byte-consuming decode planner. Addressing them immediately keeps
`DECODE-BYTE-STREAM-PLANNER` aligned with its OpenSpec contract before the
next CLI-facing decoder slice builds on it.

## What Changes

- Preserve unsupported-structure precedence for byte streams whose prefix is
  unsupported even when later bytes would exceed a traversal limit.
- Make `IvfFrameCursor::next_frame_record()` honor its public contract that
  cursor state is unchanged on fatal frame-header errors.
- Keep decode fuzz smoke valid-path seeds intact for `decode_plan_bytes`.
- Update stale `DecodeContext` docs now that `plan_bytes` accepts raw bytes.
- Archive the completed `decode-byte-stream-planner` OpenSpec change and sync
  its delta into the main decoder-support spec.

## Capabilities

### New Capabilities

- None.

### Modified Capabilities

- `decoder-support`: tighten the byte-consuming planner acceptance criteria for
  error precedence, cursor retry behavior, fuzz seed coverage, and public docs.

## Impact

- Feature ID: `DECODE-BYTE-STREAM-PLANNER`.
- Affected code: `crates/splot-decode/src/byte_stream.rs`,
  `crates/splot-core/src/ivf.rs`, `crates/splot-decode/src/context.rs`,
  `fuzz/fuzz_targets/decode_plan_bytes.rs`, and CI fuzz corpus seeding.
- Affected docs/specs: OpenSpec decoder-support spec and the archived
  `decode-byte-stream-planner` change.
- No new dependencies, no crate-graph changes, no AVM/dav2d source or runner
  integration, no decode output, no CLI byte-planner handoff.
