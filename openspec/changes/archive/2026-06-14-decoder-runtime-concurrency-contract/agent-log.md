# Agent Log: decoder-runtime-concurrency-contract

## Orchestrator Plan

- Owner: `@orchestrator`
- Model/effort: Codex GPT-5 / high-equivalent local thread
- Change scope: documentation, OpenSpec, and matrix synchronization only.
- Feature ID: `INFRA-PARALLEL-RUNTIME-POLICY`
- Goal: incorporate the merged PR #101 concurrency runtime policy into decoder
  and reconstruction design before byte-consuming decode or reconstruction
  algorithms land.
- Non-goals: no decode implementation, no reconstruction algorithm, no new
  diagnostics, no new dependencies, no AVM/dav2d repo integration, no CI
  reference-tool execution.

Implementation sequence:

1. Create OpenSpec proposal/design/spec/tasks artifacts.
2. Validate the OpenSpec change before implementation edits.
3. Update decoder roadmap, decoder support matrix, and implementation matrix
   notes to reflect `DecodeContext`/`DecodeRuntimeConfig` and the current
   `splot-decode -> splot-parallel` edge.
4. Regenerate generated status docs.
5. Run OpenSpec, decoder-support, feature-status, and concurrency policy checks.
6. Run review subagents and record findings.

## Subagents And Sub-Subagents

### `@architect`: PR #101 concurrency-model audit

- Agent id: `019ec4c9-eeeb-7180-ac3e-42fe2330946b`
- Prompt/objective: inspect merged PR #101 concurrency model and report decoder
  and reconstruction design constraints; spawn a sub-subagent sanity-check
  reviewer.
- Output/findings:
  - `splot-decode` parallel work must flow through `DecodeContext::pool()`,
    `WorkerPool::install()`, and `splot_parallel::prelude::*`.
  - `DecodeRuntimeConfig` owns runtime thread-count policy only; `auto` and `0`
    resolve once at pool creation.
  - Coarse pipeline handoffs must use bounded queues from `splot_parallel`.
  - `splot-recon` should remain scheduler-free; `splot-decode` should partition
    and schedule work, then call pure reconstruction primitives.
  - Decode hashes, diagnostics, stats, packets, and progress must be committed
    in bitstream/presentation order rather than completion order.
  - Docs needing synchronization: decoder roadmap, decoder support matrix,
    generated decoder support status, decoder-support OpenSpec, and possibly
    testing docs when runtime decode lands.
- Sub-subagent output: the spawned sanity-check agreed and flagged risks to keep
  one underlying pool per context, treat the line-based xtask scanner as
  defense-in-depth rather than the only review mechanism, and avoid copying
  `rayon::prelude::*` examples into downstream decode code.

### `@documenter`: decoder/recon mission artifact audit

- Agent id: `019ec4ca-0fcf-7112-bac1-0070de457f7a`
- Prompt/objective: audit current decoder/recon mission artifacts and recommend
  the next smallest goal-owned change incorporating PR #101; spawn a
  sub-subagent reviewer.
- Output/findings:
  - Focused checks passed before edits: `cargo xtask check-decoder-support`,
    `cargo xtask check-feature-status`, and
    `cargo xtask check-concurrency-policy`.
  - `docs/DECODER-ROADMAP.md` omits `DecodeRuntimeConfig`,
    `DecodeContext`, `splot-parallel`, `--threads`, and thread-count
    determinism.
  - `docs/DECODER-SUPPORT-MATRIX.toml` has a stale crate-scaffolding note and
    lacks a decoder runtime context row.
  - `openspec/specs/decoder-support/spec.md` lacks decoder-specific worker-pool
    and runtime-config language.
  - `docs/IMPLEMENTATION-MATRIX.toml` has stale
    `INFRA-DECODER-CRATE-SCAFFOLDING` notes.
  - Recommended smallest change: docs/spec/matrix synchronization only.
- Sub-subagent output: independently agreed with the docs/spec/matrix-only
  scope and identified the primary risk as overclaiming runtime decode support
  or implying `splot-recon` owns concurrency.

## Local Reference Commands / Evidence

None. This change does not use AVM or dav2d and does not add any local reference
evidence.

## Review Comments And Fixes

- `@reviewer` (`019ec4d2-7b47-7c33-a100-38166dacc464`) initially found:
  1. thread-count proof wording was inconsistent between proposal/design and
     roadmap/spec;
  2. roadmap Stage 1 overclaimed byte-consuming enforcement as partial;
  3. proposal impact underreported implementation-matrix and generated-doc
     changes.
- Fixes made:
  1. roadmap/spec now require future supported decode proof across
     `--threads 1`, `--threads auto`, and at least one fixed positive
     `--threads N`;
  2. roadmap Stage 1 now says byte-consuming enforcement is planned;
  3. proposal impact now lists `docs/IMPLEMENTATION-MATRIX.toml`,
     `docs/FEATURE-STATUS.md`, `docs/SPEC-COVERAGE.md`, and
     `INFRA-DECODER-CRATE-SCAFFOLDING` stale-note cleanup.
- `@reviewer` re-reviewed after fixes and signed off with no findings.
- `@security-reviewer` (`019ec4d2-94d9-7083-a1a8-f1aa7e1ccde6`) signed off
  with no findings. Boundary statement: No AVM/dav2d source, snippets,
  binaries, submodules, dependencies, build probes, wrappers, CI jobs, required
  scripts, required xtask commands, or mandatory tests were added.
- `@spec-conformance-reviewer`
  (`019ec4d2-ae16-7ed0-8da7-2b6621fa9666`) signed off with no findings. The
  patch remains project runtime policy only and does not edit
  `docs/spec/av2/1.0.0/`.
- `@encoder-impact-reviewer` (`019ec4d2-c299-7a63-ba7e-aa1cb5121a1a`) signed
  off with no findings. The patch keeps `splot-recon` scheduler-free and
  reusable by a future encoder while leaving orchestration to `splot-decode` or
  another caller that owns runtime policy.

Verification recorded during review:

- `openspec validate decoder-runtime-concurrency-contract --strict`
- `openspec validate --all --no-interactive`
- `cargo xtask check-decoder-support`
- `cargo xtask check-feature-status`
- `cargo xtask check-concurrency-policy`
- `cargo xtask ci`
- `cargo test -p splot-decode --locked`
- `cargo test -p splot-cli decode_threads --locked`
- `git diff --check`

## Final Acceptance Decision

Accepted for this change. The decoder/reconstruction docs, support matrix,
implementation matrix notes, generated status docs, and OpenSpec delta now
incorporate PR #101's concurrency model without adding runtime decode behavior
or any AVM/dav2d integration.
