<!-- SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0 -->
<!-- SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com> -->

# AVM decode-output oracle corpus

`CONF-AVM-DECODE-ORACLE`: a committed decode-output differential for `splot
decode`, checked against the AVM decoder in CI **with no AVM dependency**. It
reuses the committed `CONF-AVM-VALID-STREAMS` corpus
(`tests/conformance/vectors/valid/*.ivf`) rather than a parallel corpus.

## Differential basis

`splot decode --output-format raw` and `avmdec --i420 --rawvideo` emit the same
visible I420 samples per frame (AV2 §6.18) — confirmed byte-identical. The AVM
hash is computed once locally and committed; `splot` runs at CI time.

## Manifest and runner

- `tests/conformance/decoder-oracle.toml` — one entry per fixture: `id`
  (basename under `vectors_dir`), `status`, `features`, `ivf_sha256`,
  `avm_raw_sha256`, and for `xfail_splot` the `unsupported_reason` / `matrix_row`.
- `tests/conformance/decoder-oracle-coverage.toml` — the capability taxonomy;
  `docs/decoder/DECODER-ORACLE-COVERAGE.md` is the generated coverage report.
- CI gate: `crates/splot-cli/tests/decoder_oracle.rs` (in-process, no AVM) —
  `must_pass` compares `splot` raw output to `avm_raw_sha256`; `xfail_splot`
  asserts the fail-closed `decode/unsupported-feature` reason/matrix row; XPASS is
  non-blocking (strict via `SPLOT_DECODER_ORACLE_STRICT_XPASS`); every committed
  valid `.ivf` must have an entry.
- `cargo xtask decoder-fixtures verify` (metadata) and `coverage [--check]`
  (report drift) run in `cargo xtask ci`.

## Regenerate locally (needs a local AVM checkout — never CI)

```sh
export AVM_ROOT=/path/to/avm            # default ~/Devel/avm
python3 tools/decoder-fixtures/generate.py find
python3 tools/decoder-fixtures/generate.py hashes --out /tmp/oracle.json
# fold the report into decoder-oracle.toml, then: cargo xtask decoder-fixtures coverage
# (re-encode the capability-coverage fixtures: generate.py coverage-fixtures --stage <dir>)
```

AVM is the local oracle-hash generator only — never vendored, never invoked in
CI, no committed path requires it. Corpus generated at AVM commit
`457cd58681a747465661baccb1f32095bc5b7774` (`v1.0.0-33`).

## Coverage and notes

`docs/decoder/DECODER-ORACLE-COVERAGE.md` lists every capability, its fixture
backing, and the `xfail_splot` feature-unlock backlog. Capabilities the local AVM
corpus does not exercise are marked `not_fixtureable_with_avm_encoder`.

- Distinct from `CONF-AVM-DIFF-HARNESS` (the future *live* `avm encode → splot
  validate` harness); this oracle is decode-output, hash-based, fully committed.
- `.obu` Annex-B twins are deferred: the corpus is IVF, and the raw-OBU parse
  path is covered by `tests/fixtures/*.av2` + the `parse_obu` fuzz target.
