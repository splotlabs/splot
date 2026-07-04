<!-- SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0 -->
<!-- SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com> -->

# AVM decode-output fixture corpus

`CONF-AVM-DECODE-ORACLE`: a committed **decode-output** differential oracle for
`splot decode`, checked against AVM in CI with **no AVM dependency**. Full
design history: [`avm-fixture-mission-log.md`](./avm-fixture-mission-log.md).

## What it is

The corpus is reuse, not a new set of fixtures: it layers an oracle manifest on
top of the existing committed conformance corpus
(`tests/conformance/vectors/valid/*.ivf`, AVM-generated, project-owned
synthetic input — the same vectors `CONF-AVM-VALID-STREAMS` already validates).
No parallel corpus is created.

## Differential basis

Confirmed byte-identical:

```text
sha256(splot decode --output-format raw)  ==  sha256(avmdec --i420 --rawvideo)  ==  recorded oracle hash
```

Both sides emit concatenated, visible I420 sample bytes per frame (AV2 §6.18).
The AVM side is computed once, locally, and committed as a hash; the splot side
is computed at CI time.

## Manifest and taxonomy

- [`tests/conformance/decoder-oracle.toml`](../../tests/conformance/decoder-oracle.toml) —
  one `[[fixture]]` per vector: `status`, dimensions, `features`,
  `spec_sections`, `[fixture.hashes]` (`ivf_sha256`, `avm_raw_i420_sha256`,
  `avm_raw_i420_frame_sha256`), `[fixture.expected_splot]`. Full schema is in
  the file's own header comment — not duplicated here.
- [`tests/conformance/decoder-oracle-coverage.toml`](../../tests/conformance/decoder-oracle-coverage.toml) —
  71-capability taxonomy every fixture's `features` must resolve against.

## Status model

| `status` | Meaning |
|---|---|
| `must_pass` | splot raw output must hash-equal the recorded AVM oracle hash. |
| `xfail_splot` | splot must fail closed (`decode/unsupported-feature` + reason); a known gap. |
| `avm_oracle_only` | recorded for coverage bookkeeping; not asserted either way. |
| `blocked` | excluded pending a documented prerequisite. |

**XPASS**: an `xfail_splot` fixture that now decodes is reported, not a hard
CI failure — it flags a feature that shipped without updating the manifest.
Strict mode turns XPASS into a failure: `SPLOT_DECODER_ORACLE_STRICT_XPASS=1`,
or `cargo xtask decoder-fixtures report --strict-xpass`.

## Running it

- **CI gate** (in-process, no AVM): `crates/splot-cli/tests/decoder_oracle.rs`
  (`decoder_oracle_corpus_matches_manifest`), under `cargo test` /
  `cargo xtask ci`.
- **Ergonomic/manual**: `cargo xtask decoder-fixtures verify` (manifest/taxonomy
  shape, hashes, orphan-`.ivf` check — no decode, no AVM),
  `cargo xtask decoder-fixtures report [--strict-xpass]` (decodes every fixture
  with the built `splot` binary, prints PASS/XFAIL/XPASS/FAIL), and
  `cargo xtask decoder-fixtures coverage [--check]` (regenerates or verifies
  `DECODER-ORACLE-COVERAGE.md`). `cargo xtask ci` wires in `verify` and
  `coverage --check`.

## How to regenerate locally

Needs a local AVM checkout (never CI). Tooling lives in
[`tools/decoder-fixtures/`](../../tools/decoder-fixtures/README.md):

```sh
export AVM_ROOT=/path/to/your/avm/checkout   # defaults to ~/Devel/avm
python3 tools/decoder-fixtures/find_avm.py              # confirm avmenc/avmdec resolve
python3 tools/decoder-fixtures/update_oracle_hashes.py --out /tmp/oracle.json
# fold the JSON report's per-vector fields into tests/conformance/decoder-oracle.toml by hand
```

Adding a brand-new fixture uses `gen_sources.py` (deterministic Y4M sources)
and `encode_fixture.py` (deterministic `avmenc` wrapper) before the manual
`update_oracle_hashes.py` step — see the tools README for the full sequence.

## Why AVM is local-only

AVM is the AV2 reference decoder and the oracle-hash source, never a build or
runtime dependency: not vendored, not invoked in CI, no committed path requires
an AVM checkout. Local checkout used for this corpus:
`/Users/bartosztomczyk/Devel/avm` (commit `457cd58681a747465661baccb1f32095bc5b7774`,
`v1.0.0-33`).

## Coverage and the feature-unlock backlog

Current corpus: 47 `must_pass`, 21 `xfail_splot`, 0 mismatches — splot either
matches AVM byte-for-byte or fails closed. Full per-capability breakdown is
generated: [`DECODER-ORACLE-COVERAGE.md`](./DECODER-ORACLE-COVERAGE.md) (do not
hand-edit; regenerate with `cargo xtask decoder-fixtures coverage`). Largest
`xfail_splot` clusters, i.e. the feature-unlock backlog:

- `general_intra_transform_tool_residual` (10 fixtures)
- `unsupported_cfl_intra` (5 fixtures)

## Relationship to other conformance rows

- `CONF-AVM-VALID-STREAMS` — validates the same `.ivf` corpus with the
  validator, not the decoder. This oracle reuses those vectors for a decode
  differential instead of duplicating them.
- `.obu` Annex-B twins are **deliberately deferred**: the reused corpus is IVF,
  and the raw-OBU/Annex-B parse path already has its own coverage via
  `tests/fixtures/*.av2` and the `parse_obu` fuzz target.
- Distinct from `CONF-AVM-DIFF-HARNESS` (`avm-differential-harness`): that is
  the still-future-work *live* `avm encode -> splot validate` harness. This
  oracle is decode-output, hash-based, and fully committed — no live AVM
  invocation anywhere in CI.

See also: [`avm-fixture-mission-log.md`](./avm-fixture-mission-log.md),
[`../CONFORMANCE.md`](../CONFORMANCE.md),
[`DECODER-ORACLE-COVERAGE.md`](./DECODER-ORACLE-COVERAGE.md),
[`../../tools/decoder-fixtures/README.md`](../../tools/decoder-fixtures/README.md).
