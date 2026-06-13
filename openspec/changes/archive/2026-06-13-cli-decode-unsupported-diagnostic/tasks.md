## 1. CLI Diagnostic

- [x] 1.1 Add a private decode diagnostic payload with stable `rule_id`, `severity`, `spec_section`, `matrix_row`, `feature_id`, `message`, and `remediation` fields.
- [x] 1.2 Add `--json` to `splot decode`.
- [x] 1.3 Render the unsupported diagnostic in text mode and JSON mode.
- [x] 1.4 Keep `splot decode` from reading input, creating output, invoking external tools, or changing the dependency graph.

## 2. Tests

- [x] 2.1 Add a text-mode CLI test for `decode/unsupported-feature`, `7.1`, `cli-decode-entrypoint`, `CLI-DECODE`, empty stdout, exit code `1`, and untouched output path.
- [x] 2.2 Add a JSON-mode CLI test that parses the diagnostic object and asserts the stable fields, empty stderr, exit code `1`, and untouched output path.
- [x] 2.3 Add a missing-input CLI test proving unsupported decode exits `1` without reading input or creating output.

## 3. Docs And Status

- [x] 3.1 Update `docs/DECODER-SUPPORT-MATRIX.toml` for the `cli-decode-entrypoint` diagnostic proof without marking actual decode as supported.
- [x] 3.2 Regenerate `docs/DECODER-SUPPORT-STATUS.md`.
- [x] 3.3 Update `docs/IMPLEMENTATION-MATRIX.toml`, `docs/FEATURE-STATUS.md`, and `docs/SPEC-COVERAGE.md` for `CLI-DECODE` proof.
- [x] 3.4 Update decoder roadmap/testing docs if the shipped text or JSON diagnostic contract changes reader-facing behavior.

## 4. Verification And Review

- [x] 4.1 Run `openspec validate cli-decode-unsupported-diagnostic --strict`.
- [x] 4.2 Run `cargo test -p splot-cli --test cli decode_unsupported --locked`.
- [x] 4.3 Run `cargo xtask check-decoder-support`.
- [x] 4.4 Run `cargo xtask check-feature-status`.
- [x] 4.5 Run `cargo xtask ci`.
- [x] 4.6 Record subagent review sign-offs and AVM/dav2d boundary evidence in `agent-log.md`.
