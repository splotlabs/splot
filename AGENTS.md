# AGENTS.md

Canonical entry point for humans and coding agents working in this repository.
`CLAUDE.md` and `.github/copilot-instructions.md` point here; keep this file as
the high-level source of truth and put workflow detail in `docs/agents/`.

Start with [docs/agents/README.md](./docs/agents/README.md) when you need more
than the rules below.

## 0. Unconditional Agent Behavior

Read and execute these rules before any task-specific workflow.

1. Ask, do not assume. If interaction is available and ambiguity materially
   changes intent, architecture, or requirements, ask before writing code. When
   running unattended or when the ambiguity is non-blocking, pick the most
   reasonable interpretation, proceed, and record the assumption.
2. Fit the solution to the problem. Use the simplest solution for simple
   problems and better designs only when the problem justifies them. Do not add
   flexibility that is not needed yet.
3. Keep the diff scoped. Do not touch unrelated code. If you discover bad code
   or design smells outside the task, surface them separately instead of fixing
   them opportunistically.
4. Flag uncertainty explicitly. If a small, local, low-risk experiment would
   clarify the issue, run it and bring the hypothesis and result back for
   discussion. Do not present confidence as certainty.
5. Suggest better paths when they matter. Prefer a durable improvement over a
   tactical patch when the long-term impact is clearly better, and explain the
   tradeoff.

Work like a lazy senior developer: efficient, not careless. The best code is the
code never written. After understanding the task and tracing the real flow end to
end, stop at the first rung that holds:

1. Does this need to be built at all?
2. Does it already exist in this codebase?
3. Does the standard library already do this?
4. Does a native platform feature cover it?
5. Does an already-installed dependency solve it?
6. Can this be one line?
7. Only then, write the minimum code that works.

Lazy rules:

- No abstractions that were not explicitly requested.
- No new dependency if it can be avoided.
- No boilerplate nobody asked for.
- Prefer deletion over addition, boring over clever, and the fewest files
  possible.
- Reuse before reimplementing. Duplicate code is gated: `cargo xtask
  check-duplication` enforces a ratcheting budget (`tools/dupehound/budget.toml`)
  and CI blocks PRs that duplicate existing code. Before writing a new function,
  run `dupehound check` / `dupehound scan . --explain <N>` and reuse the original.
- The shortest working diff wins only after the real problem is understood; the
  smallest change in the wrong place is a second bug.
- Question complex requests: ask whether the smaller alternative covers the real
  need.
- When two standard approaches are the same size, choose the edge-case-correct
  one.
- Mark intentional simplifications with a comment when the shortcut has a known
  ceiling, such as a global lock, O(n²) scan, or naive heuristic. Name the
  ceiling and the upgrade path.

Bug fixes target root cause, not symptoms. For a touched function, inspect its
callers and fix the shared function once when that is the smaller, correct
change. Do not patch only the reported path while leaving sibling callers broken.

Do not be lazy about understanding the problem, trust-boundary input validation,
error handling that prevents data loss, security, accessibility, real-hardware
calibration, or anything explicitly requested. Non-trivial logic needs one
runnable check that would fail if the logic breaks; trivial one-liners do not.

## 1. Project Overview

`splot` is a Rust toolkit for the **AV2** video codec. It is **validator-first**:
the first useful milestone is a safe AV2 bitstream validator and inspector. It is
a solo-developer, source-available project optimized for maintainability, clear
boundaries, and automation.

Toolchain: Rust **1.96.0**, edition **2024**, resolver **3**.

## 2. Repository Boundaries

```text
crates/splot-core      AV2 bitstream model + parsers (no other splot-* dependency)
crates/splot-parallel  approved concurrency primitives (Rayon pool + bounded crossbeam queues); no other splot-* dependency
crates/splot-tables    dependency-free generated AV2 § 9 spec tables shared across crates (no other splot-* dependency)
crates/splot-recon     reconstruction primitives -> splot-core, splot-tables
crates/splot-decode    decode planning, pipeline orchestration, diagnostics, reference/filter/output routing -> splot-core, splot-parallel, splot-recon
crates/splot-validate  parser-driven conformance diagnostics -> splot-core
crates/splot-encode    future encoder API + borrowed input views -> splot-core, splot-parallel, splot-recon, splot-tables
crates/splot-cli       thin `splot` binary -> splot-core, splot-parallel, splot-decode, splot-validate, splot-encode
xtask                  standalone automation
fuzz                   cargo-fuzz target outside the workspace
```

Hard dependency rules:

- `splot-core`, `splot-parallel`, and `splot-tables` depend on no other
  `splot-*` crate.
- `splot-tables` has no external crate dependencies.
- `splot-recon` depends only on `splot-core` and `splot-tables`.
- `splot-decode` depends only on `splot-core`, `splot-parallel`, and
  `splot-recon`.
- `splot-validate` depends only on `splot-core`.
- `splot-encode` depends only on `splot-core`, `splot-parallel`, `splot-recon`,
  and `splot-tables`.
- `splot-cli` depends only on `splot-core`, `splot-parallel`, `splot-decode`,
  `splot-validate`, and `splot-encode`.
- Nothing depends on `splot-cli`.
- Nothing depends on `splot-encode` except `splot-cli`.
- `xtask` is standalone.

These rules are enforced by `cargo xtask check-dependency-direction`; concurrency
and zero-copy details live in [docs/agents/architecture.md](./docs/agents/architecture.md).

Decoder structure guardrail:
Do not create new `runtime_minimal`, `runtime2`, `new_runtime`, `misc`, or
fixture-named runtime modules. Production decode modules must be named by
AV2/decoder domain: bitstream, entropy, tile, prediction, residual, reference,
filters, output, pipeline, support, diagnostic.

## 3. Operating Rules

- Before editing, run `git status --short`, inspect the files you will change,
  and preserve existing user work.
- Every non-trivial change uses a stable Feature ID from
  `docs/IMPLEMENTATION-MATRIX.toml`.
- Create or update an OpenSpec change under `openspec/changes/` unless the work
  is trivial.
- Commit subjects and pull request titles use Conventional Commits.
- Human sign-off triggers are listed in §10.

Details: [docs/agents/workflow.md](./docs/agents/workflow.md).

## 4. Acceptance Commands

Use `cargo xtask ci` as the acceptance gate. It runs formatting, clippy, build,
tests, doctests, rustdoc, run-if-present external checks, and repository gates.

Common focused commands are listed in
[docs/agents/commands.md](./docs/agents/commands.md).

## 5. Coding Standards

- Library-first, thin CLI: codec and validation logic live in libraries.
- Libraries use typed errors with `thiserror`; `anyhow` is allowed only in
  `splot-cli` and `xtask`.
- No runtime panics in libraries: no reachable `unwrap`, `expect`, `panic!`,
  `todo!`, or `unimplemented!`.
- Use strong types at public boundaries, not bare integers.
- Every public item has a doc comment; every crate has `//!` docs.
- Every `.rs` file starts with the required PolyForm SPDX header.
- Rust source files should stay at or below 1000 physical lines; the hard cap is
  2500 lines unless `xtask/src/source_lines.rs` documents an allowance.
- `unsafe_code = "forbid"` across the workspace.

Details: [docs/agents/coding-standards.md](./docs/agents/coding-standards.md).

## 6. AV2 Spec and Diagnostics

- Never invent AV2 syntax, constants, tables, or semantics.
- Ground AV2 claims in the committed spec mirror under `docs/spec/av2/1.0.0/`.
- Cite AV2 sections as `§ N.M` plus the mirror path.
- The AV2 OBU header is § 5.2.2. Do not use AV1 OBU fields or tables.
- Treat AVM as the differential-testing oracle.
- Validator findings are structured data with stable `rule_id`, `severity`,
  optional `spec_section`, optional offset, and `message`.

Details: [docs/agents/av2-spec-and-diagnostics.md](./docs/agents/av2-spec-and-diagnostics.md).

## 7. Encoder Reference Gate

Before changing `crates/splot-encode`, encoder-facing `splot-core`
syntax/parsing code, or encoder research documentation, read the encoder
reference gate in [docs/agents/encoder-reference-gate.md](./docs/agents/encoder-reference-gate.md).

rav1e and SVT-AV1 are engineering inspiration only. AV2 behavior must come from
the AV2 specification and AVM.

## 8. Testing and Audits

Testing priority is parser unit tests, property/fuzz no-panic coverage,
`inspect` snapshots, conformance vectors, then AVM differential testing. Parser
changes need positive, negative, and EOF cases.

Audit procedures are intentionally not expanded here. Use the repo-local audit
skills named in [docs/agents/audits.md](./docs/agents/audits.md).

Details: [docs/agents/testing.md](./docs/agents/testing.md).

## 9. Licensing

Project code, documentation, tests, fixtures, and automation are PolyForm
Noncommercial 1.0.0, with narrow exceptions for generated assistant integrations
and the quarantined AV2 spec mirror.

Details: [docs/agents/licensing.md](./docs/agents/licensing.md).

## 10. Human Sign-Off Triggers

Ask before making algorithmic encoder choices, resolving ambiguous AV2 spec
interpretation, adding a third-party dependency, changing the crate dependency
graph, or changing legal/licensing terms.

## 11. AI-Slop and Comment Policy

Source comments are rare and high-signal; new codec support extends generic
models, tables, dispatchers, or capability gates rather than adding one-off
branches. Enforced by `cargo xtask check-ai-slop` (banned history/diary phrases,
hard zero) and `cargo xtask check-comment-density` (implementation-comment
budget).

Details: [docs/agents/coding-standards.md](./docs/agents/coding-standards.md).
