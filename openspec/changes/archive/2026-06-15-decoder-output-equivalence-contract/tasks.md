## 1. Contract Documents

- [x] 1.1 Add `DOC-DECODER-OUTPUT-EQUIVALENCE-CONTRACT` to `docs/IMPLEMENTATION-MATRIX.toml` with docs-only proof.
- [x] 1.2 Add a `decoder-output-equivalence-contract` row to `docs/DECODER-SUPPORT-MATRIX.toml`.
- [x] 1.3 Update `docs/DECODER-FULL-CONFORMANCE.md` with exact output-variant, hash, raw/Y4M, and atomic output semantics.
- [x] 1.4 Update `docs/DECODER-ROADMAP.md` to point future runtime decode/output work at the output equivalence contract.

## 2. Generated Status

- [x] 2.1 Regenerate `docs/DECODER-SUPPORT-STATUS.md`.
- [x] 2.2 Regenerate feature/spec status documents when matrix checks require it.
- [x] 2.3 Confirm `docs/DECODER-SPEC-COVERAGE.md` remains drift-free and does not overclaim runtime output support.

## 3. Validation

- [x] 3.1 Run `openspec validate decoder-output-equivalence-contract --strict`.
- [x] 3.2 Run `cargo xtask check-decoder-support`.
- [x] 3.3 Run `cargo xtask check-decoder-conformance-coverage`.
- [x] 3.4 Run `cargo xtask check-feature-status`.
- [x] 3.5 Run `openspec validate --all --no-interactive`.
- [x] 3.6 Run `cargo xtask ci`.

## 4. Review And Archive

- [x] 4.1 Run independent correctness, security/reference, and performance/documentation reviews.
- [x] 4.2 Address or explicitly record every review finding.
- [x] 4.3 Archive `decoder-output-equivalence-contract` with `openspec archive decoder-output-equivalence-contract --yes`.
- [x] 4.4 Re-run validation gates after archive.
- [ ] 4.5 Commit, push, open a ready PR, and wait for CI plus Codex/Claude review.
