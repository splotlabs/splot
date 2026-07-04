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

## Committed Corpus

`tests/conformance/manifest.toml` lists committed validator vectors and their
expected outcome. `cargo xtask conformance` and the CLI integration test read
that manifest and run the project validator only.

The decoder-output oracle reuses committed vectors under
`tests/conformance/vectors/valid/`. `tests/conformance/decoder-oracle.toml`
records each fixture's AVM raw-output hash and expected `splot` outcome:
`must_pass` fixtures must match the hash, while `xfail_splot` fixtures must fail
closed with the recorded unsupported-feature reason.

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
