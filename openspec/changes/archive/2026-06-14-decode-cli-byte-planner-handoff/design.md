# Design: Decode CLI Byte Planner Handoff

## Context

`splot decode` currently accepts decode-shaped CLI arguments but does not read the
input bitstream. It emits the static `decode/unsupported-feature` descriptor
before exercising the source-backed byte planner added under
`DECODE-BYTE-STREAM-PLANNER`.

This change is the handoff from CLI argument parsing to the plan-only decoder
library. It must preserve the staged decoder boundary: no tile payload decode,
no reconstruction, no frame hashes, no Y4M writing, no reference refresh, and no
AVM/dav2d probing or execution. PR #101's concurrency model is also a hard
constraint: decoder and reconstruction code must route through
`splot_parallel::WorkerPool`; this CLI slice must not introduce direct Rayon,
crossbeam, global pools, ad-hoc queues, or `std::thread::spawn`.

PR #113 Codex review feedback is carried forward as non-regression scope:
unsupported structures must keep precedence over later traversal limits, IVF
cursor fatal frame-header errors must remain retry-stable, `decode_plan_bytes`
fuzz seeds must include prefixed valid paths, and `DecodeContext` docs must keep
raw-byte planning accurate.

## Goals / Non-Goals

**Goals:**

- Make `splot decode` read the requested input bytes and call
  `DecodeContext::plan_bytes` with finite default `DecodeOptions`.
- Render one structured `decode/*` diagnostic for planner failures and for
  plan-success/runtime-unsupported deferral.
- Promote `decode/resource-limit` from planned documentation to emitted source
  behavior when byte planning rejects a measured value.
- Add `decode/malformed-source` for malformed Annex B, malformed IVF container,
  or malformed Annex B inside IVF.
- Keep diagnostics owned by `splot-decode`; keep serde/JSON rendering owned by
  `splot-cli`.
- Prove output paths remain untouched on malformed, unsupported, limit, and
  runtime-deferral paths.

**Non-Goals:**

- No decoded pixels, tile syntax traversal, reconstruction, hash output, Y4M
  output, reference-frame storage, or encoder roundtrip support.
- No new dependencies and no crate dependency-direction changes.
- No AVM/dav2d/ffmpeg path probing, wrapper, fixture generation, test hook, CI
  hook, or local-reference metadata entry.

## Decisions

1. The CLI reads input before considering any output artifact write.

   `DecodeArgs::output_target` remains useful for clap/argument contract checks,
   but `run` only resolves the selected path and never creates, truncates, or
   writes it. Missing input stays an operational file-read error with exit code
   `2`; it is not converted into a decode diagnostic because no source bytes were
   available to the decoder.

2. Diagnostics are adapted in `splot-decode`, not hand-assembled in `splot-cli`.

   A library-owned diagnostic report wraps the static descriptor plus typed
   detail structs for malformed source, resource limit, unsupported structure,
   and runtime deferral. The CLI converts that report to text or its private
   serde JSON view. This keeps `splot-decode` dependency-free from serde while
   preventing the CLI from inventing diagnostic rule IDs or matrix metadata.

3. `decode/unsupported-feature` has two explicit variants.

   Planner-level unsupported structures use the metadata from
   `DecodeUnsupportedStructure` (`decode-stream-state` /
   `DECODE-STREAM-STATE-PLANNER`). Runtime deferral after a successful byte plan
   uses `cli-decode-entrypoint` / `CLI-DECODE` and explains that planning
   succeeded but decode/output remains unsupported.

4. `decode/resource-limit` is a `splot` policy diagnostic, not AV2 conformance.

   The diagnostic includes the measured limit name, configured threshold, actual
   value, unit, and currently-null byte/bit offsets. The AV2 spec section is
   `7.1` for the current byte-planner emission because the limit gates decode
   input traversal before any decoded output exists; IVF-only container counters
   remain documented as repository policy.

5. The CLI uses only `DecodeContext::plan_bytes` for concurrency.

   The CLI constructs `DecodeContext::new(DecodeRuntimeConfig::new(args.threads))`
   and calls `plan_bytes`. It does not call `ctx.pool().install`, spawn threads,
   use global pools, or add concurrency dependencies.

## Risks / Trade-offs

- Diagnostic detail growth can make JSON brittle -> keep stable field names and
  add optional detail blocks rather than changing base fields per case.
- Missing input no longer emits `decode/unsupported-feature` -> tests and specs
  make the operational-vs-decoder boundary explicit.
- `decode/resource-limit` becomes emitted now -> registry, support matrix,
  generated status, and tests must move in the same PR.
- Runtime deferral can be mistaken for decode success -> wording and tests state
  that byte planning succeeded but runtime decode/output remains unsupported.

## Migration Plan

1. Add the library diagnostic adapter and stable source issue string accessor.
2. Wire CLI input reads and `DecodeContext::plan_bytes` into `splot decode`.
3. Update CLI/library tests for malformed, limit, planner unsupported, runtime
   unsupported, missing input, thread policies, and output no-touch behavior.
4. Update diagnostic docs, decoder support matrix/status, implementation matrix,
   and OpenSpec artifacts.
5. Run targeted tests and full repo gates before opening a ready PR.

## Open Questions

None. This slice is intentionally limited to byte planning and diagnostics.
