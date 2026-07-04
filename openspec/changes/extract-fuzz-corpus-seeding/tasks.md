# Tasks

## Matrix and docs

- [x] Add `INFRA-FUZZ-CORPUS-SEEDING` to `docs/IMPLEMENTATION-MATRIX.toml`.
- [x] Run `cargo xtask check-feature-status`.
- [x] Record the change in `openspec/changes/README.md` active table.
- [x] Document `cargo xtask seed-fuzz-corpus` in `AGENTS.md`.

## Implementation

- [x] Add `xtask/src/seed_fuzz_corpus.rs` with `run_seed_fuzz_corpus` /
      `seed_corpus` and the pure `prefixed` / `ivf_wrap` / `ivf_dewrap` helpers.
- [x] Add the `Task::SeedFuzzCorpus` clap subcommand and dispatch arm in
      `xtask/src/main.rs`; make `fuzz_targets()` `pub(crate)` and reuse it.
- [x] Replace the inline shell/Python seeding block in
      `.github/workflows/ci.yml` with `cargo xtask seed-fuzz-corpus`, and install
      the pinned stable toolchain in the fuzz job.

## Tests and proof

- [x] Unit-test the IVF synthesis, IVF de-wrap, config prefixes, and an
      end-to-end temp-corpus seed (byte-exact assertions).
- [x] Verify byte-parity against the former script with a full `diff -r` over the
      produced corpus (974 seeds, identical).
- [x] Add proof commands to the matrix row.
- [x] `cargo xtask ci` is green.
