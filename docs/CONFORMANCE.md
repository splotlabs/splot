# Conformance

How `splot` proves it matches AV2 v1.0.0. Proof is recorded in each feature's
`[feature.proof]` in [`docs/IMPLEMENTATION-MATRIX.toml`](./IMPLEMENTATION-MATRIX.toml)
and tracked by the `CONF-*` rows.

## The conformance rows

What each `CONF-*` row proves:

- `CONF-AVM-DIFF-HARNESS` — the AVM differential harness itself:
  `avm encode -> splot validate`, later `splot encode -> avm decode`.
- `CONF-AVM-PARSER-TRACES` — `splot` parser behavior matches AVM parser traces.
- `CONF-AVM-VALID-STREAMS` — representative AVM-generated valid streams pass
  the implemented validator checks.
- `CONF-AVM-INVALID-STREAMS` — malformed/minimized streams produce the
  expected diagnostics without panics.
- `CONF-PUBLIC-VECTORS` — a public AV2 vector corpus runs against the
  validator.
- `CONF-PUBLIC-VECTOR-LICENSE-REVIEW` — redistributability is reviewed before
  any public vectors are vendored or linked.
- `CONF-INSPECT-SNAPSHOTS` — snapshot tests stabilize `splot inspect` output.
- `CONF-FUZZ-NO-PANIC` — malformed input yields errors/reports, never a panic.

For live per-row status, read the generated
[`docs/FEATURE-STATUS.md`](./FEATURE-STATUS.md) and
[`docs/SPEC-COVERAGE.md`](./SPEC-COVERAGE.md); this page does not duplicate
status.

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

Today `cargo xtask conformance` is a registered stub that only prints this plan
(`xtask/src/main.rs`). Once built, the harness will discover a local AVM
checkout/corpus; it will **not** vendor AVM and will **not** run in normal CI.
The live plan is the active OpenSpec change
[`openspec/changes/avm-differential-harness/`](../openspec/changes/avm-differential-harness/).

## Public vectors

`CONF-PUBLIC-VECTORS` integrates a public AV2 vector corpus when one is
available. `cargo xtask fetch-vectors` already exists as a registered stub;
once implemented it will fetch redistributable vectors into a gitignored
`tests/vectors/`.

### Licensing caution

License review is its own row (`CONF-PUBLIC-VECTOR-LICENSE-REVIEW`): vendor
only **redistributable / public** vectors, and do **not** commit samples whose
license is unclear. Project code, docs, tests, and fixtures are PolyForm
Noncommercial 1.0.0; see [AGENTS.md](../AGENTS.md) § 9 for the narrow
exceptions.

## No-panic fuzzing

`CONF-FUZZ-NO-PANIC`: malformed input must produce errors/reports, never a
panic.

- On **stable**, the `splot-core` parser modules carry `*_never_panic(s)`
  proptests that run in plain `cargo test`, so the invariant gates every CI
  run.
- On **nightly**, the `parse_obu` cargo-fuzz target covers the same invariant.
  CI runs a **blocking 60-second `parse_obu` smoke on every PR**
  (`.github/workflows/ci.yml`, `fuzz-smoke` job).

Commands and the full test-layer breakdown live in [AGENTS.md](../AGENTS.md)
§ 4 and [`docs/TESTING.md`](./TESTING.md).

## Inspector snapshots

`CONF-INSPECT-SNAPSHOTS`: snapshot tests stabilize `splot inspect` output. Basic
end-to-end CLI tests already exist (`crates/splot-cli/tests/cli.rs`, tracked by
`CLI-INSPECT`); insta-style snapshots are future work.

## Recording proof

A conformance stage may be marked `done` only when `[feature.proof]` records
reproducible evidence; `cargo xtask check-feature-status` is the gate. The
proof schema and status model live in
[`docs/FEATURE-TRACKING.md`](./FEATURE-TRACKING.md).
