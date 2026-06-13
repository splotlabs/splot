# Agent Log: decoder-crate-scaffolding

## Orchestrator Plan

Objective: add the approved decoder/reconstruction crate scaffolding and update
the repository automation/docs that describe and enforce the expanded workspace
boundary, without adding runtime decode behavior.

Reason for selecting this slice: maintainer approval is now available for the
crate/dependency graph change that earlier decoder mission slices deliberately
blocked on.

Feature ID: `INFRA-DECODER-CRATE-SCAFFOLDING`.

Maintainer approval: the user replied `Approved` after the prior goal was
blocked on explicit crate/dependency graph approval.

Baseline: `cargo xtask ci` passed on the rebased `origin/main` before this
change.

## Planning Agents

### @architect / Tesla

- Agent ID: `019ec1d1-7010-7c43-b60b-8b7aa14a6dcd`
- Objective: recommend a PR-sized crate-scaffolding slice.
- Output: add `splot-recon` and `splot-decode` as workspace library crates,
  update dependency-direction automation and architecture docs, keep public APIs
  empty until later allocation/frame/plane contracts are implemented in source,
  and avoid package dependencies from `splot-encode` or `splot-cli` in this
  slice.

### @spec-reader / Lorentz

- Agent ID: `019ec1d1-7fe1-70c2-ae2c-167d4653e8b6`
- Objective: identify AV2 citations and wording constraints for crate
  scaffolding.
- Output: no AV2 spec citations are needed because this change introduces crate
  topology, not syntax or semantics. The new matrix rows should use
  `spec_sections = []` and avoid claims of runtime decode, reconstruction,
  conformance, frame hashes, Y4M output, new diagnostics, or local AVM/dav2d
  evidence.

### @api-designer / Kuhn

- Agent ID: `019ec1d1-9241-79e1-b26e-bcc2eec39aaf`
- Objective: recommend the minimal crate/API shape and avoid tooling failures.
- Output: add `splot-recon` and `splot-decode` as workspace library crates with
  SPDX headers and crate docs only. Avoid public placeholder APIs and avoid
  declaring package dependencies before source uses them; record approved future
  edges in docs and dependency-direction allow-lists instead.

### @reference-oracle / Hume

- Agent ID: `019ec1d1-a26b-7a23-91fe-505d35e5b0e0`
- Objective: determine whether AVM/dav2d evidence is needed and define the
  local-reference boundary.
- Output: no AVM/dav2d evidence is needed. The scaffold is not proof of runtime
  decode, reconstruction, hash, or Y4M support. Do not commit source, binaries,
  build trees, bindings, wrappers, submodules, subtrees, Cargo dependencies,
  build probes, runtime process calls, CI jobs, tests requiring external
  decoders, local paths, copied reference code/prose, or fabricated evidence.

### @avm-reader-runner / Raman

- Agent ID: `019ec1d1-b510-7e23-b9ce-9c6e3695a320`
- Objective: determine whether local AVM read/run evidence is needed.
- Output: no local AVM run or source read is needed because this slice does not
  implement AV2 syntax parsing, reconstruction semantics, prediction,
  transforms, loop filters, reference state, or bitstream acceptance behavior.

### @dav2d-reader-runner / Parfit

- Agent ID: `019ec1d1-c736-7872-b91a-259aa6777881`
- Status: pending while the OpenSpec draft was corrected. The no-local-reference
  boundary above applies unless this agent reports a narrower constraint.

## Workflow Note

The implementation branch `codex/decoder-crate-scaffolding` was created before
the OpenSpec draft was validated, but before source edits. The change is still
validated before implementation work proceeds.

## Local Reference Boundary

No AVM, dav2d, ffmpeg, or network command is needed for this change. No
AVM/dav2d source, snippets, binaries, submodules, dependencies, build probes,
wrappers, scripts, CI jobs, required tests, or local paths may be committed.
`cargo xtask ci` must remain runnable on machines without AVM or dav2d
installed.

The local-reference evidence manifest remains portable metadata only and is not
evidence of runtime `splot decode` support.

## Implementation Notes

- Added `crates/splot-recon` and `crates/splot-decode` as workspace library
  crates with SPDX headers, crate-level docs, workspace lint inheritance, and no
  public runtime API.
- Kept `splot-decode` free of declared package dependencies until source code
  uses `splot-core` or `splot-recon`; the approved future edge is recorded in
  `cargo xtask check-dependency-direction` and docs.
- Added both crates to workspace members, default members, workspace dependency
  aliases, and `Cargo.lock`.
- Updated dependency-direction rules, decoder diagnostic registry scan roots
  (`splot-decode`, not the shared `splot-recon` root), feature-status allowed
  crate/owner metadata, and the splot-validate coverage exclusion regex in both
  `xtask` and CI.
- Updated `AGENTS.md`, `docs/ARCHITECTURE.md`, decoder roadmap/diagnostics,
  GitHub review guidance, decoder support matrix, implementation matrix schema,
  and generated status docs without claiming runtime decode support.

## Verification

- `openspec validate decoder-crate-scaffolding --strict`
- `cargo check -p splot-recon`
- `cargo check -p splot-decode`
- `cargo check -p splot-recon --locked`
- `cargo check -p splot-decode --locked`
- `git diff --check`
- `cargo xtask check-dependency-direction`
- `cargo xtask check-diagnostic-registry`
- `cargo xtask check-decoder-support`
- `cargo xtask check-feature-status`
- `openspec validate --all --no-interactive`
- `cargo machete --with-metadata`
- `cargo xtask decoder-support --format markdown --output docs/DECODER-SUPPORT-STATUS.md`
- `cargo xtask feature-status --format markdown --output docs/FEATURE-STATUS.md`
- `cargo xtask spec-coverage --format markdown --output docs/SPEC-COVERAGE.md`
- `cargo xtask ci`
- Post-review: `openspec validate decoder-crate-scaffolding --strict`
- Post-review: `git diff --check`
- Post-review: `cargo xtask check-diagnostic-registry`
- Post-review: `cargo xtask check-decoder-support`
- Post-review: `cargo xtask check-feature-status`
- Post-review: `openspec validate --all --no-interactive`
- Post-review: `cargo xtask ci`

## Review Agents

### general-reviewer / Heisenberg

- Agent ID: `019ec1dc-3ed7-7cc0-af64-474e32e24406`
- Findings: none.
- Notes: independently ran focused checks covering new crates, dependency
  direction, decoder support, diagnostic registry, feature status, license
  headers, machete, OpenSpec validation, and `git diff --check`.

### security-reviewer / Epicurus

- Agent ID: `019ec1dc-8d62-7250-98cf-2c9eb14b02ff`
- Findings: none.
- Notes: confirmed empty crate dependency lists, no transitive deps, no public
  API, no `unsafe`, panics, process execution, FFI, external decoder invocation,
  wrappers, local absolute paths, build scripts, submodules, or new supply-chain
  surface.

### spec-conformance-reviewer / Gibbs the 2nd

- Agent ID: `019ec1dc-a935-7600-8178-e8eb95f46431`
- Finding:
  - `P2`: the OpenSpec design still described declared `splot-decode ->
    splot-core/splot-recon` dependencies with private `use ... as _` markers,
    contradicting the implemented empty dependency list.
- Resolution:
  - Updated `design.md` to describe the chosen no-declared-dependencies approach
    until source imports are real.
  - Re-ran `openspec validate decoder-crate-scaffolding --strict` and
    `git diff --check`.

### encoder-impact-reviewer / Peirce the 2nd

- Agent ID: `019ec1dc-d046-7c43-b9eb-00b68631af70`
- Finding:
  - `P2`: scanning all of shared `splot-recon/src` as decoder diagnostics would
    force future encoder/reconstruction diagnostic-looking IDs into the
    `decode/*` namespace.
- Resolution:
  - Removed `crates/splot-recon/src` from `DECODER_SOURCE_ROOTS`.
  - Updated `docs/DECODER-DIAGNOSTICS.md` to explain that `splot-recon` is shared
    infrastructure and should only be scanned through a future narrower
    decoder-owned emission path.
  - No encoder reference gate was triggered because `splot-encode`,
    encoder-facing `splot-core`, and encoder research docs were not changed.

## Archive

- Ran `openspec archive decoder-crate-scaffolding --yes`.
- Archive path:
  `openspec/changes/archive/2026-06-13-decoder-crate-scaffolding/`.
- Synced requirements into `openspec/specs/decoder-support/spec.md` and
  `openspec/specs/process/spec.md`.
- Post-archive verification:
  - `cargo check -p splot-recon --locked`
  - `cargo check -p splot-decode --locked`
  - `cargo xtask check-dependency-direction`
  - `cargo xtask check-diagnostic-registry`
  - `cargo xtask check-decoder-support`
  - `cargo xtask check-feature-status`
  - `openspec validate --all --no-interactive`
  - `cargo machete --with-metadata`
  - `git diff --check`
  - `cargo xtask ci`
