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
5. Suggest better paths when they matter. Prefer a durable improvement over a
   tactical patch when the long-term impact is clearly better, and explain the
   tradeoff.

Work like a lazy senior developer: efficient, not careless. After understanding
the real flow, stop at the first rung that holds: existing code, standard
library, native platform feature, installed dependency, one-line fix, or the
minimum new code that works.

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

Toolchain: stable Rust 1.99.1, edition 2024, resolver 3. Fuzzing uses nightly.

## Crate Boundaries

```text
crates/splot-core      AV2 bitstream model + parsers; no splot-* dependency
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
- Use strong types at public boundaries.
- Public items and crates have docs.
- Every `.rs` file starts with the PolyForm SPDX header.
- Rust source files target <=1000 physical lines; hard cap is 2500.
- `unsafe_code = "forbid"` workspace-wide, except the reviewed blocks in
  `splot-decode`, the `splot-recon` plane allocator and the `splot-capi` C ABI
  (`deny` plus module `allow`).

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

The decoder must be faster than dav2d on every stream and at every thread
count. dav2d is inspiration only: do not copy its code.

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
- Measure first. Keep a change only if it is a repeatable win with no
  regression above 0.1% on another stream. Prefer changes that delete work and
  delete code; glue around kernels (per-block setup, copies, dispatch in loops)
  is a defect.
- Kernel and single-thread changes: compare instructions retired at
  `--threads=1` (`/usr/bin/time -l`). Scheduling changes: compare wall time
  with alternating runs and report the range.
- Output stays byte-exact. Compare `decode --output-format hash` with `main`
  on the EwoutH/AV2 samples, 8-bit and 10-bit, at `--threads=1` and
  `--threads=12` (also an odd count for scheduling changes).
- Each SIMD version has a unit test against the oracle on random and extreme
  inputs. x86_64 code must pass clippy with `--target x86_64-apple-darwin`
  (or another x86_64 target) when it cannot run on the host.
- An arch-specific kernel uses safe `core::arch` intrinsics. A new `unsafe`
  block needs human sign-off.
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

Source comments are rare and high-signal. New codec support extends generic
models, tables, dispatchers, or capability gates rather than one-off branches.
`cargo xtask check-ai-slop`, `cargo xtask check-comment-density`, and
`cargo xtask check-doc-budget` enforce the budget.
