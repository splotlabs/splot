## 1. Feature Tracking And OpenSpec

- [x] 1.1 Author `openspec/changes/symbol-decoder-complete/` (proposal, design, tasks, `decoder-support` delta).
- [x] 1.2 Run `openspec validate symbol-decoder-complete --strict`.

## 2. Symbol Decoder Test Evidence (crates/splot-core/src/symbol.rs)

- [x] 2.1 Add extreme-value vectors: for every arity N = 2..8, payload `0x0000` decodes symbol 0 and `0xFFFF` decodes symbol N-1, with CDF update disabled and the row unchanged.
- [x] 2.2 Add exact CDF-update results at the minimum and maximum adaptation rates (hand-verified rows), pinning the `>> rate` shift extremes by value.
- [x] 2.3 Add a deep-negative-`SymbolMaxBits` run over a tiny payload: many `read_symbol` reads stay in range, never panic, and are deterministic across two fresh decoders.
- [x] 2.4 Add a property test over random valid CDF rows of arity N = 2..8: decoded symbol < N; updated row stays in `[1, 32767]` with count capped at 32; decoding is deterministic; disabled update leaves the row byte-for-byte unchanged.

## 3. Status And Generated Docs

- [x] 3.1 Promote the `symbol-decoder` row to `supported` in `docs/DECODER-SUPPORT-MATRIX.toml` with scoped notes and the new test list.
- [x] 3.2 Advance `AV2-8.2-SYMBOL-DECODER` stages (`parse`, `validate`, `decode_check`, `tests`) to `done` with proof in `docs/IMPLEMENTATION-MATRIX.toml` and update its notes.
- [x] 3.3 Update `docs/DECODER-ROADMAP.md` (mark the symbol decoder primitive complete in Phase 3).
- [x] 3.4 Regenerate `docs/DECODER-SUPPORT-STATUS.md`, `docs/FEATURE-STATUS.md`, `docs/SPEC-COVERAGE.md`, and `docs/DECODER-SPEC-COVERAGE.md` (the last regenerates identically — the coverage group stays `partial`).

## 4. Review And Gates

- [x] 4.1 Targeted gates: `cargo test -p splot-core symbol --locked`, `cargo test -p splot-core --test tables_spot --locked`, `cargo clippy -p splot-core --all-targets --all-features --locked -- -D warnings`, `cargo xtask check-decoder-support`, `cargo xtask check-feature-status`, `cargo xtask check-decoder-conformance-coverage`.
- [x] 4.2 Independent review: spec-exactness (VERDICT COMPLETE), status-honesty + test-evidence soundness (VERDICT PASS, update-rate rows independently recomputed; no blocking issues).
- [x] 4.3 `openspec validate --all --no-interactive` green; `cargo xtask ci` green.

## 5. Archive And PR

- [x] 5.1 `openspec archive symbol-decoder-complete --yes` and commit the archive in this branch.
- [x] 5.2 Re-run gates after archive.
- [x] 5.3 Open a ready, non-draft PR with Feature ID, scope, tests, reviewer decisions, and known exclusions.
