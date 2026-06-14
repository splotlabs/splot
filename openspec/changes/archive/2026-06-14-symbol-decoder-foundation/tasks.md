## 1. Planning And Branch Setup

- [x] 1.1 Record planning subagent findings in `agent-log.md`.
- [x] 1.2 Validate the OpenSpec change with `openspec validate symbol-decoder-foundation --strict`.
- [x] 1.3 Create the feature branch from current `origin/main` only after validation passes.

## 2. Core Symbol Decoder

- [x] 2.1 Add `crates/splot-core/src/symbol.rs` with SPDX header and crate-local § 8.2 docs.
- [x] 2.2 Implement `SymbolDecoder` initialization over a bounded tile payload slice, with signed `SymbolMaxBits` and no `sz * 8` overflow.
- [x] 2.3 Implement `read_bool()`, `read_literal(n)`, `read_symbol(cdf)`, CDF update enable/disable behavior, and `finish()` / `exit_symbol()` validation.
- [x] 2.4 Add typed `splot-core::Error` variants/kinds for invalid symbol CDF rows and invalid symbol decoder state.
- [x] 2.5 Export the symbol module from `splot-core` and keep `RangeEncoder` unimplemented.

## 3. Tests And Robustness

- [x] 3.1 Add positive tests for init boundaries, boolean/literal reads, symbol reads, CDF update disabled/enabled, and count saturation.
- [x] 3.2 Add negative/edge tests for malformed CDF length/range/order/rate/count, invalid literal width, empty/short payloads, `SymbolMaxBits < -14`, missing trailing one bit, and nonzero padding.
- [x] 3.3 Add a no-panic property test or bounded arbitrary-input test for symbol decoder operations.
- [x] 3.4 Run focused checks: `cargo test -p splot-core symbol --locked`, `cargo test -p splot-core tables_spot --locked`, and any additional touched-crate checks.

## 4. Docs, Matrix, And OpenSpec

- [x] 4.1 Add `AV2-8.2-SYMBOL-DECODER` to `docs/IMPLEMENTATION-MATRIX.toml` with proof commands and status.
- [x] 4.2 Update `docs/DECODER-SUPPORT-MATRIX.toml` row `symbol-decoder` to `partial` with feature ID, module, tests, and non-goal notes.
- [x] 4.3 Update `docs/DECODER-ROADMAP.md` and regenerate `docs/DECODER-SUPPORT-STATUS.md`, `docs/FEATURE-STATUS.md`, and `docs/SPEC-COVERAGE.md`.
- [x] 4.4 Verify no AVM/dav2d source, snippets, deps, wrappers, scripts, CI jobs, required tools, or local absolute paths were introduced.

## 5. Review, Archive, And Gates

- [x] 5.1 Run mandatory review subagents: reviewer, security-reviewer, spec-conformance-reviewer, and encoder-impact-reviewer.
- [x] 5.2 Fix or explicitly close every review finding in `agent-log.md`.
- [x] 5.3 Run `openspec validate symbol-decoder-foundation --strict`, archive the change, and verify `openspec/specs/` received the expected delta.
- [x] 5.4 Run final local gates: `openspec validate --all --no-interactive`, `cargo xtask check-feature-status`, `cargo xtask check-decoder-support`, `cargo xtask check-conventional-commits`, and `cargo xtask ci`.

## 6. PR And Merge Discipline

- [ ] 6.1 Open a ready PR, not draft, with scope, non-goals, tests, matrix/docs, subagent sign-offs, and AVM/dav2d boundary statement.
- [ ] 6.2 Wait for GitHub checks to pass.
- [ ] 6.3 Request `@codex review` and wait for Codex completion on the latest PR head, not just an `eyes` reaction.
- [ ] 6.4 After every code-changing push, request Codex review again and wait for completion on the new head before merging.
- [ ] 6.5 Merge only after green checks, latest-head Codex completion, archived OpenSpec, and exact head-SHA guard.
