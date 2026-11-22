# AGENTS.md

Canonical rules for humans and coding agents in this repository. `CLAUDE.md`
and `.github/copilot-instructions.md` point here.

## Behavior

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

Work like a lazy senior developer: efficient, not careless. After understanding
the real flow, stop at the first rung that holds: existing code, standard
library, native platform feature, installed dependency, one-line fix, or the
minimum new code that works.

The rungs choose how to build the right design; they do not justify a hack,
workaround or patch in place of it. Aim for the best solution, even when it is
a larger restructure, and state the effort of each option as information.

Lazy rules:

- No abstractions that were not explicitly requested.
- No new dependency if it can be avoided.
- No boilerplate nobody asked for.
- Prefer deletion over addition, boring over clever, and the fewest files
  possible.
- Reuse before reimplementing. `cargo xtask check-duplication` enforces the
  duplicate-code budget; check before adding a new function.
- The shortest working diff wins only after the real problem is understood; the
  smallest change in the wrong place is a second bug.

Reduction and cleanup work must delete more complexity than it adds. Do not add
new xtask commands, budget ratchets, status-matrix rows, generated reports, or
process documentation just to justify a cleanup. Use existing gates and report
measurements in the PR or final note unless the user explicitly asks for a new
enforced policy. Do not move code or tests merely to game line-count metrics;
split files only when the resulting ownership or readability is better on its
own.

Bug fixes target root cause, not symptoms. For a touched function, inspect its
callers and fix the shared function once when that is the smaller correct
change.

Do not be lazy about understanding the problem, trust-boundary input validation,
error handling that prevents data loss, security, accessibility, real-hardware
calibration, or explicit requests. Non-trivial logic needs one runnable check;
trivial one-liners can rely on existing coverage.

## Project

`splot` is a Rust AV2 toolkit. The first useful milestone is a safe AV2
bitstream validator and inspector.

Toolchain: stable Rust 1.100.0, edition 2024, resolver 3. Fuzzing uses nightly.

## Crate Boundaries

```text
crates/splot-core      AV2 bitstream model + parsers -> splot-tables
crates/splot-parallel  Rayon worker pool; no splot-* dependency
crates/splot-tables    dependency-free generated AV2 § 9 tables
crates/splot-recon     reconstruction primitives -> splot-core, splot-tables
crates/splot-decode    decode planning/runtime -> splot-core, splot-parallel, splot-recon
crates/splot-validate  parser-driven diagnostics -> splot-core
crates/splot-encode    encoder API/tools -> splot-core, splot-parallel, splot-recon, splot-decode
crates/splot-cli       thin binary -> core, parallel, decode, validate, encode
crates/splot-capi      C ABI of the streaming decoder -> core, decode
xtask                  standalone automation
fuzz                   cargo-fuzz target outside the workspace
```

Nothing depends on `splot-cli` or `splot-capi`. `splot-cli` uses `splot-encode` for the
`encode` command, and the out-of-workspace fuzz crate tests its APIs.

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
- Commit subjects and PR titles use Conventional Commits.

## Autonomous Work

The owner turns autonomous mode on in the task ("autonomous", "take it to
merge") or by handing over a GitHub issue. Without it, a commit, push, PR or
merge needs the task to name it.

In autonomous mode, do the whole path without asking or reporting progress:
branch, implement, commit, run the gate and a code review, push, open the PR
(linked to its issue, if any), address every review comment (fix it, or reply
with the reason it stays), and merge when CI is green. Diagnose failing checks
and conflicts, and continue.

Stop only for a decision that is the owner's:

- The requirement is unclear.
- The best solution is a larger restructure than the task implies, or several
  sound designs exist.
- A rule in this file says to ask or needs human sign-off.
- The action is destructive or cannot be undone.

Ask once, with the options, their pros, cons and effort, and a recommendation.
Continue the work that does not depend on the answer. Never reach the merge
with a hack, workaround, weakened check, skipped test or dismissed comment.

## Acceptance

Use `cargo xtask ci` as the gate; GitHub CI runs it in full. Locally, run
`cargo xtask ci --fast` (fmt, clippy, repo checks) plus the tests of the crates
you touched. Focused commands:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-targets --locked
cargo test --doc --workspace --locked
cargo xtask check-feature-status
cargo xtask check-diagnostic-registry
cargo xtask check-doc-budget
```

Generated status markdown is not committed; render it on demand.

## Coding Standards

- Library-first, thin CLI. Codec and validation logic live in libraries.
- Library errors are typed with `thiserror`; `anyhow` is allowed only in
  `splot-cli` and `xtask`.
- No reachable library `unwrap`, `expect`, `panic!`, `todo!`, or
  `unimplemented!`.
- Do not satisfy the panic lints with a silent fallback. `unwrap_or`,
  `unwrap_or_default`, `.ok()` or `else { return }` on a value that came from
  the bitstream returns a typed error; a fallback that cannot fire is deleted.
- Use strong types at public boundaries: an enum for a closed set, a struct
  for values that travel together, `Option` for absence. No string, tuple or
  sentinel stands in for them.
- One type and one function per concept. Reuse the type or spec derivation of
  the lowest crate that owns it; do not mirror it and convert.
- An item that only tests call is `#[cfg(test)]`, not `pub`.
- Public items and crates have docs.
- Every `.rs` file starts with the PolyForm SPDX header.
- Rust source files target <=1000 physical lines; hard cap is 2500.
- `unsafe_code = "forbid"` workspace-wide, except the reviewed blocks in
  `splot-decode`, the `splot-recon` plane allocator and the `splot-capi` C ABI
  (`deny` plus module `allow`).
- Waive a lint with `#[expect(lint, reason = "...")]` on the smallest item;
  delete an expectation that is no longer fulfilled. `#[allow]` is only for a
  waiver that cannot hold in every build: target- or cfg-gated code, macro
  bodies, generated files and the `unsafe_code` module boundaries.
- Test code needs no waiver for `unwrap`, `expect` or `panic!`: `clippy.toml`
  covers `#[test]` functions and items under `#[cfg(test)]`. Helpers in
  `crates/*/tests/` and items under `cfg(all(test, ...))` still need one.

## Rust and C Decoder API

`splot-capi` mirrors the streaming decoder (`Decoder`, `DecoderSettings`,
`Picture`) in dav1d/dav2d shape: open, send, get, flush, drain, close and
plain structs; no demuxing or file access.

- A PR that changes public API on those types makes the same change in
  `splot-capi` (`splot.h`, an `abi.rs` test), or says why a C host does not
  need it. When unsure, leave out what dav2d does not expose.
- New struct fields take reserved space; sizes and existing offsets stay.

## AV2 and Diagnostics

- Never invent AV2 syntax, constants, tables, or semantics.
- Ground AV2 claims in `docs/spec/av2/1.0.0/`.
- Cite AV2 as `§ N.M` plus the mirror path when a claim needs support.
- The AV2 OBU header is § 5.2.2. Do not use AV1 OBU fields or tables.
- AVM is the differential-testing oracle.
- Findings are structured data with stable `rule_id`, `severity`, optional
  `spec_section`, optional offset, and `message`.

## Encoder Reference Gate

Before changing encoder-facing code or research docs, read
`docs/references/THIRD-PARTY-NOTICES.md`. rav1e and SVT-AV1 are engineering
inspiration only; AV2 behavior comes from the AV2 spec and AVM.

## Testing

Testing priority is parser unit tests, property/fuzz no-panic coverage,
`inspect` snapshots, conformance vectors, then AVM differential testing. Parser
changes need positive, negative, and EOF cases. See `docs/TESTING.md`.

## Performance and SIMD

Decode speed is a product requirement, on every stream and at every thread
count. Other decoders are inspiration only: do not copy their code.

Kernel shape. Each hot kernel has exactly one of these two forms:

- One `fearless_simd` kernel, generic over the sample type (`u8` and `u16`)
  where that costs neither bit depth anything. This is the default.
- Three versions, only when a measurement shows that the generic kernel is not
  the fastest: aarch64 NEON, x86_64 (AVX2 baseline, `x86-64-v3`), and the
  generic `fearless_simd` kernel as the fallback. aarch64 and x86_64 have
  priority.

Rules:

- No scalar sample-processing path at runtime: not as a twin of a SIMD kernel,
  not for tails, borders, odd sizes or rare modes. Cover those cases in the
  kernel (padded loads, masked or overlapping stores) and delete the scalar
  code. Scalar sample code is allowed only in `#[cfg(test)]`, as the oracle
  for correctness checks.
- 8-bit and 10-bit are equal targets. A change that makes one bit depth faster
  must not leave the other on a slower path. Measure both.
- Measure first. Keep a change that is a repeatable net win over the stream
  set; aim for no regression, list each stream that loses, and give a reason
  for a loss above about 0.5%. Prefer changes that delete work and code; glue
  around kernels (per-block setup, copies, dispatch in loops) is a defect.
- Kernel and single-thread changes: compare instructions retired at
  `--threads=1` (`/usr/bin/time -l`). Scheduling changes: compare wall time
  with alternating runs and report the range.
- Output stays byte-exact. Compare `decode --output-format hash` with `main`
  on the EwoutH/AV2 samples, 8-bit and 10-bit, at `--threads=1` and
  `--threads=12` (also an odd count for scheduling changes).
- Each SIMD version has a unit test against the oracle on random and extreme
  inputs. x86_64 code must pass clippy with `--target x86_64-apple-darwin`
  (or another x86_64 target) when it cannot run on the host.
- An arch-specific kernel uses safe `core::arch` intrinsics.
- Do not tune by fixed caps or per-stream heuristics. Fix the root cause.

## Licensing

Project code, docs, tests, fixtures, and automation are PolyForm Noncommercial
1.0.0, except the quarantined AV2 spec mirror. See
`docs/references/THIRD-PARTY-NOTICES.md`.

## Human Sign-Off

Ask before algorithmic encoder choices, ambiguous AV2 spec interpretation,
adding a third-party dependency, changing crate dependency graph, adding an
`unsafe` block, or changing legal/licensing terms.

## Comment and Documentation Policy

Source comments are rare and high-signal. A comment states what is true of the
code now: no history, ticket ids, milestones or "now"/"previously" wording. A
`TODO(spec: FEATURE-ID)` is one complete sentence. New codec support extends generic
models, tables, dispatchers, or capability gates rather than one-off branches.
`cargo xtask check-ai-slop`, `cargo xtask check-comment-density`, and
`cargo xtask check-doc-budget` enforce the budget.
