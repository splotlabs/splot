# Conformance

How `splot` proves it matches AV2 v1.0.0. Proof is recorded in each feature's
`[feature.proof]` in [`docs/IMPLEMENTATION-MATRIX.toml`](./IMPLEMENTATION-MATRIX.toml)
and tracked by the `CONF-*` rows.

## The conformance rows

What each `CONF-*` row proves:

- `CONF-AVM-VALID-STREAMS` — the committed, self-contained conformance corpus
  under `tests/conformance/`: small AVM-generated valid AV2 streams (and a
  bootstrap negative) validated against a manifest by a committed runner, with
  **no AVM dependency** (see [The committed corpus](#the-committed-corpus)).
- `CONF-AVM-DIFF-HARNESS` — AVM as a **local oracle/generator only**: AVM
  locally produces AV2 streams; it is never vendored and never a build/CI
  dependency. A *live* local differential harness (and any
  `splot encode -> avm decode` reverse direction) remains future work.
- `CONF-AVM-PARSER-TRACES` — `splot` parser behavior matches AVM parser traces.
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

## The committed corpus

`CONF-AVM-VALID-STREAMS` is a **committed, self-contained** conformance corpus:
every vector is committed under `tests/conformance/` and validated by a
committed runner that has **no AVM dependency** — it never invokes AVM,
requires an AVM checkout, or touches the network. AVM's only role is the
[local oracle](#avm-is-a-local-oracle-not-a-dependency) that *generated* the
committed vectors.

### Layout

```text
tests/conformance/
  manifest.toml                 expected-outcome manifest (one [[vector]] per file)
  vectors/
    valid/                      AVM-generated valid AV2 streams (IVF)
    invalid/                    negative vectors (truncations, mutations)
```

### Manifest

`tests/conformance/manifest.toml` is an array of `[[vector]]` entries. Each maps
a committed vector to its expected validation outcome (the schema is documented
in the manifest's header comment):

```toml
[[vector]]
path = "vectors/valid/avm-key-intra-352x288.ivf"  # relative to tests/conformance/
description = "…"
expect = "clean"                                   # validator reports ZERO errors

[[vector]]
path = "vectors/invalid/avm-key-intra-352x288-truncated.ivf"
description = "…"
expect = { diagnostics = ["ivf/truncated-frame-payload"] }  # EXACTLY this error-id set
```

`expect = "clean"` asserts no errors. `expect = { diagnostics = [...] }` asserts
**set equality** over the emitted error `rule_id`s: every listed id present, and
no unexpected error ids.

### Runner (no AVM)

Two committed entry points share the manifest and run the same check —
read each committed vector's bytes and validate them with
`splot_validate::Validator::validate_bytes` (the same entry point the CLI
`validate` command uses, with container auto-detect):

- **The CI gate** is the integration test
  [`crates/splot-cli/tests/conformance.rs`](../crates/splot-cli/tests/conformance.rs)
  (`conformance_corpus_matches_manifest`). It runs under `cargo test`, hence
  under `cargo xtask ci`. It also asserts the corpus exercises **both** manifest
  arms, so the diagnostics path is never vacuous.
- **The ergonomic manual entry** is `cargo xtask conformance`
  ([`xtask/src/conformance.rs`](../xtask/src/conformance.rs)), which prints a
  per-vector pass/fail summary. `xtask` is standalone (no `splot-*` dependency),
  so it shells out to the built `splot validate --json` binary — a project
  binary, not AVM.

Neither path invokes AVM or the network.

## AVM is a local oracle (not a dependency)

[AVM](https://github.com/AOMediaCodec/avm) is the AV2 reference software and our
differential-testing oracle. Per the maintainer decision, **AVM is a LOCAL
oracle/generator only**: it is never vendored, never a build/CI dependency, and
no committed runner/test/CI path invokes it or requires an AVM checkout.

- **What AVM does:** locally encode small AV2 streams that `splot validate` must
  validate clean (or flag a real defect). Those generated bitstreams may be
  committed as plain project fixtures under `tests/conformance/vectors/valid/`.
  The committed valid vectors were generated locally with, e.g.:

  ```text
  avmenc --codec=av2 --ivf --limit=N --cpu-used=8 -w 352 -h 288 -o out.ivf paris_352_288_30.y4m
  ```

  (`paris_352_288_30.y4m` is a public test input.) This recipe is the documented
  local oracle step; it is **not** a committed or CI script.

- **What stays future work:** a *live* local differential harness against an AVM
  checkout (`CONF-AVM-DIFF-HARNESS`), and the `splot encode -> avm decode`
  reverse direction once an encoder exists. Neither runs in CI; both require the
  maintainer to opt in with a local AVM checkout.

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

AVM is **BSD-3-Clause-Clear**. AVM is used only as a local encoder to generate
the committed corpus vectors; the AVM-generated AV2 bitstreams are committed as
plain project fixtures (the `splot` project licensing applies to the corpus as a
whole). No AVM source is vendored. The committed test input used to generate the
bootstrap vectors (`paris_352_288_30.y4m`) is a public test sequence.

## No-panic fuzzing

`CONF-FUZZ-NO-PANIC`: malformed input must produce errors/reports, never a
panic.

- On **stable**, the `splot-core` parser modules carry `*_never_panic(s)`
  tests (mostly proptests, plus exhaustive-truncation unit tests) that run in
  plain `cargo test`, so the invariant gates every CI run.
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
