## Context

PR #113 merged the source-backed raw byte planner after the parser duplication
fix, but the final Codex review for head `3066f4d` was not addressed before
merge. The actionable review comments are all within the byte-planner slice:
error precedence, IVF cursor state, fuzz seed coverage, and public docs.

## Goals / Non-Goals

**Goals:**

- Keep byte traversal bounded while preserving earlier unsupported-structure
  errors over later tail-limit failures.
- Ensure `IvfFrameCursor` can be retried after fatal frame-header errors and
  returns the same error.
- Ensure CI fuzz-smoke seeds exercise valid `decode_plan_bytes` traversal.
- Make `DecodeContext` type docs accurately describe `plan_bytes`.
- Keep the previous OpenSpec change archived and synced before new decoder
  work continues.

**Non-Goals:**

- No CLI input-read handoff to `DecodeContext::plan_bytes`.
- No new emitted diagnostics, hash/Y4M output, tile payload decode,
  reconstruction, reference updates, or AVM/dav2d integration.
- No direct Rayon/crossbeam use outside `splot-parallel`.

## Decisions

1. Classify each retained OBU immediately after parsing it during raw byte
   traversal.

   The parser still checks `max_obus` before parsing and retaining each OBU, so
   hostile streams cannot force unbounded retention. After a single OBU is
   retained, the byte planner classifies only that envelope through
   `stream_plan`'s single-OBU helper. If the envelope is unsupported, traversal
   records the first unsupported result and continues until EOF, a parser error,
   or a later traversal limit. Later malformed bytes, including malformed Annex
   B payloads in later IVF frames, are returned as
   `DecodeError::MalformedSource`; a later OBU or frame-candidate traversal
   limit returns the recorded unsupported result so those classification-local
   limits do not mask the earlier unsupported prefix. IVF frame-record limits
   remain typed `DecodeError::Limit` results because they bound container
   traversal before OBU classification. This keeps the parser source of truth in
   `splot-core`, keeps final clean-input classification in `stream_plan`, and
   keeps byte traversal linear in retained OBU count.

2. Do not set `IvfFrameCursor::finished` before returning fatal errors.

   Warnings and clean end states may mark the cursor finished. Fatal
   `IvfError` results must leave state unchanged so public retry behavior
   matches the documented contract.

3. Preserve existing fuzz target flag behavior but add CI-prefixed seeds.

   `decode_plan_bytes` consumes a leading flag byte to vary finite limits. CI
   should therefore seed that corpus with flag-prefixed copies in addition to
   the raw copies copied to every target.

4. Update docs without claiming runtime decode.

   `DecodeContext` owns byte-consuming planning, but it still does not inspect
   filesystem paths, reconstruct pixels, write output, or invoke external
   decoders.

## Risks / Trade-offs

- Eager single-OBU classification duplicates a pure classification call that
  `plan_stream` performs again for clean input. This is O(1) per OBU, preserves
  limit precedence, and avoids the superseded full-prefix replanning cost.
- Recorded unsupported metadata must not mask later parser errors or typed IVF
  frame-record limits. Regression coverage pins malformed Annex B suffixes and
  later malformed IVF frame payloads as `MalformedSource`, preserves earlier
  unsupported results over later OBU/frame-candidate traversal limits, and keeps
  `max_ivf_frame_records` as `DecodeError::Limit`.
- Fuzz seed changes touch CI shell/Python code, so local syntax checks and
  `cargo xtask check-fuzz-targets` are required.
