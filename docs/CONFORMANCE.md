# Conformance

How `splot` proves it matches AV2 v1.0.0. Proof is recorded in each feature's
`[feature.proof]` in [`docs/IMPLEMENTATION-MATRIX.toml`](./IMPLEMENTATION-MATRIX.toml)
and tracked by the `CONF-*` rows.

## AVM is the oracle

[AVM](https://github.com/AOMediaCodec/avm) is the AV2 reference software and our
differential-testing oracle.

- **Initial direction** (`CONF-AVM-DIFF-HARNESS`):

  ```text
  avm encode  ->  splot validate
  ```

  AVM produces streams; `splot` must validate them clean (or flag a real defect).

- **Future direction** (after the encoder exists):

  ```text
  splot encode  ->  avm decode
  ```

  AVM must decode what `splot` produces.

The harness lives in `xtask` (`cargo xtask conformance`, currently a stub). It
discovers a local AVM checkout/corpus; it does **not** vendor AVM and does not run
in normal CI.

## Public vectors

`CONF-PUBLIC-VECTORS` integrates a public AV2 vector corpus when one is available
(planned: `cargo xtask fetch-vectors` into a gitignored `tests/vectors/`).

### Licensing caution

Vendor only **redistributable / public** vectors. Do **not** commit samples whose
license is unclear. The whole repository is PolyForm Noncommercial 1.0.0; do not mix
licenses (see [AGENTS.md](../AGENTS.md) § 9).

## No-panic fuzzing

`CONF-FUZZ-NO-PANIC`: malformed input must produce errors/reports, never a panic.

- On **stable**, the `parsers_never_panic` proptest in `splot-core::annexb` runs in
  `cargo test`.
- On **nightly**, the `parse_obu` cargo-fuzz target covers the same invariant:

  ```bash
  cargo install cargo-fuzz --locked
  cargo +nightly fuzz run parse_obu
  ```

## Inspector snapshots

`CONF-INSPECT-SNAPSHOTS`: snapshot tests stabilize `splot inspect` output. Basic
end-to-end CLI tests already exist (`crates/splot-cli/tests/cli.rs`, tracked by
`CLI-INSPECT`); insta-style snapshots are future work.

## Recording proof

A conformance stage (for example `avm_diff` or `tests`) may be marked `done` only
when `[feature.proof]` records a reproducible command, a fixture/vector path, a
test module, or a diagnostic id. `cargo xtask check-feature-status` enforces this;
`cargo xtask spec-coverage` lists rows still missing proof.
