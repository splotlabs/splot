# Agent Log: deterministic-frame-hash-contract

## Orchestrator Plan

Objective: define a deterministic decoded-frame hash contract required for
future decoder fixtures, Y4M output, and encoder roundtrip evidence, without
changing the dependency graph or adding runtime decoder/hash implementation.

Reason for selecting this slice: adding `splot-recon` or `splot-decode`
requires explicit maintainer approval. That approval has not been granted, so
the orchestrator selected an independent docs/OpenSpec contract slice that
makes the requested decoder end state more precise without crossing the blocked
crate boundary.

Feature ID: `DOC-DETERMINISTIC-FRAME-HASH-CONTRACT`.

## Planning Agents

### @architect / Sagan

- Agent ID: `019ec12c-4e32-7b30-8f33-d63c6f098641`
- Objective: assess whether `deterministic-frame-hash-contract` is a valid
  independent PR-sized slice before crate scaffolding approval.
- Output: confirmed the slice is valid if it stays contract-only. Recommended
  moving `deterministic-frame-hash` from `todo` to `partial`, defining the
  canonical hash format, distinguishing it from AV2 metadata hashes, and
  avoiding runtime API, CLI behavior, crates, dependencies, or AVM/dav2d
  runners.

### @spec-reader / McClintock

- Agent ID: `019ec12c-6230-7830-b2bf-0082ba719deb`
- Objective: read committed AV2 v1.0.0 spec mirror sections relevant to
  decoded-frame hash policy.
- Output: identified AV2 § 5.17.12 metadata hash syntax, § 6.16.13 metadata
  decoded-frame-hash semantics, § 7.21.1 output process, § 7.21.2 intermediate
  output preparation, and § 7.21.7 film-grain synthesis as the required
  anchors. Confirmed cropped `OutY`/`OutU`/`OutV` dimensions, raster sample
  order, Y/U/V plane order, 8-bit single-byte samples, greater-than-8-bit
  little-endian two-byte samples, MD5 metadata semantics, and the pre-grain
  versus post-grain distinction.

### @api-designer / Dewey

- Agent ID: `019ec12c-8156-7533-bfe2-4daee92a1fd7`
- Objective: propose future stable fields for a repo-owned deterministic
  decoded-frame hash without adding code or crates.
- Output: recommended a versioned `splot.decoded_frame_hash` record with
  `contract_version = 1`, `algorithm_id = "splot-dfh-sha256-v1"`,
  `byte_stream_id = "av2-output-samples-v1"`, output order, cropped-visible
  region, stride exclusion, plane order, bit depth, chroma dimensions,
  raw-intermediate film-grain policy, metadata exclusion, and SHA-256 digest.
  Recommended keeping AV2 MD5 metadata verification as a separate future
  interop path.

### @reference-oracle / Curie

- Agent ID: `019ec12c-96c4-7313-b3f2-c53ecb2e624d`
- Objective: inspect only repo-local docs/agent logs for already-recorded
  AVM/dav2d evidence; do not run AVM/dav2d or inspect external checkouts.
- Output: confirmed archived local reference evidence exists from
  `decoder-roadmap-matrix-boundary`: AVM commit
  `f6f0b9c8914f38be39a953c0a9aa6a2e4050717c`, dav2d commit
  `f4f96cb06bb3cd3f31e29e1f190f1c0e373ab352`, and raw MD5 agreement for two
  tiny fixtures. This evidence is stale, non-executable metadata only and does
  not prove that `splot` hash computation is implemented.

## Local Reference Boundary

No AVM or dav2d command was run for this change. No AVM/dav2d source, snippets,
binaries, submodules, dependencies, build probes, wrappers, CI jobs, required
scripts, `xtask` commands, or mandatory tests are added by this change.

## Implementation Notes

- The contract uses AV2 § 6.16.13 sample-byte serialization as the canonical
  byte stream.
- The repo-owned fixture/roundtrip digest is named `splot-dfh-sha256-v1`.
- AV2 `hash_type = 0` MD5 remains a future metadata verification path, not the
  primary `splot` fixture identity.
- The default future hash variant is raw decoded output before film-grain
  synthesis, aligned with AV2 `has_grain = 0`.

## Verification

- `openspec validate deterministic-frame-hash-contract --strict` passed.
- `openspec validate --all --no-interactive` passed.
- `cargo xtask check-decoder-support` passed.
- `cargo xtask check-feature-status` passed.
- `cargo xtask check-diagnostic-registry` passed.
- `git diff --check` passed.
- `cargo xtask ci` passed.

## Review

### @reviewer / Harvey

- Agent ID: `019ec132-f637-7af2-aacf-22d1555bca87`
- Finding: P3 stale roadmap wording. `docs/DECODER-ROADMAP.md` said the hash
  contract was active in `deterministic-frame-hash-contract`, but this change
  archives the OpenSpec delta before PR completion.
- Resolution: changed the Stage 2 status to archive-stable wording: "hash
  contract documented; types planned".

### @security-reviewer / Turing

- Agent ID: `019ec133-114f-7cc2-9817-f38ba6865ab1`
- Findings: none.
- Signoff: confirmed no source, Cargo, scripts, `xtask`, CI, submodule, binary,
  wrapper, build-probe, or byte-consuming/allocation behavior changed; AVM/dav2d
  mentions are non-executable policy or archived-evidence references only; no
  local absolute paths were found.

### @spec-conformance-reviewer / Hypatia

- Agent ID: `019ec133-34bb-7101-aebc-fae662aca08a`
- Findings: none.
- Signoff: confirmed the contract matches AV2 § 5.17.12, § 6.16.13,
  § 7.21.1, § 7.21.2, and § 7.21.7; `splot-dfh-sha256-v1` remains repo-owned
  and distinct from AV2 metadata MD5; `SPEC-COVERAGE` does not overclaim
  runtime implementation.

### @encoder-impact-reviewer / Zeno

- Agent ID: `019ec133-ef47-7f10-968e-4b9966957775`
- Findings: none.
- Signoff: confirmed the hash contract helps future encoder closed-loop
  evidence, is versioned, fixes output order/crop/stride/bit-depth/chroma/
  film-grain/metadata policies, and avoids dependency-graph or runtime API
  overreach.

## Archive

- `openspec archive deterministic-frame-hash-contract --yes` synced the
  deterministic decoded-frame hash contract requirement into
  `openspec/specs/decoder-support/spec.md`.
- Remaining unchecked task is PR lifecycle work, which cannot be completed until
  this archived change is committed, pushed, reviewed in GitHub, and merged.
