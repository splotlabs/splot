## Why

The decoder output-equivalence contract already defines headerless raw output
as canonical visible sample bytes, but `splot decode` currently exposes only
hash JSON and Y4M for the supported minimal tier. Adding raw output for that
same tier completes the first minimal output artifact surface without adding
new AV2 syntax, reference-tool integration, or broad decoder claims.

## What Changes

- Add Feature ID `DECODE-MINIMAL-RAW-RUNTIME-OUTPUT`.
- Add `--output-format raw` for the existing
  `minimal-intra-8bit420-hash-v1` IVF tier.
- Serialize raw output as the existing `av2-output-samples-v1`
  `raw_intermediate_output` sample byte stream: visible Y, U, then V samples,
  no header, no metadata, no container bytes.
- Publish raw output atomically through the same no-partial-output discipline as
  runtime Y4M output.
- Add focused runtime and CLI tests for successful raw output, no-touch failure
  paths, output byte limits, output writer failures, and thread determinism.
- Add Feature ID `CONF-DECODE-RUNTIME-RAW-FUZZ` and a byte-consuming fuzz target
  for the raw runtime API.
- Update decoder roadmap, support matrix/status, feature matrix/status,
  conformance coverage docs, and OpenSpec decoder-support requirements.
- Keep broad runtime decode, raw Annex B runtime success, output ordering beyond
  the one-frame minimal tier, post-film-grain output, metadata MD5 verification,
  AVM/dav2d invocation, and external reference-tool integration out of scope.

## Capabilities

### New Capabilities

- None.

### Modified Capabilities

- `decoder-support`: add the minimal-tier raw runtime output and raw runtime
  fuzz support requirements.

## Impact

- Affected code: `crates/splot-decode` runtime output adapters,
  `crates/splot-cli` decode output selection/publication, CLI/runtime tests,
  and `fuzz/` target registration.
- Affected docs/status: `docs/DECODER-FULL-CONFORMANCE.md`,
  `docs/DECODER-ROADMAP.md`, `docs/DECODER-SUPPORT-MATRIX.toml`,
  generated `docs/DECODER-SUPPORT-STATUS.md`,
  `docs/DECODER-SPEC-COVERAGE.md`, `docs/IMPLEMENTATION-MATRIX.toml`,
  generated feature/spec coverage docs, and OpenSpec `decoder-support`.
- Diagnostics: reuse `decode/unsupported-feature`, `decode/malformed-source`,
  `decode/resource-limit`, and `decode/output-error`; no new diagnostic rule is
  required.
- Dependencies: no new crate dependencies and no `splot-*` dependency graph
  changes.
- Reference boundary: no AVM/dav2d source, snippets, binaries, submodules,
  dependencies, wrappers, build probes, scripts, CI jobs, runtime process
  execution, local absolute paths, or mandatory reference-tool tests.
