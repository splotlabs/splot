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

### Temporal fixture recipe

The three `syn-tlayer*-reproducible-*` fixtures exercise the same temporal
layer patterns as the earlier dependent and sparse fixtures. The recipe
defines its own I420 inputs. It checks that two AVM runs reproduce the committed
streams. It also compares complete output and temporal layer selections between
Splot and AVM. The recipe revision and the decoder oracle pin produce the same
decoded bytes for all three streams.

Use an AVM checkout at `a1ec4594d97028ebc34083ec8346616d697c2fc9`.
Set `AVM_SOURCE` to its absolute path. Set `AVM_BUILD` to a new build
directory. Run these commands from the Splot repository.

```sh
cmake -S "$AVM_SOURCE" -B "$AVM_BUILD" -DCMAKE_BUILD_TYPE=Release \
  -DAVM_TARGET_CPU=generic -DCONFIG_ML_PART_SPLIT=0 \
  -DCONFIG_DIP_EXT_PRUNING=0 -DCONFIG_TENSORFLOW_LITE=0 \
  -DENABLE_TESTS=0 -DENABLE_EXAMPLES=0 -DENABLE_TOOLS=0
cmake --build "$AVM_BUILD" --target avm avmdec --parallel
cargo build -p splot-cli --locked
python3 tools/decoder-fixtures/generate.py temporal-fixtures \
  --avm-source "$AVM_SOURCE" --avm-build "$AVM_BUILD" \
  --splot target/debug/splot --stage "$(mktemp -d)"
```

AVM's inspection callback confirms that both TL1 frames in each two-layer
stream use TL0 inter references. It also confirms that the sparse stream's TL2
frame uses TL0 and its TL3 frame uses TL0 and TL2. The trace counts references
for each 4×4 luma cell, including both references in compound prediction.

| Fixture | Frame | References by layer |
| --- | --- | --- |
| 8-bit two-layer | First and second TL1 | TL0 (560 and 580) |
| 10-bit two-layer | First and second TL1 | TL0 (252 and 316) |
| Sparse | TL2 | TL0 (196) |
| Sparse | TL3 | TL0 (40), TL2 (240) |

### Embedded fixture recipe

The four `syn-mlayer2-reproducible-*` fixtures define their source I420
samples in the same encoder harness as the temporal recipe. Each frame
submits the same source picture to ML0 and ML1. The recipe checks the source
hash, repeats the encode, and compares the result to the committed stream.
It clears the local LCR's three-bit parent ID for these single-stream
fixtures. Whole-stream and selected-layer Splot outputs must match the
corresponding native AVM pictures.

Use the AVM revision and build configuration in the temporal recipe. Run
this command from the Splot repository.

```sh
python3 tools/decoder-fixtures/generate.py embedded-fixtures \
  --avm-source "$AVM_SOURCE" --avm-build "$AVM_BUILD" \
  --splot target/debug/splot --stage "$(mktemp -d)"
```

AVM inspection counts references to ML0 in actual ML1 inter blocks.
Use `AV2_SET_INSPECTION_CALLBACK` with an AVM build configured with
`CONFIG_INSPECTION=1`. Wrap each Annex B temporal unit in one IVF record.
For each decoded 4×4 luma cell, resolve each inter reference from
`ifd_inspect` with `get_ref_frame_buf` and count its `mlayer_id`.

| Fixture | ML0 references in each ML1 picture |
| --- | --- |
| 8-bit equal resolution | 512, 256, 256 |
| 10-bit equal resolution | 512, 256, 256 |
| 8-bit mixed resolution | 5948, 3004, 2780 |
| 8-bit with two temporal layers and two tile groups | 1024, 512, 512, 512 |

The count includes both references in compound prediction. The decoder-oracle
pin `457cd58681a747465661baccb1f32095bc5b7774` produces the same decoded
samples for all four streams.

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
