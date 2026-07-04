# AGENTS.md

Canonical rules for humans and coding agents in this repository. `CLAUDE.md`
and `.github/copilot-instructions.md` point here.

## Behavior

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
  check-duplication` enforces a ratcheting budget
  (`tools/dupehound/budget.toml`) and CI blocks PRs that duplicate existing
  code. Before writing a new function, run `dupehound check` or
  `dupehound scan . --explain <N>` and reuse the original.
- The shortest working diff wins only after the real problem is understood; the
  smallest change in the wrong place is a second bug.
- Question complex requests: ask whether the smaller alternative covers the real
  need.
- When two standard approaches are the same size, choose the edge-case-correct
  one.
- Mark intentional simplifications with a comment when the shortcut has a known
  ceiling, such as a global lock, O(n^2) scan, or naive heuristic. Name the
  ceiling and the upgrade path.

Bug fixes target root cause, not symptoms. For a touched function, inspect its
callers and fix the shared function once when that is the smaller, correct
change. Do not patch only the reported path while leaving sibling callers
broken.

Do not be lazy about understanding the problem, trust-boundary input validation,
error handling that prevents data loss, security, accessibility, real-hardware
calibration, or anything explicitly requested. Non-trivial logic needs one
runnable check that would fail if the logic breaks; trivial one-liners do not.

## Project

`splot` is a Rust toolkit for AV2. It is validator-first: the first useful
milestone is a safe AV2 bitstream validator and inspector.

Toolchain: Rust 1.96.0, edition 2024, resolver 3.

## Crate Boundaries

```text
crates/splot-core      AV2 bitstream model + parsers; no splot-* dependency
crates/splot-parallel  Rayon worker pool + bounded crossbeam queues; no splot-* dependency
crates/splot-tables    dependency-free generated AV2 § 9 tables
crates/splot-recon     reconstruction primitives -> splot-core, splot-tables
crates/splot-decode    decode planning/runtime -> splot-core, splot-parallel, splot-recon
crates/splot-validate  parser-driven diagnostics -> splot-core
crates/splot-encode    encoder API/tools -> splot-core, splot-parallel, splot-recon, splot-tables
crates/splot-cli       thin binary -> core, parallel, decode, validate, encode
xtask                  standalone automation
fuzz                   cargo-fuzz target outside the workspace
```

Nothing depends on `splot-cli`. Nothing depends on `splot-encode` except
`splot-cli`. Enforced by `cargo xtask check-dependency-direction`.

Decoder modules must be named by AV2/decoder domain: bitstream, entropy, tile,
prediction, residual, reference, filters, output, pipeline, support, diagnostic.
Do not add `runtime_minimal`, `runtime2`, `new_runtime`, `misc`, or
fixture-named runtime modules.

## Before Editing

- Run `git status --short`.
- Inspect files you will change.
- Preserve existing user work.
- Non-trivial changes use a stable Feature ID from
  `docs/IMPLEMENTATION-MATRIX.toml`.
- Create or update an OpenSpec change under `openspec/changes/` unless the work
  is trivial.
- Commit subjects and PR titles use Conventional Commits.

## Acceptance

Use `cargo xtask ci` as the gate. Focused commands:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-targets --locked
cargo test --doc --workspace --locked
cargo xtask check-feature-status
cargo xtask check-diagnostic-registry
cargo xtask check-doc-budget
```

Generated status markdown is not committed. Use the xtask render commands in
`README.md` when a status view is needed.

## Coding Standards

- Library-first, thin CLI. Codec and validation logic live in libraries.
- Library errors are typed with `thiserror`; `anyhow` is allowed only in
  `splot-cli` and `xtask`.
- No reachable library `unwrap`, `expect`, `panic!`, `todo!`, or
  `unimplemented!`.
- Use strong types at public boundaries.
- Public items and crates have docs.
- Every `.rs` file starts with the PolyForm SPDX header.
- Rust source files target <=1000 physical lines; hard cap is 2500 unless
  `xtask/src/source_lines.rs` records an allowance.
- `unsafe_code = "forbid"` workspace-wide.

## AV2 and Diagnostics

- Never invent AV2 syntax, constants, tables, or semantics.
- Ground AV2 claims in `docs/spec/av2/1.0.0/`.
- Cite AV2 as `§ N.M` plus the mirror path when a claim needs support.
- The AV2 OBU header is § 5.2.2. Do not use AV1 OBU fields or tables.
- AVM is the differential-testing oracle.
- Findings are structured data with stable `rule_id`, `severity`, optional
  `spec_section`, optional offset, and `message`.

## Encoder Reference Gate

Before changing `crates/splot-encode`, encoder-facing `splot-core`
syntax/parsing code, or encoder research docs, read
`docs/references/THIRD-PARTY-NOTICES.md`. rav1e and SVT-AV1 are engineering
inspiration only; AV2 behavior comes from the AV2 spec and AVM.

## Testing

Testing priority is parser unit tests, property/fuzz no-panic coverage,
`inspect` snapshots, conformance vectors, then AVM differential testing. Parser
changes need positive, negative, and EOF cases. See `docs/TESTING.md`.

## Licensing

Project code, documentation, tests, fixtures, and automation are PolyForm
Noncommercial 1.0.0, with narrow exceptions for generated assistant integrations
and the quarantined AV2 spec mirror. See
`docs/references/THIRD-PARTY-NOTICES.md`.

## Human Sign-Off

Ask before algorithmic encoder choices, ambiguous AV2 spec interpretation,
adding a third-party dependency, changing crate dependency graph, or changing
legal/licensing terms.

## Comment and Documentation Policy

Source comments are rare and high-signal. New codec support extends generic
models, tables, dispatchers, or capability gates rather than one-off branches.
`cargo xtask check-ai-slop`, `cargo xtask check-comment-density`, and
`cargo xtask check-doc-budget` enforce the budget.
