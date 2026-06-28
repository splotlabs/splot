# Change: extract-fuzz-corpus-seeding

## Feature IDs

- `INFRA-FUZZ-CORPUS-SEEDING`

## Why

The CI fuzz-smoke job seeds each target's `fuzz/corpus/<target>/` directory from
the committed fixtures and conformance vectors. That logic lived as ~100 lines of
embedded shell and inline Python inside `.github/workflows/ci.yml`: magic-byte
config prefixes, IVF header/frame synthesis, and IVF-to-OBU de-wrapping. It is
domain-specific, likely to grow as fuzz targets are added, and — buried in YAML —
untestable and not runnable locally. YAML is the wrong long-term home for it.

This change moves the seeding into `cargo xtask seed-fuzz-corpus`, a unit-tested
Rust subcommand, and reduces the workflow step to a single command. The byte
layouts become testable in the `ci` job's `cargo test` and reproducible locally,
and the workflow stays declarative.

## Scope

- Spec sections: none (infrastructure; extends the `tooling` capability, sibling
  to the zero-copy, concurrency, and duplicate-code policies).
- Crates/modules: `xtask` — new `xtask/src/seed_fuzz_corpus.rs` and a
  `Task::SeedFuzzCorpus` subcommand; the existing `fuzz_targets()` helper is made
  `pub(crate)` and reused for target enumeration (no nightly `cargo fuzz list`).
- CI/docs/tests: `.github/workflows/ci.yml` (replace the inline seeding block with
  `cargo xtask seed-fuzz-corpus`; install the pinned stable toolchain in the fuzz
  job so the xtask builds), the implementation matrix, `docs/FEATURE-STATUS.md`,
  the `tooling` capability spec, and `docs/agents/commands.md`. Byte-layout unit
  tests for the IVF synthesis, IVF de-wrap, config prefixes, and an end-to-end
  temp-dir seed.

## Non-goals

- No change to what is fuzzed, which targets exist, or the fuzz matrix structure.
- No change to the seed bytes: the produced corpus is byte-identical to the former
  inline script (verified by a full `diff -r` over all seeds).
- No decoder, reconstruction, encoder, validator, or residual algorithm work; no
  algorithmic stage marked implemented; no AV2 conformance behavior change.
- No new Cargo dependency: the subcommand uses only `std` plus the `anyhow` and
  `toml` that `xtask` already depends on.
- No CI artifacts: the corpus is regenerated in each fuzz leg by the one command,
  not uploaded/downloaded (artifact storage is billed).

## Acceptance criteria

- [ ] Implementation matrix row `INFRA-FUZZ-CORPUS-SEEDING` exists with proof.
- [ ] `cargo xtask seed-fuzz-corpus` exists, is deterministic, reuses
      `fuzz_targets()`, and produces a corpus byte-identical to the former script.
- [ ] `xtask/src/seed_fuzz_corpus.rs` unit-tests the IVF synthesis, de-wrap,
      config prefixes, and an end-to-end seed of a temp corpus.
- [ ] `.github/workflows/ci.yml` replaces the inline shell/Python seeding block
      with `cargo xtask seed-fuzz-corpus` and installs the pinned toolchain in the
      fuzz job.
- [ ] `docs/agents/commands.md` documents the new command.
- [ ] `docs/FEATURE-STATUS.md` is regenerated and `cargo xtask check-feature-status`
      passes.
- [ ] `cargo xtask ci` passes.
