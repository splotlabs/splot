# Change: decoder-avm-oracle-corpus

## Feature IDs

- `CONF-AVM-DECODE-ORACLE`

## Why

The committed conformance corpus (`CONF-AVM-VALID-STREAMS`) proves `splot
validate` accepts AVM-generated streams, but nothing systematically compares
`splot`'s *decoded output* against the AVM decoder. Decoder correctness could
regress — or hide behind a single hand-checked stream — with no gate.

The reference decoder output is byte-comparable: `splot decode --output-format
raw` emits the same visible I420 samples (§6.18) as `avmdec --i420 --rawvideo`.
Recording the AVM oracle hash per stream lets a committed runner compare `splot`
against AVM in CI **without invoking AVM**.

## What changes

- A committed decode-output oracle manifest
  (`tests/conformance/decoder-oracle.toml`) over the reused
  `CONF-AVM-VALID-STREAMS` corpus: each fixture records the AVM raw-output oracle
  hash and an expected `splot` outcome (`must_pass` = output equals the oracle;
  `xfail_splot` = fails closed with a recorded `decode/unsupported-feature`
  diagnostic).
- A committed capability taxonomy
  (`tests/conformance/decoder-oracle-coverage.toml`) and a generated coverage
  report (`docs/decoder/DECODER-ORACLE-COVERAGE.md`) that expose which
  decoder-relevant capabilities are fixture-backed and which remain unimplemented.
- A CI gate: `crates/splot-cli/tests/decoder_oracle.rs` decodes each fixture
  in-process and asserts the recorded outcome. `xfail_splot` fixtures never block
  CI; an unexpected pass (XPASS) is reported and fails only in a strict local
  mode.
- `cargo xtask decoder-fixtures {verify,report,coverage}` (verify + coverage
  drift-check wired into `cargo xtask ci`), plus local-only regeneration tooling
  under `tools/decoder-fixtures/`.

## Non-goals

- No vendoring of AVM; AVM is never invoked, a build dependency, or required in
  CI. Oracle hashes are recorded offline into the committed manifest.
- No re-encoded parallel corpus: the oracle system reuses the committed
  `tests/conformance/vectors/` `.ivf` files (maintainer decision).
- Not the live `avm encode → splot validate` harness — that is the separate
  `avm-differential-harness` change.

## Acceptance criteria

- [x] Every committed valid `.ivf` has a decoder-oracle entry (no stream hides).
- [x] `must_pass` fixtures decode to output whose SHA-256 equals the recorded AVM
      oracle hash; `xfail_splot` fixtures fail closed with the recorded diagnostic.
- [x] The runner and `verify`/`coverage` checks run in `cargo xtask ci` with no
      AVM and no network.
- [x] The coverage report lists every taxonomy capability and its fixture
      backing, marking non-emittable capabilities `not_fixtureable_with_avm_encoder`.
- [x] Proof recorded in the `CONF-AVM-DECODE-ORACLE` row.
