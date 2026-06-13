# Decoder Support Status

Generated from `docs/DECODER-SUPPORT-MATRIX.toml` by `cargo xtask decoder-support --format markdown`. Do not edit by hand.

Matrix version 1. Last reviewed 2026-06-13. 15 row(s).

## Status Counts

| Status | Rows |
|---|---:|
| `todo` | 7 |
| `partial` | 3 |
| `supported` | 4 |
| `unsupported-intentional` | 1 |
| `blocked` | 0 |

## Tier Counts

| Tier | Rows |
|---|---:|
| `encoder-reuse` | 1 |
| `foundation` | 9 |
| `tier0-plan` | 1 |
| `tier1-intra` | 4 |

## Rows

| ID | Name | Feature | Tier | Status | Spec Sections | Tests | Diagnostics | Local Evidence | Module |
|---|---|---|---|---|---|---|---|---|---|
| `decoder-roadmap` | Decoder roadmap | `DOC-DECODER-ROADMAP` | `foundation` | `supported` | none | cargo xtask check-decoder-support | none | none | `docs/DECODER-ROADMAP.md` |
| `decoder-support-matrix` | Decoder support matrix and generated status | `DOC-DECODER-SUPPORT-MATRIX` | `foundation` | `supported` | none | cargo xtask check-decoder-support<br>cargo xtask ci | none | none | `docs/DECODER-SUPPORT-MATRIX.toml` |
| `decoder-status-drift-gate` | Decoder support drift gate | `XTASK-DECODER-SUPPORT-STATUS` | `foundation` | `supported` | none | decoder_support::tests::check_decoder_support_detects_drift<br>cargo xtask check-decoder-support | none | none | `xtask/src/decoder_support.rs` |
| `decoder-diagnostic-registry` | Decoder diagnostic registry | `DOC-DECODER-DIAGNOSTICS` | `foundation` | `supported` | none | xtask/src/diagnostic_registry.rs::tests::decoder_registry_accepts_matching_source_and_doc<br>cargo xtask check-diagnostic-registry | none | none | `docs/DECODER-DIAGNOSTICS.md` |
| `cli-decode-entrypoint` | splot decode CLI entry point | `CLI-DECODE` | `foundation` | `unsupported-intentional` | 7.1 | crates/splot-cli/tests/cli.rs::decode_unsupported_text_mode_emits_stable_diagnostic<br>crates/splot-cli/tests/cli.rs::decode_unsupported_json_mode_emits_diagnostic_object<br>crates/splot-cli/tests/cli.rs::decode_unsupported_missing_input_does_not_touch_files | decode/unsupported-feature | none | `crates/splot-cli/src/commands/decode.rs` |
| `decode-limits-budget` | Decode limits and allocation budget | `DOC-DECODE-LIMITS-CONTRACT` | `foundation` | `partial` | 6.4.1<br>6.4.6<br>6.17.4.1<br>6.17.7.2<br>5.19<br>5.20<br>7.1<br>7.21<br>7.23 | openspec validate --all --no-interactive<br>cargo xtask check-decoder-support | decode/resource-limit (planned) | none | `docs/DECODER-ROADMAP.md` |
| `decoded-frame-plane-model` | Decoded frame and plane model | `DOC-DECODED-FRAME-PLANE-MODEL-CONTRACT` | `foundation` | `partial` | 6.4.1<br>6.17.4.1<br>6.17.4.4<br>7.1<br>7.21.1<br>7.21.2<br>7.21.5<br>7.21.6<br>7.23 | openspec validate --all --no-interactive<br>cargo xtask check-decoder-support | decode/resource-limit (planned) | none | `docs/DECODER-ROADMAP.md` |
| `deterministic-frame-hash` | Deterministic decoded-frame hash | `DOC-DETERMINISTIC-FRAME-HASH-CONTRACT` | `foundation` | `partial` | 5.17.12<br>6.16.13<br>7.21.1<br>7.21.2<br>7.21.7 | openspec validate --all --no-interactive<br>cargo xtask check-decoder-support | none | Archived local AVM/dav2d raw MD5 agreement for two tiny fixtures is recorded in openspec/changes/archive/2026-06-13-decoder-roadmap-matrix-boundary/agent-log.md; this is non-executable metadata only and not proof of splot hash implementation. | `docs/DECODER-ROADMAP.md` |
| `decode-stream-state` | Decode stream traversal and layer selection | none | `tier0-plan` | `todo` | 5.2.1<br>7.1<br>7.3<br>7.4 | none | decode/unsupported-feature (planned) | none | `planned` |
| `symbol-decoder` | Symbol and CDF decoder foundation | none | `tier1-intra` | `todo` | 8.2<br>8.3<br>9 | none | decode/unsupported-feature (planned) | none | `planned` |
| `tile-payload-decode` | Tile payload decode boundary | none | `tier1-intra` | `todo` | 5.20<br>7.1<br>8.3 | none | decode/unsupported-feature (planned) | none | `planned` |
| `intra-reconstruction` | Scalar intra reconstruction | none | `tier1-intra` | `todo` | 7.13<br>7.14<br>7.15 | none | decode/unsupported-feature (planned) | none | `planned` |
| `output-y4m` | Y4M output | none | `tier1-intra` | `todo` | 7.21 | none | none | none | `planned` |
| `reference-frame-store` | Reconstructed reference-frame store | none | `encoder-reuse` | `todo` | 7.23 | none | none | none | `planned` |
| `decode-fuzz-entrypoint` | Decode fuzz entry point | none | `foundation` | `todo` | none | none | none | none | `planned` |
