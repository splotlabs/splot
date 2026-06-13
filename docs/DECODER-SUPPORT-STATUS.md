# Decoder Support Status

Generated from `docs/DECODER-SUPPORT-MATRIX.toml` by `cargo xtask decoder-support --format markdown`. Do not edit by hand.

Matrix version 1. Last reviewed 2026-06-13. 14 row(s).

## Status Counts

| Status | Rows |
|---|---:|
| `todo` | 10 |
| `partial` | 0 |
| `supported` | 3 |
| `unsupported-intentional` | 1 |
| `blocked` | 0 |

## Tier Counts

| Tier | Rows |
|---|---:|
| `encoder-reuse` | 1 |
| `foundation` | 8 |
| `tier0-plan` | 1 |
| `tier1-intra` | 4 |

## Rows

| ID | Name | Feature | Tier | Status | Spec Sections | Tests | Diagnostics | Local Evidence | Module |
|---|---|---|---|---|---|---|---|---|---|
| `decoder-roadmap` | Decoder roadmap | `DOC-DECODER-ROADMAP` | `foundation` | `supported` | none | cargo xtask check-decoder-support | none | none | `docs/DECODER-ROADMAP.md` |
| `decoder-support-matrix` | Decoder support matrix and generated status | `DOC-DECODER-SUPPORT-MATRIX` | `foundation` | `supported` | none | cargo xtask check-decoder-support<br>cargo xtask ci | none | none | `docs/DECODER-SUPPORT-MATRIX.toml` |
| `decoder-status-drift-gate` | Decoder support drift gate | `XTASK-DECODER-SUPPORT-STATUS` | `foundation` | `supported` | none | decoder_support::tests::check_decoder_support_detects_drift<br>cargo xtask check-decoder-support | none | none | `xtask/src/decoder_support.rs` |
| `cli-decode-entrypoint` | splot decode CLI entry point | `CLI-DECODE` | `foundation` | `unsupported-intentional` | 7.1 | none | decode/unsupported-feature (planned) | none | `crates/splot-cli/src/commands/decode.rs` |
| `decode-limits-budget` | Decode limits and allocation budget | none | `foundation` | `todo` | 6.4.1<br>6.17.4.1<br>7.1 | none | decode/resource-limit (planned) | none | `planned` |
| `decoded-frame-plane-model` | Decoded frame and plane model | none | `foundation` | `todo` | 6.4.1<br>7.21 | none | none | none | `planned` |
| `deterministic-frame-hash` | Deterministic decoded-frame hash | none | `foundation` | `todo` | 6.16.13<br>7.21 | none | none | Local AVM/dav2d raw MD5 matched for existing tiny fixtures; see openspec/changes/decoder-roadmap-matrix-boundary/agent-log.md. | `planned` |
| `decode-stream-state` | Decode stream traversal and layer selection | none | `tier0-plan` | `todo` | 5.2.1<br>7.1<br>7.3<br>7.4 | none | decode/unsupported-feature (planned) | none | `planned` |
| `symbol-decoder` | Symbol and CDF decoder foundation | none | `tier1-intra` | `todo` | 8.2<br>8.3<br>9 | none | decode/unsupported-feature (planned) | none | `planned` |
| `tile-payload-decode` | Tile payload decode boundary | none | `tier1-intra` | `todo` | 5.20<br>7.1<br>8.3 | none | decode/unsupported-feature (planned) | none | `planned` |
| `intra-reconstruction` | Scalar intra reconstruction | none | `tier1-intra` | `todo` | 7.13<br>7.14<br>7.15 | none | decode/unsupported-feature (planned) | none | `planned` |
| `output-y4m` | Y4M output | none | `tier1-intra` | `todo` | 7.21 | none | none | none | `planned` |
| `reference-frame-store` | Reconstructed reference-frame store | none | `encoder-reuse` | `todo` | 7.23 | none | none | none | `planned` |
| `decode-fuzz-entrypoint` | Decode fuzz entry point | none | `foundation` | `todo` | none | none | none | none | `planned` |
