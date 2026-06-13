# Agent Log: decoder-limits-contract

## Orchestrator Plan

Goal: execute the next unblocked decoder mission slice after PR #85 merged.
Crate scaffolding is blocked on explicit dependency-graph approval, so this
change is limited to OpenSpec, docs, matrices, generated status, and existing
drift checks.

Selected change: `decoder-limits-contract`.

Feature ID: `DOC-DECODE-LIMITS-CONTRACT`.

Scope boundaries:

- no new workspace crates;
- no dependency graph changes;
- no runtime `splot decode` behavior changes;
- no emitted `decode/resource-limit` source yet;
- no AVM/dav2d source, snippets, binaries, wrappers, scripts, `xtask`
  commands, CI jobs, required tests, or local paths.

## Planning Agents

### @architect

Agent: Archimedes (`019ec10d-1dc1-7df3-bcf0-3607186db1ac`)

Objective: evaluate whether `decoder-limits-contract` is a valid next
OpenSpec change for the `decode-limits-budget` row without crate approval.

Output summary:

- Confirmed the change is valid if contract-only.
- Recommended budget categories: input/OBU bytes, tile payload bytes, sequence
  max dimensions, per-frame dimensions, tile counts, reference-frame storage,
  frame count, and output-byte limits.
- Recommended planned `decode/resource-limit` fields but no emission yet.
- Confirmed no dependency-graph approval is needed unless crates, dependencies,
  AVM/dav2d integration, or runtime APIs are added.

### @spec-reader

Agent: Goodall (`019ec10d-3296-7481-b83e-d09f292e5811`)

Objective: inspect the committed AV2 v1.0.0 mirror and identify exact anchors
and requirements relevant to decode limits/allocation budgets.

Output summary:

- Primary anchors: § 6.4.1 sequence header semantics,
  § 6.17.4.1 frame size semantics, § 7.1 general decoding process,
  § 5.20 tile payload/decode tile syntax, § 7.21 output processes, and
  § 7.23 reference frame update/storage.
- Noted that output arrays use cropped dimensions while internal frame buffers
  are based on padded frame dimensions.
- Noted reference storage, tile counts, output frames, bit depth, chroma format,
  and extracted-stream scope as required contract surfaces.

### @api-designer

Agent: Aquinas (`019ec10d-48f6-7ba0-a653-16f1aa1251ac`)

Objective: assess future `DecodeLimits`/`DecodeOptions` and
`decode/resource-limit` diagnostic shape without adding crates or runtime APIs.

Output summary:

- Recommended conceptual `DecodeOptions { limits: DecodeLimits }`.
- Recommended treating limits as caller/resource policy, not AV2 conformance.
- Recommended stable resource fields: `limit_name`, `limit`, `actual`, `unit`,
  `byte_offset`, and `bit_offset` in addition to existing decoder diagnostic
  fields.
- Confirmed `decode/resource-limit` must remain outside the emitted registry
  marker until source emits it.
- Confirmed the current CLI JSON shape is a single diagnostic object and should
  not be expanded by this docs-only change.

## Local Reference Evidence

No AVM or dav2d runs were used for this change. The change is contract-only and
does not generate or validate decoder fixtures.

## Implementation Notes

Implemented as a contract-only documentation/matrix change:

- Added OpenSpec proposal, design, decoder-support delta spec, and tasks.
- Updated `docs/DECODER-ROADMAP.md` with the conceptual
  `DecodeOptions { limits: DecodeLimits }` contract, the limit field set,
  pre-allocation/pre-traversal requirements, checked arithmetic requirements,
  and planned `decode/resource-limit` shape.
- Updated `docs/DECODER-SUPPORT-MATRIX.toml` so `decode-limits-budget` is
  `partial`, owned by `DOC-DECODE-LIMITS-CONTRACT`, and proven by
  archive-stable checks.
- Added planned `decode/resource-limit` documentation outside the enforced
  emitted diagnostic registry marker region in `docs/DECODER-DIAGNOSTICS.md`.
- Added `DOC-DECODE-LIMITS-CONTRACT` to `docs/IMPLEMENTATION-MATRIX.toml` as a
  docs-only feature with no normative spec-section ownership and no runtime
  `decode_check` status.
- Regenerated `docs/DECODER-SUPPORT-STATUS.md`, `docs/FEATURE-STATUS.md`, and
  `docs/SPEC-COVERAGE.md`.

Verification before review fixes:

- `openspec validate decoder-limits-contract --strict`
- `cargo xtask check-decoder-support`
- `cargo xtask check-feature-status`
- `cargo xtask check-diagnostic-registry`
- `cargo xtask ci`

Verification after review fixes:

- `openspec validate decoder-limits-contract --strict`
- `cargo xtask check-decoder-support`
- `cargo xtask check-feature-status`
- `cargo xtask check-diagnostic-registry`
- `git diff --check`
- `cargo xtask ci`

Post-archive verification:

- `openspec validate --all --no-interactive`
- `git diff --check`
- `cargo xtask ci`

## Review Notes

### @reviewer

Agent: Popper (`019ec117-9563-7c41-9227-dd625c59c8d7`)

Findings:

- P2: Proof commands used `openspec validate decoder-limits-contract --strict`,
  which would become stale after archive.
- P2: `DOC-DECODE-LIMITS-CONTRACT` marked `decode_check = "partial"` even
  though no runtime decoder/inspector/validator limit checks exist.
- P3: Verification tasks were checked, but implementation/review notes were
  still incomplete in this log.

Resolution:

- Replaced matrix/support proof commands with archive-stable
  `openspec validate --all --no-interactive` plus
  `cargo xtask check-decoder-support`.
- Changed the implementation-matrix `decode_check` stage to `not-applicable`
  for the docs-only contract row and regenerated generated docs.
- Follow-up review found the first `decode_check` edit missed the target row;
  the intended `DOC-DECODE-LIMITS-CONTRACT` row now renders `DecChk n/a` in
  `docs/FEATURE-STATUS.md`.
- Filled in implementation notes, verification evidence, and review resolutions
  in this log.

Final verification: signed off with no remaining findings.

### @spec-conformance-reviewer

Agent: Pascal (`019ec117-ce27-7af3-b691-566cb60c3578`)

Findings:

- Important: the contract under-cited the AV2 sections that supply
  `NumRefFrames` and tile count measured values. The original anchors for
  reference storage (§ 7.23) and tile traversal (§ 5.20) were valid but
  incomplete for the count fields.

Resolution:

- Added § 6.4.6 for reference-frame count, § 6.17.7.2 for tile grid counts, and
  § 5.19 for `NumTiles` derivation to the roadmap, OpenSpec delta, design, and
  decoder support matrix.
- Kept `DOC-DECODE-LIMITS-CONTRACT` with `spec_sections = []` in the global
  implementation matrix so `docs/SPEC-COVERAGE.md` continues to list it only
  under sectionless documentation work.

Final verification: signed off with no remaining findings.

### @security-reviewer

Agent: Helmholtz (`019ec117-af9d-77f3-ab01-a7d815fbf4f2`)

Findings:

- Medium: `max_input_bytes` and `max_obus` were listed but the normative
  allocation-gating scenario did not require checking them before input
  buffering or OBU traversal.
- Low: derived size checks did not explicitly require checked arithmetic before
  comparison/allocation.

Resolution:

- Updated `docs/DECODER-ROADMAP.md` and the decoder-support OpenSpec delta to
  require checking `max_input_bytes` before buffering/accepting bytes and
  `max_obus` before continuing OBU traversal or accumulating OBU state.
- Added a checked-arithmetic requirement: overflow while deriving dimensions,
  strides, tile products, plane sizes, reference-storage bytes, output bytes, or
  frame counts is a `decode/resource-limit` failure, not wraparound or panic.

Boundary sign-off:

No AVM/dav2d source, snippets, binaries, submodules, dependencies, build
probes, wrappers, CI jobs, required scripts, required `xtask` commands,
mandatory tests, or local absolute paths were added.

Final verification: signed off with no remaining findings.

### @encoder-impact-reviewer

Agent: Maxwell (`019ec117-e2f2-7a32-b6b5-c894849c61bc`)

Findings: none.

Sign-off: the contract helps future encoder closed-loop/reconstruction work by
requiring limits before decoded-frame allocation, tile traversal, reference
storage, hashing, and Y4M output; it keeps the API conceptual and does not block
the future `splot-recon` / `splot-decode` split.

Final verification: signed off with no remaining findings.
