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
oracle hashes. It is not vendored or required by CI. The committed local
comparison harness is opt-in; CI compares splot output with recorded hashes.

[Fixture recipes](../tools/decoder-fixtures/generate.py) record the AVM revision,
source and output hashes, and any instrumentation needed for regeneration.
The multiple-BRT recipe distinguishes native monochrome output from forced I420.
Only redistributable inputs and recorded hashes are committed; reference tools
and instrumentation remain local.

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

## Reproduce the MSDO fixtures

Use AVM revision `a1ec4594d97028ebc34083ec8346616d697c2fc9`.
Set `AVM_SOURCE` to the checkout path and `AVM_BUILD` to a new build directory.
The reference run used a native macOS arm64 Release build with AVM's default
encoder options. Run these commands from the Splot repository.

```sh
cmake -S "$AVM_SOURCE" -B "$AVM_BUILD" -DCMAKE_BUILD_TYPE=Release \
  -DENABLE_EXAMPLES=ON -DCONFIG_INSPECTION=1
cmake --build "$AVM_BUILD" --target avmenc avmdec scalable_encoder --parallel
cargo build -p splot-cli --locked
AVM_BUILD="$AVM_BUILD" python3 tools/decoder-fixtures/msdo.py \
  --splot target/debug/splot --stage "$(mktemp -d)"
```

The recipe encodes each component from its own deterministic I420 input.
It uses the earlier streams for descriptor bytes and resend positions.
It compares the decoded components with full and selected multiplexed
output and checks Splot at 1, 3, and 12 threads. The original vectors remain
in the corpus. The [reference evidence](LOCAL-REFERENCE-EVIDENCE.toml) records
the source hashes, descriptor offsets, and decoded output hashes. AVM inspection
confirmed real inter references in all ten components. In the OPS fixture,
every temporal-layer 1 picture references temporal-layer 0.

## Public Vectors

Public vector ingestion is gated by redistributability review. Do not commit
samples with unclear rights. Project fixtures are PolyForm Noncommercial 1.0.0
unless a specific third-party notice says otherwise.

## Non-Claims

Do not claim broad decoder output equivalence, film-grain output, reference
refresh completeness, encoder output validity, or live AVM differential success
until the matrix row records proof.
