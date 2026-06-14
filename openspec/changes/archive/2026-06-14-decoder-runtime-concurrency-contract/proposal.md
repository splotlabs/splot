## Why

PR #101 added the repository concurrency runtime policy, but the decoder mission
docs still describe future decode/reconstruction stages without making that
policy a decoder design constraint. The decoder and reconstruction plan must
incorporate `INFRA-PARALLEL-RUNTIME-POLICY` before byte-consuming decode or
pixel reconstruction work starts, so future performance work uses the approved
local worker-pool and bounded-queue model from the beginning.

## What Changes

- Add a decoder/reconstruction runtime concurrency contract to the
  `decoder-support` spec.
- Update the decoder roadmap and decoder support matrix so future decode,
  reconstruction, frame-hash, Y4M, and reference-state work must use the
  `splot_parallel` model.
- Record that `splot-decode` owns decode orchestration through one
  `DecodeContext` `WorkerPool`, while `splot-recon` remains pool-agnostic
  reconstruction/data-structure infrastructure.
- Require deterministic observable decode output across `--threads 1`, `auto`,
  and fixed positive thread counts before future runtime decode/hash/Y4M rows can
  be marked supported.
- Regenerate the generated decoder support status document.
- Do not add decoder algorithms, new dependencies, external reference-tool
  integration, or CI use of AVM/dav2d.

## Capabilities

### New Capabilities

- None.

### Modified Capabilities

- `decoder-support`: Add decoder/reconstruction requirements that bind future
  runtime decode work to `INFRA-PARALLEL-RUNTIME-POLICY`.

## Impact

- Affected docs: `docs/DECODER-ROADMAP.md`,
  `docs/DECODER-SUPPORT-MATRIX.toml`, generated
  `docs/DECODER-SUPPORT-STATUS.md`, `docs/IMPLEMENTATION-MATRIX.toml`,
  generated `docs/FEATURE-STATUS.md`, generated `docs/SPEC-COVERAGE.md`, and
  OpenSpec artifacts.
- Affected Feature IDs: `INFRA-PARALLEL-RUNTIME-POLICY` and stale-note cleanup
  for `INFRA-DECODER-CRATE-SCAFFOLDING`.
- Affected source behavior: none. `splot decode` remains intentionally
  unsupported and no byte-consuming decode path is added.
- Dependencies: no new dependencies. `rayon` and `crossbeam-channel` remain
  isolated behind `splot-parallel` per PR #101.
- Validator and diagnostics impact: no validator behavior change and no new
  emitted decoder diagnostic. Planned unsupported/resource-limit behavior remains
  documented through existing matrix rows.
