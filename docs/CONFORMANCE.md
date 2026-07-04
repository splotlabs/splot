# Conformance

How `splot` proves it matches AV2 v1.0.0. Proof is recorded in each feature's
`[feature.proof]` in [`docs/IMPLEMENTATION-MATRIX.toml`](./IMPLEMENTATION-MATRIX.toml)
and tracked by the `CONF-*` rows.

## The conformance rows

What each `CONF-*` row proves:

- `CONF-AVM-VALID-STREAMS` — the committed, self-contained conformance corpus
  under `tests/conformance/`: small valid AV2 streams (AVM-generated or
  explicitly provenance-noted local retimings, plus a bootstrap negative)
  validated against a manifest by a committed runner, with
  **no AVM dependency** (see [The committed corpus](#the-committed-corpus)).
- `CONF-AVM-DIFF-HARNESS` — AVM as a **local oracle/generator only**: AVM
  locally produces AV2 streams; it is never vendored and never a build/CI
  dependency. A *live* local differential harness (and any
  `splot encode -> avm decode` reverse direction) remains future work.
- `CONF-AVM-PARSER-TRACES` — `splot` parser behavior matches AVM parser traces.
- `CONF-AVM-INVALID-STREAMS` — malformed/minimized streams produce the
  expected diagnostics without panics, proven by the committed
  [negative mutator](#the-negative-mutator-conf-avm-invalid-streams).
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
requires an AVM checkout, or touches the network. AVM's role is the
[local oracle](#avm-is-a-local-oracle-not-a-dependency) that generated or seeded
the committed vectors; any repository-retimed vector must say so in
`tests/conformance/manifest.toml` and must not claim local reference evidence
unless that evidence is refreshed.

### Layout

```text
tests/conformance/
  manifest.toml                 expected-outcome manifest (one [[vector]] per file)
  vectors/
    valid/                      valid AV2 streams (IVF, mostly AVM-generated;
                                retimed vectors are noted in manifest.toml):
                                diverse resolutions, 8-bit + 10-bit, intra + inter, OPS
    needs-external-hls/         valid AVM streams referencing external-HLS-provided
                                resources (global LCR / QM level); validated
                                standalone they emit the §7.3.8.3 / §7.3.8.9 diagnostic
    invalid/                    negative vectors (truncations, mutations)
```

### Manifest

`tests/conformance/manifest.toml` is an array of `[[vector]]` entries. Each maps
a committed vector to its expected validation outcome (the schema is documented
in the manifest's header comment):

```toml
[[vector]]
path = "vectors/valid/syn-key-intra-64x64.ivf"  # relative to tests/conformance/
description = "…"
expect = "clean"                                   # validator reports ZERO errors

[[vector]]
path = "vectors/invalid/syn-key-intra-64x64-truncated.ivf"
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

### The negative mutator (CONF-AVM-INVALID-STREAMS)

`CONF-AVM-INVALID-STREAMS` is proven by a committed, deterministic **negative
mutator**:
[`crates/splot-cli/tests/negative_mutations.rs`](../crates/splot-cli/tests/negative_mutations.rs)
(`negative_mutations_emit_expected_diagnostics`). It holds a table of
`(committed valid seed, documented mutation, expected diagnostic)` rows. For each
row it:

1. reads a committed valid seed from `tests/conformance/vectors/valid/`;
2. asserts the **unmutated** seed validates clean (causation: the diagnostic is
   provably caused by the mutation, not a pre-broken seed);
3. applies a documented, deterministic byte/field mutation **in memory** (the
   committed file is never written);
4. runs `splot_validate::Validator::validate_bytes` and asserts the expected
   error `rule_id` is **present** (and that the validator did not panic —
   `validate_bytes` returning at all is the no-panic proof).

The mutations are **targeted, named-diagnostic** malformations — not random
fuzzing (the cargo-fuzz targets own no-panic over arbitrary bytes), and not a
live AVM diff. They hit stable, decidable diagnostics across the IVF-container,
OBU-header, and LEB128/OBU-framing layers (e.g. shrinking the IVF `header_len`
below the 32-byte baseline → `ivf/invalid-header-length`; inflating an
`obu_size` LEB128 past the input → `bitstream/parse-error`; setting a key/switch
frame's `obu_tlayer_id` non-zero → `obu-header/temporal-layer-zero-only-types`).
Every expected id is an **existing registered diagnostic** (see
[`docs/VALIDATOR-DIAGNOSTICS.md`](./VALIDATOR-DIAGNOSTICS.md)) verified
empirically; the mutator adds no new diagnostics. It runs under `cargo test`
(hence `cargo xtask ci`), with no AVM and no network.

## AVM is a local oracle (not a dependency)

[AVM](https://github.com/AOMediaCodec/avm) is the AV2 reference software and our
differential-testing oracle. Per the maintainer decision, **AVM is a LOCAL
oracle/generator only**: it is never vendored, never a build/CI dependency, and
no committed runner/test/CI path invokes it or requires an AVM checkout.

- **What AVM does:** locally encode small AV2 streams that `splot validate` must
  validate clean (or flag a real defect). Those generated bitstreams may be
  committed as plain project fixtures under `tests/conformance/vectors/valid/`.
  The committed valid vectors were generated locally from a **project-owned
  synthetic input** (a small generated YUV pattern — no third-party video
  content), e.g.:

  ```text
  avmenc --codec=av2 --ivf --limit=N --cpu-used=8 -w 64 -h 64 -o out.ivf synthetic.y4m
  ```

  This recipe is the documented local oracle step; it is **not** a committed or
  CI script.

- **What stays future work:** a *live* local differential harness against an AVM
  checkout (`CONF-AVM-DIFF-HARNESS`), and the `splot encode -> avm decode`
  reverse direction once an encoder exists. Neither runs in CI; both require the
  maintainer to opt in with a local AVM checkout.

The decoder local-reference evidence manifest
([`docs/LOCAL-REFERENCE-EVIDENCE.toml`](./LOCAL-REFERENCE-EVIDENCE.toml)) is
separate from `tests/conformance/manifest.toml`. It records portable metadata
for future local decoder/hash comparisons and is checked offline; it does not
extend the validator conformance corpus and does not run AVM, dav2d, or
`splot decode`.

## Public vectors

`CONF-PUBLIC-VECTORS` integrates a public AV2 vector corpus when one is
available. `cargo xtask fetch-vectors` already exists as a registered stub;
once implemented it will fetch redistributable vectors into a gitignored
`tests/vectors/`.

### Licensing caution

License review is its own row (`CONF-PUBLIC-VECTOR-LICENSE-REVIEW`): vendor
only **redistributable / public** vectors, and do **not** commit samples whose
license is unclear. Project code, docs, tests, and fixtures are PolyForm
Noncommercial 1.0.0; see the Licensing section in [AGENTS.md](../AGENTS.md) for the narrow
exceptions.

AVM is **BSD-3-Clause-Clear**. AVM is used only as a local encoder to generate
the committed corpus vectors; the AVM-generated AV2 bitstreams are committed as
plain project fixtures (the `splot` project licensing applies to the corpus as a
whole). No AVM source is vendored. The committed bootstrap vectors are encoded
from a **project-owned synthetic YUV input** — there is no third-party video
content in the corpus, so no third-party media provenance applies (the encoder
tool, AVM, is BSD-3-Clause-Clear and not committed).

## No-panic fuzzing

`CONF-FUZZ-NO-PANIC`: malformed input must produce errors/reports, never a
panic.

- On **stable**, the `splot-core` parser modules carry `*_never_panic(s)`
  tests (mostly proptests, plus exhaustive-truncation unit tests) that run in
  plain `cargo test`, so the invariant gates every CI run.
- On **nightly**, cargo-fuzz targets cover the parser, validator, byte planner,
  minimal tier hash/Y4M byte surfaces, decoded-frame/plane runtime types,
  and Y4M output serialization surfaces. CI runs a blocking per-target smoke on
  every PR: each registered target is one parallel matrix leg, gated by the
  `fuzz-smoke` status check (`.github/workflows/ci.yml`).
- Each fuzz matrix leg **seeds** its target's corpus from the curated
  `tests/fixtures/*.av2` AND the committed conformance corpus
  (`tests/conformance/vectors/**.ivf`): the diverse AVM-generated streams are
  strong coverage-guided seeds — fed to `parse_ivf` directly, to
  `validate_bytes` config-prefixed, to `decode_plan_bytes` and
  `decode_runtime_hash_bytes` with prefix-preserving raw-byte seeds, and
  de-wrapped to a raw OBU stream for `parse_obu` / `parse_bitstream`.

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
