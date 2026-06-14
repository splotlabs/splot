## Why

The current decoder stream planner only accepts already parsed `splot-core`
structures, so there is still no bounded byte-consuming decode-side entry point
for untrusted AV2 inputs. The decoder mission needs that handoff before tile
payload decode, reconstruction, hashes, or CLI success can be claimed.

## What Changes

- Add a no-new-dependency byte-stream planner API in `splot-decode` that accepts
  raw Annex B or IVF/DKIF bytes and returns the existing deterministic
  `DecodeStreamPlan`.
- Enforce relevant `DecodeLimits` during byte traversal before retaining the
  next OBU or IVF frame record, avoiding the current unbounded vector-producing
  `splot-core::stream::parse_bitstream_partial` path for this new entry point.
- Preserve the existing base-layer-only planner semantics, typed malformed
  source errors, typed unsupported-structure errors, and context-owned
  `splot_parallel::WorkerPool` runtime boundary.
- Add a self-contained fuzz target for the first byte-consuming decode planner
  surface with finite safe limits.
- Update decoder support docs, matrix/status output, implementation matrix, and
  OpenSpec state for Feature ID `DECODE-BYTE-STREAM-PLANNER`.
- Keep `splot decode` CLI behavior unsupported in this slice; no reconstruction,
  frame hash digest, Y4M output, AVM/dav2d invocation, or external decoder
  wrapper is added.

## Capabilities

### New Capabilities

- None.

### Modified Capabilities

- `decoder-support`: adds the source-backed byte-consuming stream planning
  requirement and fuzz coverage for the initial decode entrypoint handoff.

## Impact

- Affected code: `crates/splot-decode`, `fuzz`, `docs/DECODER-ROADMAP.md`,
  `docs/DECODER-SUPPORT-MATRIX.toml`, generated decoder support docs, and
  `docs/IMPLEMENTATION-MATRIX.toml`.
- Public API: `DecodeContext` gains a byte-stream planning method that returns
  `DecodeStreamPlan` and uses existing `DecodeOptions`, `DecodeLimits`, and
  `DecodeError`.
- Dependencies: no new third-party dependency and no new production/workspace
  crate dependency edge; the fuzz harness gains local path dependencies on
  `splot-decode` and `splot-parallel` for the byte-planner fuzz target.
- Diagnostics: existing library-level `decode/unsupported-feature` metadata is
  reused for unsupported planner structures; CLI emission remains unchanged.
