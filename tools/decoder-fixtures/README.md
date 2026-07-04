<!-- SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0 -->
<!-- SPDX-FileCopyrightText: 2026 Bartosz Tomczyk <bartekplus@gmail.com> -->

# Decoder fixture / AVM oracle tooling

Local-only developer scripts that regenerate the AVM decode-output oracle data
consumed by `tests/conformance/decoder-oracle.toml`, and that generate new
fixture candidates for the committed corpus at
`tests/conformance/vectors/valid/`.

**AVM is local-only.** These scripts are never run in CI and never invoked by
any committed test. AVM itself — its source, its build, and any raw/Y4M output
it produces — is never vendored or committed. Only the committed `.ivf`
vectors and the oracle hashes recorded in
`tests/conformance/decoder-oracle.toml` are committed; see
[docs/decoder/AVM-FIXTURE-CORPUS.md](../../docs/decoder/AVM-FIXTURE-CORPUS.md)
and [docs/decoder/avm-fixture-mission-log.md](../../docs/decoder/avm-fixture-mission-log.md)
for the full design record.

## Prerequisites

- A local AVM checkout with `avmenc`/`avmdec` already built (see
  [AOMediaCodec/avm](https://github.com/AOMediaCodec/avm)).
- `ffmpeg`, `python3` on `PATH`.
- A release `splot` build: `cargo build --release -p splot-cli`.

```sh
export AVM_ROOT=/path/to/your/avm/checkout   # optional; scripts default to
                                              # /Users/bartosztomczyk/Devel/avm
# or, if your build output lives elsewhere:
export AVM_BUILD=/path/to/avm/build-dir
export SPLOT_BIN=/path/to/splot              # optional; defaults to
                                              # target/release/splot
```

## Scripts

- **`find_avm.py`** — resolves `avmenc`/`avmdec` via `$AVM_BUILD`, `$AVM_ROOT`,
  the default checkout, or `$PATH`. Run it first to confirm your environment:

  ```sh
  python3 tools/decoder-fixtures/find_avm.py
  ```

- **`update_oracle_hashes.py`** — the main refresh script. For every `.ivf`
  under `tests/conformance/vectors/valid/`, decodes with AVM, hashes the
  whole-stream and per-shown-frame raw output, probes `splot decode`, and
  classifies each vector (`must_pass` / `xfail_splot` / `mismatch` /
  `splot_error` / `avm_error`). Emits a JSON report; fold the relevant fields
  into `tests/conformance/decoder-oracle.toml` by hand.

  ```sh
  python3 tools/decoder-fixtures/update_oracle_hashes.py --out /tmp/oracle.json
  ```

- **`gen_sources.py`** — generates small (<=64x64, <=4 frame), 8/10/12-bit
  deterministic `ffmpeg lavfi` Y4M sources (flat, testsrc2, gradient,
  checkerboard, moving square) under `target/decoder-fixtures/source-y4m/`,
  for feeding into `encode_fixture.py` when adding a new fixture.

  ```sh
  python3 tools/decoder-fixtures/gen_sources.py --patterns flat checkerboard
  ```

- **`encode_fixture.py`** — wraps `avmenc` with the deterministic flag set
  (`-D`, single-threaded, fixed `--qp`/`--kf-max-dist`) to turn a Y4M source
  into a candidate `.ivf`, encoding twice to verify byte-identical
  determinism. Never writes into `tests/`; prints a `cp` command and the
  manifest follow-up steps for a human to run after vetting the candidate.

  ```sh
  python3 tools/decoder-fixtures/encode_fixture.py \
    target/decoder-fixtures/source-y4m/flat-32x32-1f-8bit.y4m \
    --name syn-my-new-fixture --qp 100 --kf-max-dist 0
  ```

## The differential recipe

`splot decode --output-format raw` emits concatenated visible I420 sample
bytes per frame (AV2 § 6.18), confirmed byte-identical to
`avmdec --i420 --rawvideo`:

```
sha256(avmdec --i420 --rawvideo -o out.raw <ivf>) == sha256(splot decode --output-format raw -o out.raw <ivf>)
```

`update_oracle_hashes.py` records the left-hand side (the AVM oracle hash);
the committed CI runner (`crates/splot-cli/tests/decoder_oracle.rs`) computes
the right-hand side at test time, with no AVM dependency.

## Adding a new fixture

1. `gen_sources.py` to produce or pick a Y4M source.
2. `encode_fixture.py` to encode + determinism-check a candidate `.ivf`.
3. Manually inspect/vet the candidate, then `cp` it into
   `tests/conformance/vectors/valid/`.
4. Add a `[[vector]]` entry to `tests/conformance/manifest.toml` (validator
   outcome).
5. Re-run `update_oracle_hashes.py` and add the new `[[fixture]]` entry (with
   its `[fixture.hashes]` / `[fixture.expected_splot]` sub-tables) to
   `tests/conformance/decoder-oracle.toml`.
