# Conformance

`splot` makes AV2 conformance claims only when there is committed proof. The
canonical proof ledger is `docs/IMPLEMENTATION-MATRIX.toml`; generated coverage
views come from `cargo xtask spec-coverage`,
`cargo xtask decoder-conformance-coverage`, and
`cargo xtask decoder-fixtures coverage`.

## What Counts

- Parser and validator behavior is proven by unit tests, property tests, fuzz
  targets, committed fixtures, and the committed conformance corpus.
- A diagnostic claim is valid only when the emitted `rule_id` is registered in
  `docs/DIAGNOSTICS.md` and source/registry drift checks pass.
- Decoder support claims are recorded in `docs/DECODER-SUPPORT-MATRIX.toml`.
- Decoder output claims can be backed by the committed AVM oracle manifest
  `tests/conformance/decoder-oracle.toml`; CI runs only `splot` against the
  committed hashes and never invokes AVM.
- Local reference evidence is metadata in `docs/LOCAL-REFERENCE-EVIDENCE.toml`;
  it does not run external tools and does not by itself prove runtime decode.

## AVM Boundary

AVM is the AV2 reference oracle. It may be used locally to generate streams or
oracle hashes, but it is not vendored, not required by CI, and not invoked by
committed tests. Any future live AVM differential harness must be opt-in and
recorded as a separate conformance row.

The `syn-profile31-mono-intra-16x16`, `syn-output-multi-brt-16x16`, and `syn-2frame-intra-only-mono-16x16-q255` recipes pin AVM `457cd58681a747465661baccb1f32095bc5b7774`; their source, IVF, raw-output, instrumentation, producer, and recipe hashes are recorded in the fixture generator.
The multiple-BRT fixture uses isolated encoder instrumentation invoking AVM's existing writer twice; native monochrome AVM and splot output is 256-byte SHA-256 `5a5f307aa9ce504d9235634f15cf382e8914c49fbd8dd4d4c47136c917886f7b`, while AVM's separate forced-I420 output is 384-byte SHA-256 `f83545d43c6939ec393b6b8310959b6174fd764b08a12fc22d908408a7e6a43e`.
The intra-only fixture uses isolated encoder frame-type instrumentation; the generator rejects an unrecorded revision, patch, binary producer, or FFmpeg version and compares two regenerated streams before verifying the AVM raw hash. Only redistributable IVF files and hashes are committed; AVM and the local instrumentation remain opt-in.

## Committed Corpus

`tests/conformance/manifest.toml` lists committed validator vectors and their
expected outcome. `cargo xtask conformance` and the CLI integration test read
that manifest and run the project validator only.

The decoder-output oracle reuses committed vectors under
`tests/conformance/vectors/valid/`. `tests/conformance/decoder-oracle.toml`
schema 2 records each fixture's AVM raw-output hash. Every listed fixture must
decode byte-exactly to that hash at the serial and parallel CI widths.

Validator manifest expectations are set equality over diagnostic `rule_id`s:

```toml
expect = "clean"
expect = { diagnostics = ["ivf/truncated-frame-payload"] }
```

## Public Vectors

Public vector ingestion is gated by redistributability review. Do not commit
samples with unclear rights. Project fixtures are PolyForm Noncommercial 1.0.0
unless a specific third-party notice says otherwise.

## Non-Claims

Do not claim broad decoder output equivalence, film-grain output, reference
refresh completeness, encoder output validity, or live AVM differential success
until the matrix row records proof.
