# splot

`splot` is a Rust toolkit for the **AV2** video codec: a bitstream **validator** and
**inspector** today, and an experimental **encoder** later.

> **Status: pre-alpha / validator-first.** The Annex B envelope parser, AV2 OBU
> header parser, and a structured header-conformance validator work today. The
> decoder and encoder are reserved API shapes only.

AV2 is tracked against the **AV2 Bitstream & Decoding Process Specification
v1.0.0** (Final Deliverable, **2026-05-28**): <https://av2.aomedia.org/v1.0.0/index.html>.

## Why validator-first?

A validator/inspector is independently useful, has a small and verifiable surface,
and forces an honest, spec-faithful model of the bitstream before any encoder
decisions are made. Every validator finding is structured data — a stable rule id,
severity, spec section, byte/bit offset, and message — not a log line.

This is **AV2**, not AV1: the OBU header follows AV2 v1.0.0 § 5.2.2 (no
`obu_forbidden_bit`, no `obu_has_size_field`, no AV1 OBU type table).

## Build / install

```bash
rustup update stable        # or use the pinned toolchain in rust-toolchain.toml
cargo build --release
./target/release/splot --help
```

The toolchain is pinned to Rust **1.96.0**, edition **2024** (see `rust-toolchain.toml`).

## Usage

```bash
splot validate sample.av2              # human-readable conformance report
splot validate sample.av2 --json       # machine-readable report (exit 1 if non-conformant)
splot inspect sample.av2 --headers     # list OBUs and their headers
splot encode input.y4m -o output.av2 --qp 120 --speed 6   # not yet implemented
splot decode input.av2 -o output.y4m                      # not yet implemented
```

Exit codes for `validate`/`inspect`: `0` success/conformant, `1` validation errors
or unparseable input, `2` I/O or CLI errors.

## Project layout

```text
crates/splot-core      AV2 bitstream model + parsers (LEB128, OBU header, Annex B)
crates/splot-validate  parser-driven conformance diagnostics
crates/splot-encode    future encoder API (stub)
crates/splot-cli       thin `splot` binary
xtask                  project automation (ci, repo checks)
fuzz                   cargo-fuzz target (outside the workspace)
```

## Roadmap

1. Annex B + OBU header validator. *(done)*
2. OBU ordering and header-level conformance.
3. Sequence/frame header parsing.
4. Inspector snapshots and conformance vectors.
5. AVM differential testing.
6. Encoder experiments.

See [docs/ENCODER-ROADMAP.md](./docs/ENCODER-ROADMAP.md) for milestones tied to
Feature IDs.

## Encoder research references

`splot` uses rav1e and SVT-AV1 as engineering references for future AV2 encoder architecture, not as
sources of AV2 syntax or copied implementation material. See:

- `docs/references/ENCODER-RESEARCH-NOTES.md`
- `docs/references/RAV1E-SOURCE-MAP.md`
- `docs/references/SVT-AV1-RESEARCH-MAPPING.md`
- `docs/references/THIRD-PARTY-NOTICES.md`

## Feature tracking

Implementation status is tracked in
[`docs/IMPLEMENTATION-MATRIX.toml`](./docs/IMPLEMENTATION-MATRIX.toml) — the
canonical source of truth — and rendered with:

```bash
cargo xtask feature-status            # aligned table
cargo xtask check-feature-status      # fail on drift (also part of cargo xtask ci)
cargo xtask spec-coverage             # coverage summary
```

OpenSpec changes under [`openspec/`](./openspec/) describe intent; the matrix is the
canonical status. See [docs/FEATURE-TRACKING.md](./docs/FEATURE-TRACKING.md) and the
generated [docs/FEATURE-STATUS.md](./docs/FEATURE-STATUS.md).

## License

`splot` is licensed under **PolyForm Noncommercial 1.0.0** (see [LICENSE.md](./LICENSE.md)).
It is free for noncommercial use. **Commercial use of any component** — validator,
inspector, decoder, encoder, CLI, docs, tests — requires a separate commercial
license: <bartekplus@gmail.com>.

## Contributing

See [CONTRIBUTING.md](./CONTRIBUTING.md) and the canonical agent/contributor guide
[AGENTS.md](./AGENTS.md).
