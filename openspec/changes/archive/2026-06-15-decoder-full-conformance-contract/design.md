# Design: decoder-full-conformance-contract

## Context

The full decoder mission starts from a green baseline but not from a runtime
decoder. `splot decode` currently plans bytes and emits structured diagnostics;
it does not decode tile payloads, reconstruct pixels, compute runtime hashes, or
write Y4M output. The Step 0 audit in `docs/DECODER-FULL-CONFORMANCE-GAP-AUDIT.md`
also shows that many AV2 v1.0.0 decode-relevant section families do not have an
explicit runtime decoder owner row.

The first decoder-program PR therefore needs to make conformance measurable,
not implement codec behavior. It must define the public claim, enumerate the
spec coverage surface, and add a self-contained check that fails when generated
coverage docs drift or when invalid coverage states are recorded.

Two pre-existing active OpenSpec changes remain outside this decoder track:
`avm-differential-harness` is proposed and conflicts with the new mission's
no-AVM/dav2d-integration rule, while `toy-intra-encoder-v0` is explicitly
parked encoder work. This change must not depend on either.

## Goals / Non-Goals

**Goals:**

- Define `docs/DECODER-FULL-CONFORMANCE.md` as the source for the future public
  decoder conformance claim, current non-claim, output variants, diagnostics,
  reference-evidence boundary, and final completion criteria.
- Add a generated `docs/DECODER-SPEC-COVERAGE.md` table that maps AV2 v1.0.0
  decoder-relevant section families to an implementation owner, status, tests,
  fuzz targets, diagnostics, and local-reference evidence.
- Add `cargo xtask check-decoder-conformance-coverage` and wire it into
  `cargo xtask ci`.
- Add decoder support matrix rows for the full conformance contract and the
  coverage gate, with status backed only by self-contained docs/tooling tests.
- Keep all coverage statuses honest: unsupported and partial rows are allowed,
  but no runtime decoder feature may become `supported` from this contract work
  alone.

**Non-Goals:**

- No runtime tile decode, symbol/CDF completion, reconstruction, reference
  update, film grain, loop filtering, hash success output, Y4M output, or raw
  output implementation.
- No AVM/dav2d code, wrappers, `xtask` invocation, scripts, CI jobs, setup
  instructions that make them mandatory, or local path probes.
- No crate dependency graph changes and no public Rust API changes.
- No new third-party dependencies.

## Decisions

### Coverage is a decoder-specific document, not the global spec coverage

`docs/SPEC-COVERAGE.md` already summarizes global feature tracking from
`docs/IMPLEMENTATION-MATRIX.toml`. Full decoder conformance needs a different
question: for each decode-relevant AV2 section family, who owns runtime decoder
coverage, what is the status, and what proof exists?

This change adds `docs/DECODER-SPEC-COVERAGE.md` instead of overloading the
global feature-status renderer. The new document is generated from a small
decoder coverage data model in `xtask`, cross-referenced with
`docs/DECODER-SUPPORT-MATRIX.toml` and local-reference evidence where rows name
them.

Alternative considered: make every spec section a row in
`docs/DECODER-SUPPORT-MATRIX.toml`. Rejected for this first contract because the
support matrix already tracks implementation slices, not a full spec index, and
forcing hundreds of rows before a renderer exists would create hard-to-review
manual churn.

### Coverage rows are section families with exact citations

The first generated coverage document groups related AV2 sections into
reviewable section families, such as Section 4 descriptors, Sections 5.18/6.17
frame header state, Sections 7.14-7.20 reconstruction/filtering, Section 8
symbol/CDF process, Section 9 normative tables, Annex A, Annex B, and Annex E.
Each row records exact `spec_sections` and notes what remains unsupported.

Alternative considered: one row for every `docs/spec/av2/1.0.0/index.md`
entry. Rejected for this PR because a family-level gate is enough to prevent
silent no-owner gaps and keeps the first review focused; later changes may
expand the generator to finer-grained rows.

### Status vocabulary is intentionally stricter than runtime claims

Decoder conformance coverage rows use:

- `unsupported`
- `partial`
- `supported`
- `blocked`
- `out-of-scope-nonnormative`

`supported` requires self-contained tests and, for runtime decode claims,
runtime evidence rather than parser-only evidence. This contract may mark docs
and tooling rows supported, but the generated decode section coverage will still
show many `unsupported` and `partial` rows.

Alternative considered: reuse `unsupported-intentional` from the support matrix.
Rejected because the final full-conformance target must drive temporary
unsupported decode rows to zero or move only nonnormative material out of scope.

### External reference evidence stays metadata-only

The coverage gate may validate that a row names local-reference evidence already
present in `docs/LOCAL-REFERENCE-EVIDENCE.toml`, but it must not run AVM, dav2d,
ffmpeg, or any local reference command. The existing live
`avm-differential-harness` proposal is not part of this decoder mission.

Alternative considered: add an optional local live check behind an environment
variable. Rejected because the mission explicitly forbids repository wrappers or
`xtask` commands that invoke external decoders.

### The gate starts as drift and honesty enforcement

The new `check-decoder-conformance-coverage` command should fail when:

- the generated `docs/DECODER-SPEC-COVERAGE.md` differs from committed output;
- a row uses an unknown status;
- a row names a missing decoder support row or local-reference evidence id;
- a row marked `supported` has no self-contained test or proof reference;
- a row marked `out-of-scope-nonnormative` lacks a note explaining why it is not
  required for decoder conformance.

It should not fail merely because many decode sections are currently
`unsupported` or `partial`; that is the honest baseline this change is meant to
expose.

## Risks / Trade-offs

- Broad coverage rows can hide sub-section detail. Mitigation: rows must name
  exact spec sections and notes, and future implementation changes can split
  rows as they add runtime owners.
- A docs/tooling PR can be mistaken for runtime progress. Mitigation: the full
  conformance document and support matrix rows must explicitly state that no
  codec feature is implemented by this change.
- The new gate may duplicate some support-matrix checks. Mitigation: keep
  `check-decoder-support` as the matrix schema/render gate and make the new gate
  responsible only for decoder conformance coverage rendering and cross-links.
- Pre-existing active non-decoder OpenSpec changes remain visible in
  `openspec list`. Mitigation: this change records the conflict and does not
  depend on them; the stale AVM live-harness proposal should be retired or
  re-proposed separately under the new no-integration rule.

## Review Notes

- Spec-mapping review required explicit coverage dimensions for normative,
  informative, and mixed rows, with notes for nonnormative out-of-scope
  material.
- Decoder-architecture review kept the scope to docs/tooling and required the
  coverage gate to run after decoder-support checks in CI.
- Security/reference-evidence review required metadata-only local-reference
  evidence validation and no AVM/dav2d repository integration, wrappers, CI jobs,
  or local path probes.
- Correctness re-review passed after the generated coverage included AV2
  Section 8.1 and Annex A.3, the gate rejected false `supported` claims whose
  linked decoder support rows are not supported, and negative tests covered
  missing support rows, missing evidence ids, and unsupported-row visibility.
