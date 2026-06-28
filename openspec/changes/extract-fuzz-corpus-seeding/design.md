# Design: extract-fuzz-corpus-seeding

## Context

The fuzz-smoke job is a matrix with one leg per registered fuzz target; each leg
seeds its corpus then runs `cargo fuzz run` for a short slice. The seeding was a
single ~100-line `run:` block of shell plus two inline `python3 -c` scripts. Each
fuzz target consumes a small leading config prefix before the bitstream, so the
seeds prepend the right prefix to a fixture or conformance vector, embed a few
hand-tuned minimal payloads, synthesize IVF wrappers, and de-wrap IVF frames into
raw OBU streams.

## Data model / API

`xtask::seed_fuzz_corpus`:

- `run_seed_fuzz_corpus(root)` — enumerates targets via `crate::fuzz_targets(root)`
  and calls `seed_corpus`, then prints a one-line summary.
- `seed_corpus(root, targets) -> SeedSummary` — orchestrates: create every
  target's corpus dir + copy each fixture in; write the static seeds; write the
  per-fixture and per-conformance-vector derivatives.
- Pure, unit-tested helpers:
  - `prefixed(prefix, body)` — config bytes then bitstream.
  - `ivf_wrap(data)` — 32-byte IVF header + one 12-byte frame header + data,
    reproducing the former `struct.pack("<4sHH4sHHIIII", …)` / `struct.pack("<IQ", …)`.
  - `ivf_dewrap(data) -> Option<Vec<u8>>` — concatenate frame payloads, reading the
    header length from bytes `[6:8]`; `None` for inputs shorter than 32 bytes.
- `Task::SeedFuzzCorpus` clap subcommand. The task is **not** wired into
  `run_ci()` — seeding is only needed by the fuzz job; the `ci` job covers the
  logic through `cargo test -p xtask`.

## Why xtask, invoked per fuzz leg (not an artifact)

The maintainer's call: keep the logic in a unit-tested Rust subcommand, but do not
share the corpus across the matrix via a CI artifact, because artifact storage is
billed. So each fuzz leg runs `cargo xtask seed-fuzz-corpus`. The per-leg xtask
build is marginal: each leg already compiles the full instrumented workspace to
build its fuzz target, so the cargo registry is downloaded regardless and an extra
stable xtask build is incremental. The fuzz job installs the repo's pinned stable
toolchain so the xtask build is deterministic.

## Why reuse `fuzz_targets()`

The former first loop used `cargo +nightly fuzz list`. `fuzz_targets()` already
derives the same set from `fuzz/fuzz_targets/*.rs` and asserts it equals the
`[[bin]]` entries (the `check-fuzz-targets` gate). Reusing it avoids a nightly
dependency in the seeding step and avoids duplicating the enumeration.

## Byte-parity guarantee

The produced corpus is byte-identical to the former inline script. Verified by
running the original shell/Python block and the new subcommand into clean corpus
trees and diffing: `diff -r` over all 974 seed files reported no difference. The
in-repo regression guard is the `seed_corpus_writes_byte_exact_seeds` test plus
the per-helper byte-layout tests.

## Spec mapping

None. Non-normative repository tooling; it adds no AV2 conformance behavior.
Captured as the `tooling` capability.

## Diagnostics

None (no validator diagnostics; this is a build-time corpus generator).

## Tests

- `xtask/src/seed_fuzz_corpus.rs::tests`: `ivf_wrap` header/frame layout,
  `ivf_dewrap` single/multi-frame round-trip and short-input rejection, `prefixed`,
  and an end-to-end `seed_corpus` over a temp root asserting exact seed bytes
  (raw copies, prefixed copies, static seeds, IVF wrap, conformance naming, and
  de-wrapped OBU output).
- `cargo xtask seed-fuzz-corpus` runs in each CI fuzz leg.

## Alternatives considered

- A standalone `scripts/seed_fuzz_corpus.py`: rejected — it would not join the
  `cargo test` gate, and the repo is xtask-first.
- A dedicated pre-matrix seed job uploading the corpus as an artifact: rejected by
  the maintainer — artifact storage is billed; the per-leg command avoids it.
- Per-target seeding (`--target <name>`): not needed — seeding the whole corpus in
  each leg matches the former behavior exactly and keeps the command argument-free.

## Risks

- Spec ambiguity: none (non-normative).
- Drift: a new fuzz target that needs a bespoke seed must be added to
  `seed_static`/`seed_fixtures`; the generic per-target fixture copy already
  covers any new target with no edit. The byte-exact test guards the existing
  layouts.
- CI time: a marginal per-leg xtask build (see above); no artifact storage cost.
