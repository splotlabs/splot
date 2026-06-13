# Agent Log: decoded-frame-plane-model-contract

## Orchestrator Plan

Objective: define a docs/OpenSpec-only decoded frame and plane model contract
needed by future reconstruction crates, frame hashes, Y4M output, reference
storage, and encoder closed-loop APIs.

Reason for selecting this slice: adding `splot-recon` or `splot-decode` still
requires explicit maintainer approval. This contract advances Stage 2 without
changing the workspace dependency graph, runtime APIs, CLI behavior, or
reference-tool boundary.

Feature ID: `DOC-DECODED-FRAME-PLANE-MODEL-CONTRACT`.

## Planning Agents

### @architect / Pasteur

- Agent ID: `019ec140-3e38-7160-b720-1a9fc87e621e`
- Objective: assess whether `decoded-frame-plane-model-contract` is a valid
  independent PR-sized docs/OpenSpec slice.
- Output: confirmed the slice is valid if it stays docs/OpenSpec-only. Recommended
  moving `decoded-frame-plane-model` from `todo` to contract-only `partial`,
  defining semantic frame/plane/pixel-format/bit-depth/crop/stride contracts,
  and avoiding crates, Cargo changes, CLI behavior, diagnostics, runtime modules,
  or AVM/dav2d integration.

### @spec-reader / Godel

- Agent ID: `019ec140-55b4-7330-ad8c-af7fb130f733`
- Objective: read committed AV2 v1.0.0 spec mirror sections relevant to decoded
  frame and plane modeling.
- Output: identified AV2 § 6.4.1 for `BitDepth`, `SubsamplingX`,
  `SubsamplingY`, `Monochrome`, `NumPlanes`, and chroma-format mapping;
  § 6.17.4.1 / § 6.17.4.4 for frame dimensions and crop constraints;
  § 7.21.1 / § 7.21.2 for cropped output arrays; and § 7.23 for
  reference-frame storage metadata and padded loop-restored `FrameStore`.

### @api-designer / Carson

- Agent ID: `019ec140-6eef-7602-9dfb-35d19f36a3e1`
- Objective: propose future stable contract fields and invariants for
  `DecodedFrame`, `Plane<T>`, `PixelFormat`, and `BitDepth` without adding code
  or crates.
- Output: recommended a semantic contract with `PixelFormat`, bit depth, separate
  stored and visible dimensions, visible luma crop rectangle, output-order
  indexing, explicit `stride_samples`, checked `buffer_len`, immutable emitted
  frames, and aliasing rules for reference-slot reuse.

### @reference-oracle / Ramanujan

- Agent ID: `019ec140-8380-7ab2-8b25-53c1af749cc0`
- Objective: inspect only repo-local docs/agent logs for already-recorded
  AVM/dav2d evidence relevant to frame layout or output hashes.
- Output: confirmed no new local reference evidence applies to this plane-model
  contract. Existing AVM/dav2d raw MD5 evidence remains scoped to the
  deterministic-frame-hash row and is not proof of `splot` frame-plane modeling.

## Local Reference Boundary

No AVM or dav2d command was run for this change. No AVM/dav2d source, snippets,
binaries, submodules, dependencies, build probes, wrappers, CI jobs, required
scripts, `xtask` commands, or mandatory tests are added by this change.

## Implementation Notes

- The contract distinguishes cropped output arrays from padded reference-frame
  storage.
- The contract uses semantic future names only; no Rust types or crates are
  introduced.
- The contract requires checked arithmetic and `DecodeLimits` gating before
  future allocation, hashing, Y4M output, or reference storage.

## Verification

- `openspec validate decoded-frame-plane-model-contract --strict` passed before
  branch creation and again after docs edits.
- `openspec validate --all --no-interactive` passed.
- `cargo xtask check-decoder-support` passed.
- `cargo xtask check-feature-status` passed.
- `cargo xtask check-diagnostic-registry` passed.
- `git diff --check` passed.
- `cargo xtask ci` passed before archive.
- After review fixes, `openspec validate decoded-frame-plane-model-contract --strict`,
  `openspec validate --all --no-interactive`, `cargo xtask check-decoder-support`,
  `cargo xtask check-feature-status`, `cargo xtask check-diagnostic-registry`,
  `git diff --check`, and `cargo xtask ci` passed again.

## Review

### @reviewer / Gauss

- Agent ID: `019ec14b-cb78-7552-8df3-9131d8a5031b`
- Finding: none.

### @security-reviewer / Peirce

- Agent ID: `019ec14b-f4d4-7f60-99eb-e206ada6afe7`
- Finding: shared-frame aliasing wording allowed reference-counted mutable
  buffers to mutate already emitted output frames.
- Resolution: tightened the contract to allow borrowed/shared views only when
  backing samples are immutable for the output view, the output owns an
  independent copy, or copy-on-write / unique ownership is proven before
  reference-slot mutation.
- Finding: plane buffer length units and byte-limit charging were ambiguous.
- Resolution: replaced the ambiguous `buffer_len` invariant with checked
  `required_samples`, checked `allocation_bytes`, backing sample length, and
  full backing allocation charging against `DecodeLimits`.

### @spec-conformance-reviewer / Boyle

- Agent ID: `019ec14c-0e3d-7390-8b4c-e9c97eeb26e6`
- Finding: the contract incorrectly described zero-based output order as AV2
  § 7.21 output order.
- Resolution: reworded zero-based ordering as a `splot`-owned emission index
  over frames emitted by AV2 output processes, added § 7.1, § 7.21.5, and
  § 7.21.6 to the contract citations, and updated the adjacent hash-policy
  wording to use the same repository-owned index.

### @encoder-impact-reviewer / Russell

- Agent ID: `019ec14c-285f-7c03-845c-194c7bc253b5`
- Finding: none. The change does not touch `splot-encode`, encoder-facing
  `splot-core` syntax/parsing code, encoder research docs, Cargo manifests, or
  the dependency graph; the encoder reference gate does not apply.

## Archive

- Archived with `openspec archive decoded-frame-plane-model-contract --yes`.
- OpenSpec synced the decoded-frame and plane model requirement into
  `openspec/specs/decoder-support/spec.md`.
- Post-archive `openspec validate --all --no-interactive`,
  `cargo xtask check-decoder-support`, `cargo xtask check-feature-status`,
  `cargo xtask check-diagnostic-registry`, `git diff --check`, and
  `cargo xtask ci` passed.
