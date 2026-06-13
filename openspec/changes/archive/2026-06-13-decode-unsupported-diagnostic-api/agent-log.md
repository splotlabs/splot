# Agent Log: decode-unsupported-diagnostic-api

## Orchestrator Plan

Objective: move the existing `decode/unsupported-feature` diagnostic descriptor
from CLI-local constants into `splot-decode`, wire the CLI to render it
unchanged, and update dependency-direction/docs/matrix state.

Reason for selecting this slice: `splot-decode` now exists, but it owns no
decoder-domain API. Owning the existing unsupported diagnostic is the smallest
library-backed decoder step that preserves the current no-runtime-decode
contract.

Feature ID: `DECODE-UNSUPPORTED-DIAGNOSTIC-API`.

Baseline: PR #94 (`feat(decoder): scaffold decode and recon crates`) merged as
`f0cb914`; this branch started from `origin/main` at that commit.

## Planning Agents

### @architect / Meitner the 2nd

- Agent ID: `019ec1ec-d6af-7261-a53c-ac60b7c7dfc8`
- Output: recommended `decode-unsupported-diagnostic-api`, moving diagnostic
  data into `splot-decode` while preserving CLI behavior. Add
  `splot-cli -> splot-decode`; do not add `splot-decode -> splot-core` or
  `splot-decode -> splot-recon`. Keep runtime decode, reconstruction, hashes,
  Y4M, AVM/dav2d, and reference evidence out of scope.

### @spec-reader / Linnaeus the 2nd

- Agent ID: `019ec1ec-eb0e-7a83-88db-8df9f6104280`
- Output: keep `decode/unsupported-feature` tied to AV2 §7.1 as diagnostic
  context only. Do not cite §7.2, §7.21, §7.23, Annex A, or §5.2 as
  implementation evidence. Preserve diagnostic field values exactly and keep
  `CLI-DECODE` as the user-visible diagnostic Feature ID.

### @api-designer / Huygens the 2nd

- Agent ID: `019ec1ec-ffe9-7162-88b7-a3071c7957c6`
- Output: keep `splot-decode` dependency-free and expose only
  `DecodeDiagnostic`, `DecodeSeverity`, `UNSUPPORTED_FEATURE_DIAGNOSTIC`, and
  `unsupported_feature_diagnostic()`. Keep JSON serialization and clap/output
  selection in the CLI. Add `splot-decode` unit tests for exact field values.

### @reference-oracle / Boyle the 2nd

- Agent ID: `019ec1ed-1209-71c0-836d-33850cfe5180`
- Output: no AVM/dav2d evidence is needed. This slice changes diagnostic
  ownership only and does not decode input, parse more syntax, reconstruct
  pixels, manage references, compute hashes, emit Y4M, or compare outputs.

### @avm-reader-runner / Nietzsche the 2nd

- Agent ID: `019ec1ed-23f9-7900-9778-594e51876a3e`
- Output: no local AVM read/run is needed because the slice has no decoder
  semantics. Evidence should stay limited to diagnostic behavior, JSON/text
  rendering, exit code, and no input/output side effects.

### @dav2d-reader-runner / Ohm the 2nd

- Agent ID: `019ec1ed-4d5e-7c50-ada1-b4b8f796ffd5`
- Status: closed after no actionable output. This no-reference slice did not
  need dav2d source reads or runs; the no-external-reference boundary above
  remains the applicable constraint.

## Local Reference Boundary

No AVM, dav2d, ffmpeg, network command, local reference source read, or
reference-output fixture is needed. This change must not add external decoder
source, binaries, wrappers, bindings, submodules, scripts, CI jobs, `xtask`
runners, build/runtime probes, tests requiring external decoders, local paths,
copied reference code/prose, or claims of runtime decode/conformance/parity.

## Implementation Notes

- Added a dependency-free public diagnostic descriptor API in
  `crates/splot-decode/src/lib.rs`:
  `DecodeDiagnostic`, `DecodeSeverity`, `UNSUPPORTED_FEATURE_DIAGNOSTIC`, and
  `unsupported_feature_diagnostic()`.
- Made `DecodeSeverity` and `DecodeDiagnostic` `#[non_exhaustive]` so future
  decoder diagnostics can add variants or fields without forcing an immediate
  public API break.
- Kept `splot-decode` with no normal or dev dependencies. JSON serialization
  stays in `splot-cli` through a private serializable view.
- Added `splot-cli -> splot-decode` as the only new internal dependency and
  updated `cargo xtask check-dependency-direction` allow-list.
- Updated `splot decode` to render the library-owned descriptor while preserving
  exit code `1`, stdout/stderr split, text field names/order, JSON field names,
  message, remediation, and no input/output file touching.
- Updated architecture, agent guidance, decoder diagnostics docs, decoder
  support matrix, implementation matrix, generated status docs, and the
  diagnostic-registry source-root comment. No Claude review workflow file was
  touched.

## Verification

- `openspec validate decode-unsupported-diagnostic-api --strict` passed.
- Focused checks passed:
  - `cargo test -p splot-decode --locked`
  - `cargo test -p splot-cli --test decode_cli --locked`
  - `cargo xtask check-dependency-direction`
  - `cargo xtask check-diagnostic-registry`
  - `cargo xtask check-decoder-support`
  - `cargo xtask check-feature-status`
  - `openspec validate --all --no-interactive`
  - `cargo machete --with-metadata`
  - `git diff --check`
- Full gate passed twice after implementation/review fixes:
  `cargo xtask ci`.
- `cargo xtask ci` warnings were limited to existing source-line advisory
  warnings and existing `cargo-deny` unmatched license allowance warnings; the
  gate ended with `ci: all checks passed`.

## Review Agents

### @reviewer / Darwin the 2nd

- Agent ID: `019ec1fa-644a-7671-a509-1832fc711d9c`
- Findings:
  1. P2 unrelated `AV2-4.11.6-LEB128` matrix `decode_check` regression.
  2. P3 stale diagnostic-registry comment still saying the CLI owns the decoder
     diagnostic.
  3. P3 tests did not pin exact message/remediation preservation.
- Resolution:
  1. Restored the LEB128 matrix row to its original `decode_check = "done"`.
  2. Updated `xtask/src/diagnostic_registry.rs` comment to state that
     `splot-decode` owns the descriptor and CLI renders it.
  3. Added exact message/remediation assertions in `splot-decode` unit tests and
     CLI JSON tests.

### @security-reviewer / Sartre the 2nd

- Agent ID: `019ec1fa-671d-7bd3-93e8-330df684a26b`
- Findings: none.
- Sign-off: no runtime decode path, no input reads/output writes, no library
  panics/unwraps in non-test code, no external dependency, and no AVM/dav2d
  source, snippets, binaries, submodules, deps, build probes, wrappers, CI jobs,
  required scripts, or external decoder execution paths were added.

### @spec-conformance-reviewer / Chandrasekhar the 2nd

- Agent ID: `019ec1fa-dc14-7581-9e5b-2d9b6b8f77c6`
- Findings: none.
- Sign-off: §7.1 is cited as diagnostic context only, existing diagnostic
  values are preserved, no runtime decode support is claimed, and OpenSpec/main
  specs are consistent.

### @encoder-impact-reviewer / Herschel the 2nd

- Agent ID: `019ec1fa-dedc-7ea2-9978-4dded828ae15`
- Findings:
  1. `DecodeDiagnostic` was exhaustive despite future decoder diagnostic fields
     being planned.
  2. New feature row marked `decode_check = "done"` even though this is
     metadata-only.
- Resolution:
  1. Added `#[non_exhaustive]` to `DecodeDiagnostic`.
  2. Changed `DECODE-UNSUPPORTED-DIAGNOSTIC-API` to
     `decode_check = "not-applicable"` and regenerated status docs.

## Archive

- `openspec archive decode-unsupported-diagnostic-api --yes` archived the
  change as `2026-06-13-decode-unsupported-diagnostic-api` and folded deltas
  into `openspec/specs/decoder-support/spec.md` and
  `openspec/specs/process/spec.md`.
- Post-archive focused checks passed:
  - `cargo test -p splot-decode --locked`
  - `cargo test -p splot-cli --test decode_cli --locked`
  - `cargo xtask check-dependency-direction`
  - `cargo xtask check-diagnostic-registry`
  - `cargo xtask check-decoder-support`
  - `cargo xtask check-feature-status`
  - `openspec validate --all --no-interactive`
  - `cargo machete --with-metadata`
  - `git diff --check`
- Post-archive full gate passed: `cargo xtask ci`.
