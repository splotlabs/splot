## 1. Planning and Scope

- [x] 1.1 Record planning subagents, Feature ID, and local-reference boundary in
  `agent-log.md`.
- [x] 1.2 Validate OpenSpec proposal/design/spec deltas with
  `openspec validate decode-unsupported-diagnostic-api --strict`.

## 2. Library Diagnostic API

- [x] 2.1 Add documented `DecodeDiagnostic`, `DecodeSeverity`,
  `UNSUPPORTED_FEATURE_DIAGNOSTIC`, and `unsupported_feature_diagnostic()` to
  `splot-decode` without adding dependencies.
- [x] 2.2 Add focused `splot-decode` unit tests for stable field values and
  severity spelling.

## 3. CLI Wiring and Dependency Direction

- [x] 3.1 Add `splot-cli -> splot-decode` to Cargo and the dependency-direction
  allow-list.
- [x] 3.2 Update `splot decode` to render the library-owned descriptor while
  preserving current text, JSON, exit-code, and no-I/O behavior.

## 4. Docs and Matrix

- [x] 4.1 Update architecture/agent docs for the new CLI-to-decode dependency
  edge without touching the Claude review workflow.
- [x] 4.2 Update decoder diagnostics docs, decoder support matrix, and
  implementation matrix for `DECODE-UNSUPPORTED-DIAGNOSTIC-API`.
- [x] 4.3 Regenerate `docs/DECODER-SUPPORT-STATUS.md`,
  `docs/FEATURE-STATUS.md`, and `docs/SPEC-COVERAGE.md`.

## 5. Verification

- [x] 5.1 Run focused checks: `cargo test -p splot-decode --locked`,
  `cargo test -p splot-cli --test decode_cli --locked`,
  `cargo xtask check-dependency-direction`,
  `cargo xtask check-diagnostic-registry`,
  `cargo xtask check-decoder-support`,
  `cargo xtask check-feature-status`,
  `openspec validate --all --no-interactive`, `cargo machete --with-metadata`,
  and `git diff --check`.
- [x] 5.2 Run `cargo xtask ci`.

## 6. Review, Archive, and PR

- [x] 6.1 Run required review subagents: reviewer, security-reviewer,
  spec-conformance-reviewer, and encoder-impact-reviewer.
- [x] 6.2 Resolve or record every review finding in `agent-log.md`.
- [x] 6.3 Archive the OpenSpec change with
  `openspec archive decode-unsupported-diagnostic-api --yes`.
- [x] 6.4 Re-run focused checks and `cargo xtask ci` after archive.
- [ ] 6.5 Commit, push, open PR, wait for CI/review, and merge only when green.
