## 1. Runtime Raw Output

- [x] 1.1 Add `DECODE-MINIMAL-RAW-RUNTIME-OUTPUT` to `docs/IMPLEMENTATION-MATRIX.toml`.
- [x] 1.2 Add a `decode-minimal-raw-runtime-output` row to `docs/DECODER-SUPPORT-MATRIX.toml`.
- [x] 1.3 Add a `DecodeContext::decode_raw_bytes` runtime API that buffers the complete minimal raw sample byte stream before writing to the caller-provided writer.
- [x] 1.4 Add focused `splot-decode` tests for raw success, zero-timebase success, out-of-tier rejection, output byte limits, writer I/O errors, and thread determinism.

## 2. CLI Raw Publication

- [x] 2.1 Add `raw` to `splot decode --output-format` with required `-o`.
- [x] 2.2 Reuse/generalize the same-directory temp-file publication flow for raw output while preserving existing Y4M diagnostics.
- [x] 2.3 Add CLI tests for explicit raw success, existing-file replacement, no-touch source failures, missing parent source-diagnostic precedence, output errors, and thread determinism.

## 3. Fuzzing

- [x] 3.1 Add Feature ID `CONF-DECODE-RUNTIME-RAW-FUZZ` to `docs/IMPLEMENTATION-MATRIX.toml`.
- [x] 3.2 Add and register `fuzz/fuzz_targets/decode_runtime_raw_bytes.rs`.
- [x] 3.3 Update CI corpus seeding comments/targets for the new raw runtime fuzz target.

## 4. Documentation And Status

- [x] 4.1 Update `docs/DECODER-FULL-CONFORMANCE.md` and `docs/DECODER-ROADMAP.md` to describe supported minimal raw output without overclaiming broad decoder support.
- [x] 4.2 Regenerate `docs/DECODER-SUPPORT-STATUS.md`, `docs/FEATURE-STATUS.md`, `docs/SPEC-COVERAGE.md`, and `docs/DECODER-SPEC-COVERAGE.md` as required.
- [x] 4.3 Run `cargo xtask check-decoder-support`, `cargo xtask check-feature-status`, and `cargo xtask check-decoder-conformance-coverage`.

## 5. Validation, Review, Archive

- [x] 5.1 Run targeted gates: `cargo fmt --all -- --check`, `cargo test -p splot-decode runtime_raw --locked`, `cargo test -p splot-cli --test decode_raw_cli --locked`, `cargo check --manifest-path fuzz/Cargo.toml --bins --locked`, and `cargo xtask check-fuzz-targets`.
- [x] 5.2 Run `openspec validate decode-minimal-tier-raw-output --strict` and `openspec validate --all --no-interactive`.
- [x] 5.3 Run independent correctness, security/reference, and performance/documentation reviews.
- [x] 5.4 Run `cargo xtask ci`.
- [x] 5.5 Archive `decode-minimal-tier-raw-output` with `openspec archive decode-minimal-tier-raw-output --yes`, then re-run validation gates.
- [ ] 5.6 Commit, push, open a ready PR, and wait for CI plus Codex/Claude/human review before merge.
