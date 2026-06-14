# Decoder Support Status

Generated from `docs/DECODER-SUPPORT-MATRIX.toml` by `cargo xtask decoder-support --format markdown`. Do not edit by hand.

Matrix version 1. Last reviewed 2026-06-14. 23 row(s).

## Status Counts

| Status | Rows |
|---|---:|
| `todo` | 6 |
| `partial` | 4 |
| `supported` | 12 |
| `unsupported-intentional` | 1 |
| `blocked` | 0 |

## Tier Counts

| Tier | Rows |
|---|---:|
| `encoder-reuse` | 1 |
| `foundation` | 17 |
| `tier0-plan` | 1 |
| `tier1-intra` | 4 |

## Rows

| ID | Name | Feature | Tier | Status | Spec Sections | Tests | Diagnostics | Local Evidence | Module |
|---|---|---|---|---|---|---|---|---|---|
| `decoder-roadmap` | Decoder roadmap | `DOC-DECODER-ROADMAP` | `foundation` | `supported` | none | cargo xtask check-decoder-support | none | none | `docs/DECODER-ROADMAP.md` |
| `decoder-support-matrix` | Decoder support matrix and generated status | `DOC-DECODER-SUPPORT-MATRIX` | `foundation` | `supported` | none | cargo xtask check-decoder-support<br>cargo xtask ci | none | none | `docs/DECODER-SUPPORT-MATRIX.toml` |
| `decoder-status-drift-gate` | Decoder support drift gate | `XTASK-DECODER-SUPPORT-STATUS` | `foundation` | `supported` | none | decoder_support::tests::check_decoder_support_detects_drift<br>cargo xtask check-decoder-support | none | none | `xtask/src/decoder_support.rs` |
| `local-reference-evidence-manifest` | Portable local-reference evidence manifest | `XTASK-LOCAL-REFERENCE-EVIDENCE-MANIFEST` | `foundation` | `supported` | none | xtask/src/reference_evidence/tests.rs::tests<br>cargo test -p xtask reference_evidence --locked<br>cargo xtask check-reference-evidence<br>cargo xtask check-decoder-support | none | none | `xtask/src/reference_evidence.rs` |
| `decoder-crate-scaffolding` | Decoder and reconstruction crate scaffolding | `INFRA-DECODER-CRATE-SCAFFOLDING` | `foundation` | `supported` | none | cargo check -p splot-recon --locked<br>cargo check -p splot-decode --locked<br>cargo xtask check-dependency-direction<br>cargo xtask check-decoder-support | none | none | `crates/splot-recon/src/lib.rs; crates/splot-decode/src/lib.rs` |
| `decode-runtime-context` | Decode runtime context and concurrency contract | `INFRA-PARALLEL-RUNTIME-POLICY` | `foundation` | `supported` | none | crates/splot-decode/src/context.rs::tests<br>crates/splot-cli/tests/decode_cli.rs::decode_threads_fixed_is_accepted_emits_unsupported<br>crates/splot-cli/tests/decode_cli.rs::decode_threads_auto_is_accepted<br>crates/splot-cli/tests/decode_cli.rs::decode_threads_invalid_is_usage_error<br>cargo test -p splot-decode --locked<br>cargo test -p splot-cli decode_threads --locked<br>cargo xtask check-concurrency-policy<br>cargo xtask check-dependency-direction<br>cargo xtask check-decoder-support | none | none | `crates/splot-decode/src/context.rs; crates/splot-decode/src/runtime.rs; docs/DECODER-ROADMAP.md; docs/CONCURRENCY.md` |
| `decoded-frame-plane-runtime-types` | Decoded frame and plane runtime types | `INFRA-RECON-FRAME-PLANE-TYPES` | `foundation` | `supported` | 6.4.1<br>6.17.4.1<br>6.17.4.4<br>7.1<br>7.21.1<br>7.21.2<br>7.23 | crates/splot-recon/src/format.rs::tests<br>crates/splot-recon/src/geometry.rs::tests<br>crates/splot-recon/src/plane.rs::tests<br>crates/splot-recon/src/frame.rs::tests<br>cargo test -p splot-recon --locked<br>cargo clippy -p splot-recon --all-targets --locked -- -D warnings<br>cargo xtask check-dependency-direction<br>cargo xtask check-decoder-support | none | none | `crates/splot-recon/src/format.rs; crates/splot-recon/src/geometry.rs; crates/splot-recon/src/plane.rs; crates/splot-recon/src/frame.rs` |
| `decode-unsupported-diagnostic-api` | Decode unsupported diagnostic API | `DECODE-UNSUPPORTED-DIAGNOSTIC-API` | `foundation` | `supported` | 7.1 | crates/splot-decode/src/lib.rs::tests::unsupported_feature_diagnostic_has_stable_fields<br>crates/splot-decode/src/lib.rs::tests::unsupported_feature_diagnostic_function_returns_public_descriptor<br>crates/splot-decode/src/lib.rs::tests::decode_severity_displays_stable_spelling<br>cargo test -p splot-decode --locked<br>cargo xtask check-diagnostic-registry<br>cargo xtask check-decoder-support | decode/unsupported-feature | none | `crates/splot-decode/src/lib.rs` |
| `decoder-diagnostic-registry` | Decoder diagnostic registry | `DOC-DECODER-DIAGNOSTICS` | `foundation` | `supported` | none | xtask/src/diagnostic_registry.rs::tests::decoder_registry_accepts_matching_source_and_doc<br>cargo xtask check-diagnostic-registry | none | none | `docs/DECODER-DIAGNOSTICS.md` |
| `cli-decode-entrypoint` | splot decode CLI entry point | `CLI-DECODE` | `foundation` | `unsupported-intentional` | 7.1 | crates/splot-cli/tests/decode_cli.rs::decode_unsupported_text_mode_emits_stable_diagnostic<br>crates/splot-cli/tests/decode_cli.rs::decode_unsupported_json_mode_emits_diagnostic_object<br>crates/splot-cli/tests/decode_cli.rs::decode_unsupported_missing_input_does_not_touch_files | decode/unsupported-feature | none | `crates/splot-cli/src/commands/decode.rs; crates/splot-decode/src/lib.rs` |
| `cli-decode-hash-output-contract` | splot decode hash output CLI contract | `CLI-DECODE-HASH-OUTPUT` | `foundation` | `partial` | 7.1<br>7.21 | crates/splot-cli/tests/decode_cli.rs::decode_hash_output_format_emits_unsupported_text_without_output_path<br>crates/splot-cli/tests/decode_cli.rs::decode_hash_output_format_missing_input_does_not_touch_files<br>crates/splot-cli/tests/decode_cli.rs::decode_hash_output_format_json_emits_same_diagnostic<br>crates/splot-cli/tests/decode_cli.rs::decode_invalid_output_format_is_usage_error<br>crates/splot-cli/tests/decode_cli.rs::decode_hash_output_format_with_output_path_does_not_touch_file<br>crates/splot-cli/tests/decode_cli.rs::decode_without_output_selection_is_usage_error<br>crates/splot-cli/tests/decode_cli.rs::decode_explicit_y4m_output_format_requires_output_path<br>crates/splot-cli/tests/decode_cli.rs::decode_explicit_y4m_output_format_matches_implicit_no_touch_behavior | decode/unsupported-feature | none | `crates/splot-cli/src/commands/decode.rs` |
| `decode-limits-budget` | Decode limits and allocation budget | `DOC-DECODE-LIMITS-CONTRACT` | `foundation` | `partial` | 4.11.6<br>Annex B.2<br>Annex B.3<br>5.2.1<br>6.4.1<br>6.4.6<br>5.18.4.1<br>6.17.4.1<br>5.18.4.4<br>6.17.4.4<br>5.18.7.2<br>6.17.7.2<br>5.19<br>6.18<br>5.20.1<br>6.19.1<br>7.1<br>7.21<br>7.23 | openspec validate --all --no-interactive<br>cargo xtask check-decoder-support | decode/resource-limit (planned) | none | `docs/DECODER-ROADMAP.md` |
| `decode-limits-runtime-api` | Decode limits runtime API | `DECODE-LIMITS-RUNTIME-API` | `foundation` | `supported` | 4.11.6<br>Annex B.2<br>Annex B.3<br>5.2.1<br>6.4.1<br>6.4.6<br>5.18.4.1<br>6.17.4.1<br>5.18.4.4<br>6.17.4.4<br>5.18.7.2<br>6.17.7.2<br>5.19<br>6.18<br>5.20.1<br>6.19.1<br>7.1<br>7.21<br>7.23 | crates/splot-decode/src/limits.rs::tests<br>crates/splot-decode/src/lib.rs::tests::unsupported_feature_diagnostic_has_stable_fields<br>cargo test -p splot-decode --locked<br>cargo clippy -p splot-decode --all-targets --locked -- -D warnings<br>cargo xtask check-diagnostic-registry<br>cargo xtask check-dependency-direction<br>cargo xtask check-decoder-support | none | none | `crates/splot-decode/src/limits.rs` |
| `decoded-frame-plane-model` | Decoded frame and plane model | `DOC-DECODED-FRAME-PLANE-MODEL-CONTRACT` | `foundation` | `supported` | 6.4.1<br>6.17.4.1<br>6.17.4.4<br>7.1<br>7.21.1<br>7.21.2<br>7.21.5<br>7.21.6<br>7.23 | openspec validate --all --no-interactive<br>cargo test -p splot-recon --locked<br>cargo xtask check-decoder-support | none | none | `docs/DECODER-ROADMAP.md` |
| `deterministic-frame-hash` | Deterministic decoded-frame hash | `RECON-HASH-INPUT-SERIALIZATION` | `foundation` | `partial` | 5.17.12<br>6.16.13<br>7.21.1<br>7.21.2<br>7.21.7 | crates/splot-recon/src/hash_input.rs::tests<br>cargo test -p splot-recon --locked<br>cargo clippy -p splot-recon --all-targets --locked -- -D warnings<br>cargo xtask check-dependency-direction<br>cargo xtask check-decoder-support | none | Archived local AVM/dav2d raw MD5 agreement for two tiny fixtures is recorded in openspec/changes/archive/2026-06-13-decoder-roadmap-matrix-boundary/agent-log.md; this is non-executable metadata only and not proof of splot hash implementation. | `crates/splot-recon/src/hash_input.rs` |
| `minimal-decode-tier-contract` | Minimal decode tier contract | `DOC-MINIMAL-DECODE-TIER-CONTRACT` | `foundation` | `partial` | Annex B.2<br>Annex B.3<br>5.2<br>6.2<br>6.4.1<br>6.17.2<br>6.17.4.1<br>6.17.7.2<br>6.18<br>6.19.1<br>7.1<br>7.2<br>7.3<br>7.4<br>7.21<br>7.23<br>Annex A.2<br>Annex A.5 | openspec validate --all --no-interactive<br>cargo xtask check-decoder-support | decode/unsupported-feature (planned for streams outside the supported tier)<br>decode/resource-limit (planned) | none | `docs/DECODER-ROADMAP.md` |
| `decode-stream-state` | Decode stream traversal and layer selection | none | `tier0-plan` | `todo` | 5.2.1<br>7.1<br>7.3<br>7.4 | none | decode/unsupported-feature (planned) | none | `planned` |
| `symbol-decoder` | Symbol and CDF decoder foundation | none | `tier1-intra` | `todo` | 8.2<br>8.3<br>9 | none | decode/unsupported-feature (planned) | none | `planned` |
| `tile-payload-decode` | Tile payload decode boundary | none | `tier1-intra` | `todo` | 5.20<br>7.1<br>8.3 | none | decode/unsupported-feature (planned) | none | `planned` |
| `intra-reconstruction` | Scalar intra reconstruction | none | `tier1-intra` | `todo` | 7.13<br>7.14<br>7.15 | none | decode/unsupported-feature (planned) | none | `planned` |
| `output-y4m` | Y4M output | none | `tier1-intra` | `todo` | 7.21 | none | none | none | `planned` |
| `reference-frame-store` | Reconstructed reference-frame store | `RECON-REFERENCE-FRAME-STORE` | `encoder-reuse` | `supported` | 3<br>5.4.6<br>6.4.6<br>7.23 | crates/splot-recon/src/reference.rs::tests<br>cargo test -p splot-recon --locked<br>cargo clippy -p splot-recon --all-targets --locked -- -D warnings<br>cargo xtask check-dependency-direction<br>cargo xtask check-decoder-support | none | none | `crates/splot-recon/src/reference.rs` |
| `decode-fuzz-entrypoint` | Decode fuzz entry point | none | `foundation` | `todo` | none | none | none | none | `planned` |
