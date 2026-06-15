## Why

The full decoder conformance contract requires exact output identity before
runtime decode success can be claimed. The repository already has
source-backed raw decoded-frame hash and Y4M primitives, but the decoder mission
still needs one explicit contract for output variants, output ordering, hash
JSON, crop/sample serialization, and atomic file-output behavior.

## What Changes

- Add Feature ID `DOC-DECODER-OUTPUT-EQUIVALENCE-CONTRACT`.
- Formalize the two decoder output variants:
  `raw_intermediate_output` and `post_film_grain_output`.
- Keep existing `splot-dfh-sha256-v1` semantics for raw intermediate output and
  define the planned post-film-grain variant boundary without implementing film
  grain.
- Define output order, show-existing frame handling, flush behavior, visible
  crop, chroma plane dimensions, bit-depth sample serialization, and decoded
  metadata-hash verification expectations.
- Define the canonical JSON schema for future
  `splot decode --output-format hash --json` success output.
- Define raw and Y4M runtime output contracts, including atomic file semantics
  and failure no-touch guarantees.
- Update the decoder roadmap, support matrix/status, full conformance docs,
  implementation matrix, and OpenSpec decoder-support requirements.
- Keep runtime decode success, tile payload completion, reconstruction, film
  grain synthesis, raw/Y4M file emission, AVM/dav2d invocation, and external
  reference-tool integration out of scope.

## Capabilities

### New Capabilities

- None.

### Modified Capabilities

- `decoder-support`: add the decoder output equivalence contract that future
  runtime hash, raw, Y4M, film-grain, output-order, and metadata-verification
  work must satisfy before any output row can be marked supported.

## Impact

- Affected docs/status: `docs/DECODER-FULL-CONFORMANCE.md`,
  `docs/DECODER-ROADMAP.md`, `docs/DECODER-SUPPORT-MATRIX.toml`, generated
  `docs/DECODER-SUPPORT-STATUS.md`, `docs/IMPLEMENTATION-MATRIX.toml`, and
  OpenSpec `decoder-support`.
- Affected code: xtask/status generation only if needed to validate the new
  matrix row; no decoder runtime behavior is claimed in this slice.
- Diagnostics: no new emitted diagnostic is required in this docs-only slice.
  The contract requires future successful file-output support to register and
  emit `decode/output-error` for output path, temporary-file, flush, sync,
  rename, cleanup, or serialization failures.
- Dependencies: no new crate dependencies and no `splot-*` dependency graph
  changes.
- Reference boundary: no AVM/dav2d source, snippets, binaries, submodules,
  dependencies, wrappers, build probes, scripts, CI jobs, runtime process
  execution, local absolute paths, or mandatory reference-tool tests.
