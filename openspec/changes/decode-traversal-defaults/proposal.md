## Why

The current `splot decode /Users/bartosztomczyk/Documents/SplotLabs/local-decoder-mission.ivf`
gate is the repository-owned default resource policy, not an AV2 syntax or
runtime-decode gate: the byte planner stops at `max_frames_to_decode = 128`
before it can expose the next unsupported decoder capability. The target is a
9.7 MB AV02 IVF stream that the inspector reports as 12964 OBUs, still below
the default input-byte policy.

## What Changes

- Raise the finite default OBU and frame-count policy ceilings to mission-scale
  values while keeping them bounded and explicitly pinned by tests.
- Admit `OBU_REGULAR_TIP` as a planner frame candidate so the real target
  reaches the runtime's honest unsupported-feature gate instead of stopping in
  byte traversal. This does not add TIP reconstruction or output support.
- Preserve explicit low-limit behavior and `decode/resource-limit` diagnostics
  for callers, tests, and fuzz targets that set smaller `DecodeLimits`.
- Document that this is local resource policy only. It does not decode new AV2
  syntax, reconstruct pixels, change the verified subset, or claim `local decoder mission`
  conformance.

## Impact

- Affects `crates/splot-decode/src/limits.rs`, `byte_stream.rs`,
  `stream_plan.rs`, and their focused tests.
- Updates decoder support / implementation tracking for
  `DECODE-LIMITS-RUNTIME-API` and `DOC-DECODE-LIMITS-CONTRACT`.
- Expected next `local decoder mission` gate after this change is a structured runtime
  unsupported-feature diagnostic, not decode success.
