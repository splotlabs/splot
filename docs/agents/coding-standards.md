# Agent Coding Standards

Use this file for implementation rules that apply across Rust crates. The review
checklist is [../CODE_REVIEW.md](../CODE_REVIEW.md).

## Library First

`splot-cli` parses arguments, sets up logging, reads/writes files, and calls
library APIs. Codec logic, validation logic, parsing, and diagnostics live in
library crates.

## Error Handling

- Libraries use typed errors with `thiserror`.
- `anyhow` is allowed only in `splot-cli` and `xtask`.
- Malformed input returns errors or diagnostics; it must not panic.
- Recognized but unimplemented functionality returns
  `Error::Unimplemented { feature }` or a structured `Diagnostic`.

## Panic Policy

No `unwrap`, `expect`, `panic!`, `todo!`, or `unimplemented!` may be reachable in
library code.

Tests may use `unwrap` or `expect` only inside test-only code with the local
allowance pattern described in [../TESTING.md](../TESTING.md).

## Types and Public API

- Use newtypes and enums at public boundaries, for example `ObuType`,
  `TemporalLayerId`, and `ByteOffset`.
- Avoid bare integers in public APIs when a domain type exists or should exist.
- Every public item has a doc comment.
- Every crate has crate-level `//!` docs.

## SPDX Headers

Every `.rs` file starts with:

```rust
// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com>
```

Enforcement:

```bash
cargo xtask check-license-headers
```

## Comment Diet

Comments explain invariants or exceptions, not normal control flow. Prefer names,
types, and smaller functions over prose that restates the next statement.

Use short AV2 section anchors instead of copying spec prose into source. If the
explanation needs more than a short anchor, move it to a design doc or ADR and
link it only where needed.

Public Rustdoc must be minimal but complete enough for `missing_docs` and public
API users. Avoid historical notes, implementation chronology, and long spec
quotations in Rustdoc.

Historical explanations belong in ADRs or design docs, not source comments. AI
agents must not add comments containing filler or process-history language such
as "this helper", "this function", "now", or "former". For the enforced
process-history and fixture-diary phrase list — and the domain-vocabulary
carve-outs (e.g. "previously decoded", round-trip) — see the AI-Slop section.

Enforcement:

```bash
cargo xtask check-comment-density
```

## AI-Slop

Beyond the comment diet, two patterns are banned outright.

**Banned comment phrases** (process history and development diary) — enforced at
zero by `cargo xtask check-ai-slop`:

- process history: `formerly`, `round-<n>`, `PR #<n>`, `this refactor`,
  `this used to`, `used to be`, `old behavior`/`old behaviour`;
- fixture diary: `oracle fixture`, `verified fixture`, `pinned by fixture`,
  `not yet fixtured`.

Replace each with a current invariant, a capability description, or a short
`§ N.M` spec anchor. The gate skips generated files and leaves established
domain vocabulary alone (for example the decoder partition `frontier` and AV2
"previously decoded" spec language); extend the phrase list as new slop appears.

**Banned code shape** (one-off branches). New codec support extends a generic
model, table, dispatcher, capability gate, or shared helper. Do not add a branch
for a single fixture, transform, block shape, MV case, prediction angle,
bit-depth, or neighbour position unless the AV2 algorithm has a real special
case and the branch name says so. Report missing support as a capability, not a
fixture story.

Allowed comments are unchanged from the comment diet: license or generated-file
headers, minimal public Rustdoc (`# Errors` / `# Panics` / `# Safety` contracts),
a non-obvious invariant, a short spec anchor, an external workaround, a stable
diagnostic note, or a `TODO`/`FIXME` carrying a tracker.

Enforcement:

```bash
cargo xtask check-ai-slop
```

## Source Size

Rust source files should stay at or below 1000 physical lines. The hard cap is
2500 lines unless `xtask/src/source_lines.rs` records a temporary allowance and
rationale.

Enforcement:

```bash
cargo xtask check-source-lines
```

Split large files by responsibility before adding more code. Do not grow an
allowlisted file past its recorded cap without deliberately updating the
allowance and rationale.

## Unsafe and SIMD

`unsafe_code = "forbid"` across the workspace. Future SIMD or FFI must live
behind narrowly scoped, documented, tested modules and requires maintainer
approval.

## Concurrency and Zero-Copy

For concurrency and media-buffer ownership, follow
[architecture.md](./architecture.md), [../CONCURRENCY.md](../CONCURRENCY.md),
and [../ZERO_COPY.md](../ZERO_COPY.md).
