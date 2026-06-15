## 1. Runtime Y4M Planning

- [x] 1.1 Split shared minimal-tier validation/frame construction out of `runtime_hash.rs` without changing existing hash behavior.
- [x] 1.2 Add `DecodeContext::decode_y4m_bytes` and a runtime Y4M adapter that writes one minimal-tier Y4M stream through `splot-recon::Y4mWriter`.
- [x] 1.3 Enforce complete Y4M stream `max_output_bytes` accounting before publication or success.
- [x] 1.4 Add `splot-decode` unit tests for exact minimal Y4M bytes, output-byte-limit failure, unsupported-tier failure, invalid timebase rejection, and thread determinism.

## 2. CLI Atomic Publication

- [x] 2.1 Add structured `decode/output-error` support for output serialization/publication failures.
- [x] 2.2 Implement same-directory temporary-file Y4M publication with source validation before temp creation, flush, file sync, rename, best-effort parent sync where supported, and cleanup.
- [x] 2.3 Preserve hash-mode `-o` no-touch behavior and current diagnostic exit behavior.
- [x] 2.4 Add CLI tests for explicit Y4M success, implicit Y4M success, deterministic bytes across thread policies, no-touch failure paths, source-diagnostic priority before output publication, output-error JSON/text diagnostics, and temp cleanup.

## 3. Documentation And Matrix

- [x] 3.1 Add `DECODE-Y4M-RUNTIME-OUTPUT` to `docs/IMPLEMENTATION-MATRIX.toml`.
- [x] 3.2 Add/update decoder support matrix rows for runtime Y4M output, CLI decode entrypoint, minimal tier, Y4M writer, limits, diagnostics, and output equivalence.
- [x] 3.3 Update `docs/DECODER-ROADMAP.md`, `docs/DECODER-FULL-CONFORMANCE.md`, and `docs/DECODER-DIAGNOSTICS.md` to describe the narrow Y4M support and remaining exclusions.
- [x] 3.4 Regenerate decoder support/status/coverage docs and verify drift checks.

## 4. Review And Gates

- [x] 4.1 Run focused tests: `cargo test -p splot-decode --locked` and `cargo test -p splot-cli --test decode_cli --locked`.
- [x] 4.2 Run feature/status/support/coverage checks: `cargo xtask feature-status`, `cargo xtask check-feature-status`, `cargo xtask check-decoder-support`, and `cargo xtask check-decoder-conformance-coverage`.
- [x] 4.3 Run independent correctness, security, performance, documentation, and reference-evidence reviews with pass/block decisions.
- [x] 4.4 Run `cargo xtask ci` and `openspec validate --all --no-interactive`.

## 5. Archive And PR

- [x] 5.1 Archive `decode-y4m-runtime-output` with `openspec archive decode-y4m-runtime-output --yes`.
- [x] 5.2 Rerun required gates after archive and commit the archived change.
- [x] 5.3 Open a ready (not draft) PR with tests, matrix rows, diagnostics, known exclusions, and review decisions.
- [ ] 5.4 Wait for CI, Claude/human feedback, and a latest Codex review verdict on the current head before merge.
