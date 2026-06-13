# Agent Log: minimal-decode-tier-contract

## Orchestrator Plan

Objective: define the first intended `splot decode` success tier as a
docs/OpenSpec-only contract while crate and dependency-graph changes remain
blocked on explicit maintainer approval.

Reason for selecting this slice: it advances decoder implementation planning
without adding crates, changing Cargo manifests, changing CLI behavior, adding
runtime byte consumption, or introducing AVM/dav2d integration.

Feature ID: `DOC-MINIMAL-DECODE-TIER-CONTRACT`.

## Planning Agents

### @architect / Gibbs

- Agent ID: `019ec15d-67be-72d0-a83f-10ff6d5b1272`
- Objective: assess whether `minimal-decode-tier-contract` is a valid
  independent docs/OpenSpec-only PR-sized slice while crate approval is blocked.
- Output: confirmed the slice is valid if it stays contract-only. Recommended
  a `partial` decoder support row, docs/OpenSpec/generated-status files only,
  no runtime rows marked supported, no Cargo or diagnostic registry changes, and
  Feature ID `DOC-MINIMAL-DECODE-TIER-CONTRACT`.

### @spec-reader / Einstein

- Agent ID: `019ec15d-93b5-71f1-819d-1705de33fd58`
- Objective: read committed AV2 v1.0.0 spec mirror sections needed to define a
  conservative first decode tier.
- Output: recommended calling the tier a `splot` implementation subset rather
  than an Annex A decoder conformance claim. Identified Annex B input,
  single-layer OBU header constraints, temporal/CELU shape, 8-bit 4:2:0 sequence
  format, closed-loop key-frame-only frames, fixed sequence dimensions with no
  cropping, one tile / one tile group, hash-first output, and explicit
  unsupported areas.

### @api-designer / Nash

- Agent ID: `019ec15d-b573-7103-a49b-b44277f5abd0`
- Objective: propose semantic contract fields and future diagnostic boundaries
  without adding source code.
- Output: recommended `contract_id = "splot.decode.minimal_tier"`,
  `contract_version = 1`, `tier_id = "minimal-intra-8bit420-hash-v1"`, future
  `DecodeOptions` concepts for limits/selection/input/output policy, a positive
  allowlist, planned detail fields for unsupported-tier diagnostics, and
  hash-first success before Y4M. Noted the current CLI requires `-o`, so runtime
  implementation may need a hash-output CLI mode before decode can be supported.

### @reference-oracle / Hubble

- Agent ID: `019ec15d-cffb-7970-9343-db34d80b93fd`
- Objective: inspect repo-local docs and archives for already-recorded AVM/dav2d
  evidence relevant to tier selection.
- Output: found no applicable local reference evidence for this contract. The
  existing archived AVM/dav2d raw MD5 evidence remains scoped to deterministic
  hash planning and must not be used as proof of tier selection or `splot`
  runtime behavior.

## Local Reference Boundary

No AVM or dav2d command was run for this change. No AVM/dav2d source, snippets,
binaries, submodules, dependencies, build probes, wrappers, CI jobs, required
scripts, `xtask` commands, or mandatory tests are added by this change. The
decoder support matrix row keeps `local_reference_evidence = []`.

## Implementation Notes

- Updated `docs/DECODER-ROADMAP.md` to define
  `splot.decode.minimal_tier` v1 and `minimal-intra-8bit420-hash-v1` as a
  contract-only `splot` implementation subset, not an Annex A decoder
  conformance claim.
- Added the `minimal-decode-tier-contract` row to
  `docs/DECODER-SUPPORT-MATRIX.toml` with `partial` status, planned
  `decode/unsupported-feature` and `decode/resource-limit` diagnostics, and no
  local reference evidence.
- Added `DOC-MINIMAL-DECODE-TIER-CONTRACT` to
  `docs/IMPLEMENTATION-MATRIX.toml` as a docs-only feature with runtime stages
  marked not-applicable.
- Regenerated `docs/DECODER-SUPPORT-STATUS.md`, `docs/FEATURE-STATUS.md`, and
  `docs/SPEC-COVERAGE.md`.

## Verification

- `openspec validate minimal-decode-tier-contract --strict` passed.
- `openspec validate --all --no-interactive` passed.
- `cargo xtask check-decoder-support` passed with 16 decoder support rows.
- `cargo xtask check-feature-status` passed with 145 tracked features.
- `cargo xtask check-diagnostic-registry` passed.
- `git diff --check` passed.
- `cargo xtask ci` passed.
- After review fixes, `openspec validate minimal-decode-tier-contract --strict`,
  `openspec validate --all --no-interactive`,
  `cargo xtask check-decoder-support`, `cargo xtask check-feature-status`,
  `cargo xtask check-diagnostic-registry`, `git diff --check`, and
  `cargo xtask ci` passed.
- After archive, `openspec validate --all --no-interactive`,
  `cargo xtask check-decoder-support`, `cargo xtask check-feature-status`,
  `cargo xtask check-diagnostic-registry`, `git diff --check`, and
  `cargo xtask ci` passed.

## Review

### @reviewer / Laplace

- Agent ID: `019ec16a-da21-74e2-840c-ef66a7f6fa61`
- Findings:
  - Medium: the contract said AV2 Main 4:2:0 but did not pin
    `seq_profile_idc`, leaving profile acceptance ambiguous.
  - Low: the closed-loop key-frame wording relied on `FrameType = KEY_FRAME`
    and `FrameIsIntra = 1`, which can also be true for open-loop key frames.
- Resolution: updated the roadmap, OpenSpec design/spec delta, support matrix,
  generated decoder support status, and implementation matrix notes to require
  `seq_profile_idc == 0` (`Main_420_10_IP0`) and
  `obu_type == OBU_CLOSED_LOOP_KEY`.

### @security-reviewer / Aristotle

- Agent ID: `019ec16a-dd39-7391-88d8-1d49b025f4c0`
- Findings: none. Confirmed docs/OpenSpec-only scope with no Cargo, dependency,
  CI, script, local-path, AVM/dav2d executable integration, mandatory external
  decoder tool, unsafe-code, or license-boundary issue.

### @spec-conformance-reviewer / Beauvoir

- Agent ID: `019ec16a-dfc4-7d60-8768-f3e18d881cae`
- Findings:
  - Important: the closed-loop tier predicate was under-specified without
    `obu_type == OBU_CLOSED_LOOP_KEY`.
  - Important: the AV2 Main 4:2:0 wording needed an explicit
    `seq_profile_idc` boundary or different wording.
- Resolution: same as the @reviewer resolution. The spec reviewer reported no
  issues with Annex B input, IVF-as-Annex-B payload wording, `bit_depth_idc == 1`
  for 8-bit, `chroma_format_idc == 0` for 4:2:0, no-crop and output-hash
  wording, one tile and tile group, deterministic hash emission index, or the
  non-runtime `partial` matrix/status rows.

### @encoder-impact-reviewer / Banach

- Agent ID: `019ec16a-e2cd-7160-8597-5a46e62de241`
- Findings: none. Confirmed no encoder-facing code, encoder research docs,
  Cargo manifests, dependency graph changes, runtime decode claims, or
  misleading encoder-MVP promises before runtime decode support exists.

## Archive

- Archived with `openspec archive minimal-decode-tier-contract --yes` as
  `openspec/changes/archive/2026-06-13-minimal-decode-tier-contract/`.
- Synced one added and one modified requirement into
  `openspec/specs/decoder-support/spec.md`.
