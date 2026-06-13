## Context

The decoder roadmap permits AVM and dav2d only as local development oracles.
Evidence from those tools may be useful for future tiny decoder fixtures and
hash comparisons, but it must stay portable metadata: no repo code, CI job,
script, wrapper, dependency, or `xtask` command may locate, build, or run
external decoders.

Today, local-reference evidence is recorded as prose in agent logs or
decoder-support matrix notes. That is enough for historical context, but not
enough for future committed fixture evidence because it cannot be validated for
local paths, stale fixture hashes, or accidental executable semantics.

Dependency-graph changes for `splot-recon` / `splot-decode` remain unapproved,
so this change stays in docs, OpenSpec, and standalone `xtask` automation.

## Goals / Non-Goals

**Goals:**

- Add a versioned TOML manifest contract for local-reference evidence metadata.
- Add a pure metadata checker that can run on a machine with no AVM, dav2d,
  ffmpeg, network access, or decoder checkout.
- Validate repo-relative fixture paths, fixture SHA-256 / byte lengths,
  evidence IDs, feature IDs, decoder-support row IDs, digest fields, assertions,
  and local-path leakage.
- Wire the checker into the existing decoder-support gate so reference evidence
  metadata is checked with the decoder support matrix.
- Document the manifest as non-executable evidence that does not prove current
  `splot decode` support.

**Non-Goals:**

- No `splot-recon` or `splot-decode` crate.
- No Cargo manifest, dependency graph, source dependency, build dependency,
  script, wrapper, `build.rs`, CI job, Docker image, or external decoder runner.
- No AVM/dav2d/ffmpeg invocation, discovery, local path probing, or network
  access.
- No new decoder fixtures or fresh local reference evidence.
- No runtime decode, reconstruction, deterministic hash computation, Y4M output,
  or new emitted decoder diagnostic.
- No claim that local-reference metadata proves AV2 decoder conformance or
  current `splot decode` behavior.

## Decisions

1. Use a separate structured TOML manifest.

   Free-form `local_reference_evidence` strings are useful for row summaries,
   but future decoder evidence needs stable IDs, fixture identity, tool
   revisions, digest fields, and comparison assertions. The manifest path is
   `docs/LOCAL-REFERENCE-EVIDENCE.toml` so the file is clearly documentation and
   not a test runner or fixture corpus.

2. Validate the manifest from a new `xtask/src/reference_evidence.rs` module.

   `xtask` already has `serde`, `toml`, and `sha2`, so no dependencies are
   needed. The module stays separate from `decoder_support.rs` to keep the
   support-matrix checker focused and to avoid turning the decoder-support
   renderer into a broad manifest parser.

3. Wire validation through `cargo xtask check-decoder-support`.

   Local-reference evidence is part of decoder-support documentation, and the
   existing decoder-support gate is already in `cargo xtask ci`. Calling the new
   checker from that gate avoids a separate required command while still keeping
   implementation in its own module.

4. Keep command fields descriptive, not executable.

   Manifest entries may record sanitized command summaries such as
   `avmdec {fixture:input} as raw decoder output`, but the checker rejects local
   absolute paths, `file://`, home-relative paths, environment-variable path
   tokens, executable paths, and shell metacharacters. The command summary is
   evidence prose, not a runnable recipe.

5. Treat AV2 citations as evidence-scope metadata only.

   The manifest envelope has no AV2 section. Future decoded-output hash entries
   can cite § 6.16.13 sample serialization and § 7.21 output processing, while
   AV2 metadata decoded-frame-hash syntax (§ 5.17.12) is only relevant when an
   entry explicitly records metadata-hash interop fields.

## Risks / Trade-offs

- Overclaiming local evidence -> The manifest and docs state that entries are
  non-executable metadata and do not prove current runtime decode support.
- Checker becoming an external runner -> The checker only reads committed files
  and parses TOML; no `Command`, network, external decoder lookup, or wrapper
  semantics are added.
- Empty initial manifest looks weak -> The first change establishes and gates
  the schema. Future fixture/evidence PRs can add real entries once runtime
  decoder milestones produce meaningful evidence.
- Strict path validation may reject useful prose -> Local-reference metadata is
  intentionally portable; machine-specific paths belong in uncommitted notes,
  not committed manifests.
