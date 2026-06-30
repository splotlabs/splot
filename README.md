<div align="center">

# splot

**Spec-faithful AV2 bitstream validation, in safe Rust.**

A validator and inspector for the [AV2 video codec](https://av2.aomedia.org/v1.0.0/index.html) today — an experimental encoder later.

[![CI](https://github.com/splotlabs/splot/actions/workflows/ci.yml/badge.svg)](https://github.com/splotlabs/splot/actions/workflows/ci.yml)
[![AV2 spec v1.0.0](https://img.shields.io/badge/AV2%20spec-v1.0.0-blueviolet)](https://av2.aomedia.org/v1.0.0/index.html)
[![Rust 1.96 · edition 2024](https://img.shields.io/badge/rust-1.96%20%C2%B7%20edition%202024-orange)](./rust-toolchain.toml)
[![unsafe forbidden](https://img.shields.io/badge/unsafe-forbidden-success)](./Cargo.toml)
[![License: PolyForm Noncommercial 1.0.0](https://img.shields.io/badge/license-PolyForm%20Noncommercial%201.0.0-blue)](./LICENSE.md)

</div>

Think `clippy`, but for AV2 bitstreams. Point `splot` at a raw Annex B stream
or IVF-wrapped Annex B stream and every problem comes back as structured data —
a stable rule id, a severity, the AV2 spec section it violates, and the byte
offset where it happens:

```console
$ splot validate bad.av2
[ERROR] obu-order/temporal-unit-missing-delimiter (§7.3.7) @byte 1: OBU_TEMPORAL_DELIMITER appears before a global OBU_TEMPORAL_DELIMITER starts the temporal unit
[ERROR] obu-header/global-xlayer-required (§6.2.2) @byte 1: OBU_TEMPORAL_DELIMITER requires obu_xlayer_id == GLOBAL_XLAYER_ID (31), found 5
2 error(s), 0 warning(s), 0 info
NOT conformant
```

Add `--json` and the same report becomes machine-readable (exit code `1` on
non-conformance), ready for CI pipelines and tooling. Each finding carries the
same six fields, always:

```json
{
  "rule_id": "obu-header/global-xlayer-required",
  "spec_section": "6.2.2",
  "severity": "Error",
  "byte_offset": 1,
  "bit_offset": null,
  "message": "OBU_TEMPORAL_DELIMITER requires obu_xlayer_id == GLOBAL_XLAYER_ID (31), found 5"
}
```

## Why splot?

- **Diagnostics are the product.** Every finding is a six-field `Diagnostic` —
  stable `rule_id`, severity, spec section, byte/bit offset, message — never a
  log line. Exit codes are part of the contract: `0` conformant, `1` not
  conformant, `2` operational error.
- **CI-enforced diagnostic registry**, from `obu-header/` and `obu-order/` to
  `sequence-header/`, `frame-header/`, `metadata/`, and `film-grain/`, plus
  `ivf/` container diagnostics. The full registry lives in
  [docs/VALIDATOR-DIAGNOSTICS.md](./docs/VALIDATOR-DIAGNOSTICS.md) and is
  CI-enforced: if the registry and the source disagree, the build fails.
- **The spec cannot drift.** A byte-faithful mirror of the AV2 v1.0.0
  specification is committed at [docs/spec/av2/1.0.0/](./docs/spec/av2/1.0.0/)
  — 628 indexed sections, checksum-pinned, with hand-edits rejected by
  `cargo xtask check-spec-mirror`. Parsed syntax elements cite their spec
  section in the source.
- **Parsers never panic.** `unsafe` is forbidden across the workspace, runtime
  panics are banned in library code, and the never-panic invariant is hammered
  by over 700 tests, property tests across parser modules, and a libFuzzer
  target — including a blocking 60-second fuzz smoke on every pull request.
- **Status you can audit, not vibes.** The tracked feature ledger in
  [docs/IMPLEMENTATION-MATRIX.toml](./docs/IMPLEMENTATION-MATRIX.toml) render
  into generated, drift-gated coverage docs:
  [SPEC-COVERAGE.md](./docs/SPEC-COVERAGE.md) maps every cited spec section to
  its parse/validate/test status, and
  [FEATURE-STATUS.md](./docs/FEATURE-STATUS.md) is the per-feature ledger.

## Status

> **Pre-alpha, validator-first.** The validator and inspector work today; the
> decoder and encoder are reserved API shapes only. Tracked against the **AV2
> Bitstream & Decoding Process Specification v1.0.0** (Final Deliverable,
> 2026-05-28).

| Capability | Today |
| --- | --- |
| Annex B envelope, IVF container, LEB128, AV2 OBU header parsing | working |
| Sequence-header and frame-header parsing (incl. tiling, quantization, segmentation) | working |
| Header-level and container conformance validation | working |
| `splot inspect` OBU dump (text and JSON, partial-parse tolerant) | working |
| `splot explain` diagnostic catalog (text and JSON, `--list`) | working |
| Bitstream **writer** (`BitWriter` primitives + OBU header / trailing-bits / Annex B framing, all inverse-of-parser, round-trip-proven) | library-only |
| Conformance vectors, AVM differential testing | planned |
| `splot decode` / `splot encode` | stubs — exit with a clear error |

Validator-first is deliberate: a validator/inspector is independently useful,
has a small and verifiable surface, and forces an honest, spec-faithful model
of the bitstream before any encoder decisions are made. Decoder planning is
tracked separately in [docs/DECODER-ROADMAP.md](./docs/DECODER-ROADMAP.md) and
[docs/DECODER-SUPPORT-STATUS.md](./docs/DECODER-SUPPORT-STATUS.md); those docs
do not change the current stub status.

And to be clear about what `splot` is **not** (yet): it is not a decoder — it
checks syntax and header-level conformance, it does not reconstruct pixels.
It is not an encoder — `encode`/`decode` exit with a clear error (the
bitstream **writer** foundation is library-only, validated by
`read(write(x)) == x` round-trips against the parser, with no public `encode`
command). And it is **not AV1**: the OBU header follows AV2 v1.0.0 § 5.2.2 (no
`obu_forbidden_bit`, no `obu_has_size_field`, no AV1 OBU type table), and AV1
bitstreams are out of scope.

## Quick start

```bash
rustup update stable        # or use the pinned toolchain in rust-toolchain.toml
cargo build --release
./target/release/splot --help
```

The toolchain is pinned to Rust **1.96.0**, edition **2024** (see
`rust-toolchain.toml`).

```bash
splot validate sample.av2              # raw Annex B or IVF input; human-readable report
splot validate sample.ivf --json       # machine-readable report (exit 1 if non-conformant)
splot validate sample.av2 --strict     # treat warnings as conformance failures
splot validate sample.av2 --max-diagnostics 20  # cap the listed diagnostics (presentation only; exit code unchanged)
splot validate sample.av2 --summary-only        # only the counts + conformance line (machine-friendly; exit code unchanged)
splot inspect sample.ivf --headers     # list OBUs and their headers
splot inspect sample.av2 --json        # per-OBU JSON records with parsed payload views
splot explain obu-header/global-xlayer-required  # describe a diagnostic rule id (--json, --list)
splot encode input.y4m -o output.av2   # not yet implemented (exits 1 with a clear error)
splot decode input.av2 -o output.y4m   # not yet implemented (exits 1 with a clear error)
```

`encode` and `decode` already accept `--threads auto|N` (default `auto`) to size
the worker pool, even though both remain unimplemented stubs that exit 1; the flag
selects the concurrency policy ahead of any real codec work. See
[docs/CONCURRENCY.md](./docs/CONCURRENCY.md).

`inspect` keeps stdout clean for machine output (logs go to stderr) and prints
every OBU it can parse even when the bitstream tail is malformed:

```console
$ splot inspect --headers conformant.av2
2 OBU(s)
OBU #0  @byte 1  size=1  type=OBU_TEMPORAL_DELIMITER(2)  ext=false  tlayer=0 mlayer=0 xlayer=31
OBU #1  @byte 3  size=11  type=OBU_SEQUENCE_HEADER(1)  ext=false  tlayer=0 mlayer=0 xlayer=0
```

Every finding is a rule id you can look up. `splot explain` turns a rule id into
its severity, spec section, and one-line summary — straight from the same
CI-enforced registry the validator emits, no bitstream needed — so `validate` and
`explain` compose:

```console
$ splot explain obu-header/global-xlayer-required
obu-header/global-xlayer-required
  severity: error
  section:  § 6.2.2
  summary:  OBU type requiring GLOBAL_XLAYER_ID uses a non-global obu_xlayer_id

Full registry: docs/VALIDATOR-DIAGNOSTICS.md
```

`--json` emits the same record as an object, `--list` prints every rule id
(sorted), and an unknown id exits `2` — never a panic — with same-namespace
suggestions:

```console
$ splot explain obu-header/nope
error: unknown rule id `obu-header/nope`; did you mean: obu-header/base-layer-only-types, obu-header/global-xlayer-required, … (run `splot explain --list` to see all)
```

For long reports, `--max-diagnostics N` caps the *listed* findings — the trailing
counts and the exit code always reflect the full report — and `--summary-only`
drops the per-finding lines for a machine-friendly digest:

```console
$ splot validate bad.av2 --max-diagnostics 1
[ERROR] obu-order/temporal-unit-missing-delimiter (§7.3.7) @byte 1: OBU_TEMPORAL_DELIMITER appears before a global OBU_TEMPORAL_DELIMITER starts the temporal unit
... 1 more diagnostic(s) not shown (--max-diagnostics 1)
2 error(s), 0 warning(s), 0 info
NOT conformant
```

## Project layout

```text
crates/splot-core      AV2 bitstream model + parsers (LEB128, OBU header, Annex B, IVF, headers)
crates/splot-parallel  approved concurrency primitives (local Rayon worker pool + bounded crossbeam queues)
crates/splot-recon     future reconstruction primitives (decoded frame/plane types; no decode yet)
crates/splot-decode    decoder diagnostic API + worker-pool scaffold (no byte-consuming decode yet)
crates/splot-validate  parser-driven conformance diagnostics (the validator)
crates/splot-encode    future encoder API (stub)
crates/splot-cli       thin `splot` binary
xtask                  project automation (ci, repo checks, generated docs)
fuzz                   cargo-fuzz target (outside the workspace)
```

Dependencies flow one way (`splot-core` depends on no other `splot-*` crate;
nothing depends on `splot-cli`), and that rule is itself a CI gate. See
[docs/ARCHITECTURE.md](./docs/ARCHITECTURE.md).

## Engineering discipline

One command is the acceptance gate:

```bash
cargo xtask ci
```

It runs the local acceptance pipeline: `fmt`, `clippy -D warnings`, tests,
doctests, rustdoc, spell-check (`typos`), unused dependencies
(`cargo-machete`), supply-chain policy (`cargo-deny`), license headers,
dependency direction, spec-mirror integrity, generated table drift,
feature-status drift, decoder-support drift, and the diagnostics registry. CI adds
Conventional Commits enforcement, the blocking per-PR fuzz smoke, a
supply-chain job, and workflow linting on top. Testing strategy and layers are
documented in [docs/TESTING.md](./docs/TESTING.md).

## Roadmap

Shipped: the Annex B + IVF + OBU header validator, OBU ordering and header-level
conformance, sequence/frame-header parsing, and the library-only **bit-writer
primitive** layer (`BitWriter`, the inverse of every reader primitive). In progress:
validator depth across the remaining spec sections, and the higher-level bitstream
writer (OBU header, payload writers, Annex B muxer) on top of the primitives. Next:
inspector snapshots and conformance vectors, AVM differential testing (with the
[AVM reference implementation](https://github.com/AOMediaCodec/avm) as the
oracle), staged decoder/reconstruction support for future encoder roundtrips, and
encoder experiments.

Validator details live in
[docs/VALIDATOR-ROADMAP.md](./docs/VALIDATOR-ROADMAP.md). Decoder scope and
status live in [docs/DECODER-ROADMAP.md](./docs/DECODER-ROADMAP.md) and the
generated [docs/DECODER-SUPPORT-STATUS.md](./docs/DECODER-SUPPORT-STATUS.md).
Encoder milestones will get their own roadmap once the validator and decoder
foundations justify it; until then the `ENC-*` rows in
[`docs/IMPLEMENTATION-MATRIX.toml`](./docs/IMPLEMENTATION-MATRIX.toml) are the
canonical encoder plan.

`splot` uses rav1e and SVT-AV1 as engineering references for future AV2
encoder architecture, not as sources of AV2 syntax or copied implementation
material. See
[docs/references/ENCODER-RESEARCH-NOTES.md](./docs/references/ENCODER-RESEARCH-NOTES.md),
[docs/references/RAV1E-SOURCE-MAP.md](./docs/references/RAV1E-SOURCE-MAP.md),
[docs/references/SVT-AV1-RESEARCH-MAPPING.md](./docs/references/SVT-AV1-RESEARCH-MAPPING.md),
and
[docs/references/THIRD-PARTY-NOTICES.md](./docs/references/THIRD-PARTY-NOTICES.md).

## Feature tracking

Implementation status is tracked in
[`docs/IMPLEMENTATION-MATRIX.toml`](./docs/IMPLEMENTATION-MATRIX.toml) — the
canonical source of truth — and rendered with:

```bash
cargo xtask feature-status            # aligned status table
cargo xtask check-feature-status      # fail on drift (also part of cargo xtask ci)
cargo xtask spec-coverage             # per-spec-section coverage summary
```

OpenSpec changes under [`openspec/`](./openspec/) describe intent; the matrix
is the canonical status. See
[docs/FEATURE-TRACKING.md](./docs/FEATURE-TRACKING.md).

## License

`splot` is source-available under the PolyForm Noncommercial License 1.0.0.

You may use, fork, modify, and redistribute this project only for
noncommercial purposes under the terms of the PolyForm Noncommercial License
1.0.0.

Commercial use is not permitted under this license and requires a separate
written commercial license from Bartosz Tomczyk.

Commercial use includes, without limitation:

- use by or for a for-profit company or other commercial organization;
- use in paid products, paid services, SaaS, hosted services, APIs, internal
  tools, or customer deliverables;
- use in CI/CD, QA, compliance, conformance testing, validation, benchmarking,
  or certification of commercial codecs, encoders, decoders, media files,
  products, or services;
- linking, embedding, wrapping, vendoring, containerizing, modifying,
  distributing, or operating any part of this project in a commercial
  workflow;
- using reports, diagnostics, validation results, or other outputs from this
  project to support commercial work.

For commercial licensing, contact: <bartekplus@gmail.com>.

This section is a plain-language summary. The full license text in
[LICENSE.md](./LICENSE.md) controls. Third-party materials remain under their
own licenses where explicitly stated.

OpenSpec-generated assistant integration files are MIT-licensed and isolated
to agent/tooling directories; the committed AV2 spec mirror is verbatim
AOMedia copyright material. See
[THIRD-PARTY-NOTICES.md](./docs/references/THIRD-PARTY-NOTICES.md).

## Contributing

`splot` is a solo-developer, source-available project and is **not accepting
external code contributions** at this time — please don't open pull requests.

If you hit a bug, a wrong or missing diagnostic, or a conformance gap, **open
an issue** instead: <https://github.com/splotlabs/splot/issues>. The repo
ships issue forms for [bug reports](./.github/ISSUE_TEMPLATE/bug.yml),
[AV2 features](./.github/ISSUE_TEMPLATE/av2-feature.yml), and
[conformance/vector/fuzz work](./.github/ISSUE_TEMPLATE/conformance.yml).

Developers and coding agents working in a fork should follow the canonical
guide in [AGENTS.md](./AGENTS.md).
