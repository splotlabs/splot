---
name: splot-av2-conformance-audit
description: Run the heavy splot AV2 conformance audit. Use when asked to audit implementation, validator, encoder, decoder, writer, inspector, conformance, fuzzing, automation, or docs against the AV2 spec and implementation matrix, especially with changed-file or multi-reviewer scope.
license: PolyForm-Noncommercial-1.0.0
---

# splot AV2 Conformance Audit

Use this skill for `XTASK-AUDIT-SCOPE` and `DOC-AUDIT-PROTOCOLS` heavy AV2
spec-fidelity audits. The audit finds issues; it does not silently fix codec
behavior.

## First Step: Deterministic Scope

Run audit-scope before reading files deeply:

```bash
cargo xtask audit-scope --format json
```

Use `--base <rev>` for PR/diff mode, `--all` for deliberate full passes, and
`--write-ledger` only after a completed scheduled audit whose state should be
persisted. If `cargo` is not on `PATH`, locate the pinned Rust toolchain and run
the same xtask command through it.

Trust the command for candidate selection. It discovers workspace members and
in-scope paths dynamically, so future encoder, decoder, writer, inspector,
conformance, fuzzing, and automation files are included without hardcoded crate
names.

## Required Context

For each candidate, read only the needed context:

- `AGENTS.md`
- `docs/IMPLEMENTATION-MATRIX.toml`
- `docs/CONFORMANCE.md` and `docs/DIAGNOSTICS.md` when relevant
- relevant `openspec/changes/` artifacts
- relevant AV2 mirror sections via `docs/spec/av2/1.0.0/index.md`
- the candidate file and directly related code/tests

Treat the committed AV2 mirror as read-only evidence. Cite section numbers and
mirror paths; do not copy long spec text into reports or source comments.

## Reviewer Lanes

The coordinator maps each candidate to the lanes emitted by audit-scope and may
add lanes when needed:

- `spec-citation`: every AV2 syntax, constant, table, semantic, layer, and
  reconstruction claim is grounded in the mirror or marked with a known
  `TODO(spec: <FEATURE-ID>)`.
- `parser-safety`: untrusted input returns typed errors, never panics, hangs, or
  allocates unboundedly.
- `encoder-decoder-writer-inspector`: future encoder/decoder/writer/inspector
  behavior is AV2-derived, not copied from AV1 projects; reference projects are
  inspiration only and require the encoder reference gate.
- `validator-diagnostics`: diagnostics have stable `rule_id`, severity,
  applicable `spec_section`, offset when known, and clear messages.
- `feature-matrix-openspec`: Feature IDs, matrix status, proof, and OpenSpec
  artifacts match the code and tests.
- `tests-fuzz-conformance`: parser changes include positive, negative, and EOF
  cases; no-panic/property/fuzz/conformance proof is recorded when relevant.
- `safety-boundaries`: dependency direction, library/CLI split, no unsafe code,
  no runtime panics, typed errors, and licensing boundaries hold.
- `agent-guidance` and `automation`: skills, prompts, workflows, and xtask code
  preserve the same rules and do not create hidden mutable state.

When the current agent environment and the user or scheduler explicitly authorize
parallel agent work, spawn lane-specific reviewers with disjoint read scopes and
merge their findings. Otherwise run the lanes sequentially.

## Finding Rules

Report only findings grounded in repo evidence. For each finding include:

- severity: blocker, important, or nit
- candidate path and line when available
- affected Feature ID or "unknown"
- AV2 section or source-of-truth doc when applicable
- concrete impact
- recommended next action

If AV2 interpretation is ambiguous, record "human required" and stop short of a
behavior change. Do not invent syntax, constants, tables, semantics, or rule IDs.

## Fix Policy

By default, produce an audit report or follow-up issues/OpenSpec changes. Do not
modify parser, validator, encoder, decoder, writer, or inspector behavior unless
the user explicitly asks for a fix after seeing the finding.

Small audit-protocol or ledger updates are allowed when they are the requested
task. Never auto-merge.

## Completion

If the scheduled audit completed and the user wants persistent state, rerun:

```bash
cargo xtask audit-scope --write-ledger --format json
```

Review the ledger diff. It must be deterministic, list audited file hashes, and
record the audit outcome.
