# Agent Log: decoder-diagnostic-registry

## Orchestrator Plan

- Branch: `feat/decoder-diagnostic-registry`, based on refreshed
  `origin/main` commit `bb40890240619dfed649963726d41e3bd2c56e59`.
- Change scope: add a canonical decoder diagnostic registry and extend the
  existing diagnostic-registry xtask to enforce emitted `decode/*` diagnostics.
- Feature IDs:
  - `DOC-DECODER-DIAGNOSTICS`
  - `XTASK-DECODER-DIAGNOSTIC-REGISTRY`
- Non-goals: no decoder/reconstruction crate, no dependency graph change, no
  AVM/dav2d committed integration, no new dependencies, no real pixel decode,
  no supported-tier expansion, and no validator diagnostic behavior change.
- Validation target: OpenSpec strict validation, focused xtask tests,
  `cargo xtask check-diagnostic-registry`, decoder/feature drift gates, and
  full `cargo xtask ci`.

## Agents

| Time | Agent | Role | Objective | Status |
|---|---|---|---|---|
| 2026-06-13 | @orchestrator | Main agent | Own OpenSpec, implementation sequencing, verification, and final acceptance. | in progress |
| 2026-06-13 | Noether (`019ec0e7-bf3c-7ad3-8bcc-1f2ae55f7b4f`) | @architect | Review design for dependency direction, CLI thinness, Feature IDs, docs/status surfaces. | signed off; matrix rows required before CI |
| 2026-06-13 | Volta (`019ec0e7-dad0-7611-8713-1821d5172f31`) | @spec-reader / @api-designer | Review OpenSpec and current CLI diagnostic contract for stable fields and mission DoD fit. | signed off with wording adjustments |
| 2026-06-13 | Leibniz (`019ec0e7-f0bb-7f73-87ae-a1bf326fddaf`) | @reference-oracle | Review whether local AVM/dav2d evidence is needed and whether the change preserves the forbidden integration boundary. | signed off |

## Planning Notes

- The change is infrastructure only. The current decoder diagnostic source root
  is `crates/splot-cli/src/commands/decode.rs` because the decoder library crate
  does not exist yet and adding it requires explicit approval.
- The existing command name, `cargo xtask check-diagnostic-registry`, remains the
  single diagnostic drift gate. Validator and decoder registries will be separate
  descriptors internally.
- The decoder registry should document the emitted ID set plus human-facing
  semantics: severity, spec section, Feature ID, support-matrix row, message, and
  remediation. The exact set comparison stays limited to rule IDs.

## Local Reference Evidence

- None used. This registry-only change does not require AVM or dav2d behavior
  study and must not add any local-reference executable integration.

## Implementation Notes

- Added `docs/DECODER-DIAGNOSTICS.md` with a marker-delimited emitted
  `decode/unsupported-feature` registry row. The document explicitly names the
  stable diagnostic field names and states that the xtask check enforces only
  the `rule_id` set.
- Refactored `xtask/src/diagnostic_registry.rs` around registry descriptors:
  the validator descriptor preserves the existing
  `crates/splot-validate/src` / `docs/VALIDATOR-DIAGNOSTICS.md` behavior, and
  the decoder descriptor scans `crates/splot-cli/src/commands/decode.rs`
  against `docs/DECODER-DIAGNOSTICS.md` for `decode/*` IDs.
- Added decoder registry unit tests for the matching case, emitted-but-missing
  documentation, and documented-but-unemitted documentation.
- Added `DOC-DECODER-DIAGNOSTICS` and
  `XTASK-DECODER-DIAGNOSTIC-REGISTRY` rows to
  `docs/IMPLEMENTATION-MATRIX.toml`.
- Added `decoder-diagnostic-registry` to
  `docs/DECODER-SUPPORT-MATRIX.toml` and regenerated
  `docs/DECODER-SUPPORT-STATUS.md`, `docs/FEATURE-STATUS.md`, and
  `docs/SPEC-COVERAGE.md`.
- Updated roadmap, feature-tracking, and spec-mapping docs so the decoder
  diagnostic registry is discoverable.

## Verification

- `cargo test -p xtask diagnostic_registry --locked`: passed.
- `cargo xtask check-diagnostic-registry`: passed
  (`validator ok (240 ids)`, `decoder ok (1 ids)`).
- `openspec validate decoder-diagnostic-registry --strict`: passed.
- `cargo xtask check-decoder-support`: passed (15 rows).
- `cargo xtask check-feature-status`: passed (141 features).
- `git diff --check`: passed.
- `cargo test -p xtask feature_status --locked`: passed after generated
  spec-coverage wording and matrix ownership fixes.
- `cargo xtask ci`: passed before review, after the spec coverage generator
  wording fix, and again after the matrix ownership / wrong-prefix fixes.
- `openspec archive decoder-diagnostic-registry --yes`: synced
  `decoder-support` and `process` specs and moved the change to
  `openspec/changes/archive/2026-06-13-decoder-diagnostic-registry/`.
- `cargo xtask ci`: passed after archive/spec sync.

## Review Findings

- Noether: no architecture blocker. The design preserves dependency direction,
  avoids new crates/dependencies, keeps CLI thin, and keeps future decoder
  library registry roots behind explicit dependency-graph approval. Sequencing
  note: add `DOC-DECODER-DIAGNOSTICS` and
  `XTASK-DECODER-DIAGNOSTIC-REGISTRY` matrix rows before CI-gated validation.
- Volta: no spec/API blocker. The decoder registry should document stable
  emitted fields explicitly: `rule_id`, `severity`, `spec_section`,
  `matrix_row`, `feature_id`, `message`, and `remediation`. The xtask check
  should be described as enforcing the `rule_id` set only; other fields stay
  protected by CLI tests and OpenSpec requirements.
- Leibniz: no reference-oracle blocker. This registry-only change does not need
  local AVM/dav2d evidence and, as planned, adds no forbidden committed
  reference integration.
- Huygens: no security findings. Confirmed the decoder scan root is static,
  no user-controlled path/command/network sink was added, and the diff adds no
  external decoder, AVM, or dav2d source/dependency/wrapper/script/CI/runtime
  integration.
- Plato: found stale generated spec-coverage wording that described diagnostics
  as if only `VALIDATOR-DIAGNOSTICS.md` existed. Fixed the generator in
  `xtask/src/feature_status.rs`, regenerated `docs/SPEC-COVERAGE.md`, reran
  focused checks, and reran full `cargo xtask ci`.
- Darwin: raised that non-`decode/` diagnostic-looking IDs in decoder emission
  roots or the decoder registry would have been silently ignored. Fixed by
  making the decoder registry descriptor reject wrong-prefix diagnostic IDs and
  adding source/doc unit tests.
- Cicero: found that the registry docs row was taking §7.1 and
  `decode/unsupported-feature` coverage ownership from `CLI-DECODE`. Fixed by
  removing `DOC-DECODER-DIAGNOSTICS` spec sections and diagnostics, making its
  validate stage not-applicable, and removing §7.1/diagnostic ownership from
  the decoder support registry row. The CLI row remains the owner of §7.1 and
  `decode/unsupported-feature`.
- Cicero re-review: signed off; generated spec coverage now keeps
  `DOC-DECODER-DIAGNOSTICS` in the no-spec-section list and leaves §7.1 on
  `CLI-DECODE`.
- Darwin re-review: signed off; wrong-prefix source and doc cases are covered
  and `decode/*` is explicit in the registry document.
- Plato re-review: signed off; spec-coverage wording and ownership split are
  resolved, with only the known future-source-root maintenance residual.
- Huygens re-review: signed off; no security findings and no external decoder,
  AVM, or dav2d boundary issue.

## Final Acceptance

- Accepted for this change. All tasks are complete, required review agents
  signed off, the change is archived with specs synced, and full
  `cargo xtask ci` passed after the final fixes.
