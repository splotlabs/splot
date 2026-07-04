# Design: decoder-runtime-structure

## Context

The previous `decoder-runtime-deslop` change cleaned duplicated logic while
leaving the module hierarchy intact. This change removes the misleading runtime
bucket and makes module names match decoder responsibilities.

## Data model / API

No public API changes are intended. Public exports remain `DecodeContext`,
`DecodeRuntimeConfig`, diagnostics, limits, stream planning types, and hash
report types. Internal runtime handoffs move behind domain modules:

- `pipeline`: frame/sequence orchestration and decoded-frame handoff.
- `bitstream`: raw byte stream planning and tile payload extraction frontiers.
- `prediction`: intra/inter prediction selection and chroma prediction helpers.
- `residual`: decode-side residual orchestration.
- `reference`: reference-frame buffer and refresh state.
- `filters`: CDEF, CCSO, deblock, and Wiener NS LR filter/restoration ordering.
- `output`: hash, raw, and Y4M serialization.
- `support`: capability gates and pipeline-local limit helpers.
- `tile`: block context and tile-local helper state.

## Spec mapping

This is not a new normative AV2 implementation. Existing code continues to cite
the same AV2 sections for already-supported frontiers.

## Diagnostics

No new diagnostic rule IDs are introduced. Existing
`decode/unsupported-feature`, `decode/resource-limit`, `decode/malformed-source`,
and `decode/output-error` behavior must remain stable.

## Tests

The primary proof is behavior preservation through focused decoder/CLI tests,
output byte comparisons for committed fixtures, dependency-direction checks,
feature-status checks, and the full `cargo xtask ci` gate.

## Alternatives considered

- Rename only `runtime_minimal` to `pipeline`: rejected because it preserves the
  same confusing mixed ownership under a new path.
- Move decoder scheduling into `splot-recon`: rejected because `splot-recon`
  owns scheduler-free primitives and must not acquire runtime state.

## Risks

- Compatibility: generated docs and matrix references contain old paths.
- Maintenance: several files are already large; this change should avoid making
  them larger and leave deeper splits to focused follow-up work.
- Behavior: output serializers must keep using the same decoded-frame pipeline.
