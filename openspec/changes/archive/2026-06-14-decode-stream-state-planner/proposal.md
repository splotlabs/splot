## Why

The decoder mission needs a source-backed handoff from parsed AV2 container
facts into `splot-decode` before pixel reconstruction, but the existing
`decode-stream-state` row is still plan-only and has no Feature ID. This change
adds the smallest planner boundary that incorporates the PR #101 concurrency
model without introducing a raw-byte decode entry point.

## What Changes

- Add Feature ID `DECODE-STREAM-STATE-PLANNER` for the
  `decode-stream-state` matrix row.
- Add a plan-only `splot-decode` API rooted in `DecodeContext` that consumes
  already parsed `splot_core::stream::ParsedBitstream` values, applies
  `DecodeOptions`, and returns an ordered stream plan.
- Add the approved `splot-decode -> splot-core` dependency edge for parser
  output handoff; do not add `splot-recon`, `splot-validate`, or new external
  dependencies.
- Keep stream planning deterministic across `ThreadCount` policies by making
  the initial planner serial and context-owned, with no direct Rayon/crossbeam
  use outside `splot-parallel`.
- Reject malformed parser output, resource-limit failures, invalid xlayer
  scope, non-base layers, multistream/layer-selection structures, and
  unsupported frame-carrying OBUs transactionally with typed library errors.
- Keep `splot decode` CLI behavior unchanged: it still emits the current
  `decode/unsupported-feature` diagnostic without reading input.
- Update decoder docs, support matrix, generated status docs, feature tracking,
  and OpenSpec artifacts. Do not add AVM/dav2d integration, wrappers, CI, or
  local-reference evidence.

## Capabilities

### New Capabilities

- None.

### Modified Capabilities

- `decoder-support`: Add the parsed stream-planner contract for
  `DECODE-STREAM-STATE-PLANNER`.

## Impact

- Affected crates: `crates/splot-decode` and its Cargo dependency list.
- Affected docs: `docs/DECODER-ROADMAP.md`,
  `docs/DECODER-SUPPORT-MATRIX.toml`, generated
  `docs/DECODER-SUPPORT-STATUS.md`, `docs/IMPLEMENTATION-MATRIX.toml`,
  generated feature/status coverage docs, and OpenSpec artifacts.
- Affected Feature ID: `DECODE-STREAM-STATE-PLANNER`.
- Validator impact: none.
- CLI diagnostics impact: no emitted diagnostic changes in this slice.
- Fuzz impact: no new fuzz target in this slice because the planner accepts
  already parsed `splot-core` structures, not raw untrusted bytes. A future
  raw-byte decode entry point remains tied to the `decode-fuzz-entrypoint` row.
- Dependencies: add only the workspace `splot-core` dependency to
  `splot-decode`.
