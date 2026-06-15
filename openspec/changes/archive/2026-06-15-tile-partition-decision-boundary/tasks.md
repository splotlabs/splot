## 1. Planning And Scope

- [x] 1.1 Validate the OpenSpec change strictly and branch from the merged `origin/main` state.
- [x] 1.2 Record planning subagent decisions in an agent log.

## 2. Core Implementation

- [x] 2.1 Add `crates/splot-decode/src/tile_payload/partition.rs` with typed partition outcomes, allowed/implied fact inputs, trace output, and crate-private typed errors.
- [x] 2.2 Implement the AV2 §5.20.3.2 branch order over caller-provided facts using the existing partition-entry `S()` helper and a one-bit `L(1)` literal read.
- [x] 2.3 Wire the module into `tile_payload.rs` without broadening public APIs or growing `tile_payload.rs` beyond a minimal module declaration.

## 3. Tests

- [x] 3.1 Add focused tests for early returns that do not advance symbols or mutate CDF rows.
- [x] 3.2 Add focused tests for conditional `S()` branch ordering, `Rect_Part_Table` mapping, and isolated `L(1)` consumption.
- [x] 3.3 Add focused negative/edge tests for invalid allowed/implied facts, selector/symbol/literal errors, transactionality, and deterministic repeat behavior.

## 4. Documentation And Status

- [x] 4.1 Update `docs/SPEC-MAPPING.md` with the AV2 §5.20.3.2 / §8.3.2 tile partition citation surface.
- [x] 4.2 Update `docs/DECODER-ROADMAP.md`, `docs/DECODER-SUPPORT-MATRIX.toml`, `docs/IMPLEMENTATION-MATRIX.toml`, and related notes for the new row and residual scope.
- [x] 4.3 Regenerate status/coverage artifacts and keep matrix drift gates green.

## 5. Review And Gates

- [x] 5.1 Run independent correctness/spec, security, and performance subagent reviews and record pass/block decisions.
- [x] 5.2 Run targeted gates: `cargo test -p splot-core symbol --locked`, `cargo test -p splot-core --test tables_spot --locked`, `cargo test -p splot-decode tile_payload --locked`, `cargo test -p splot-decode --locked`, and focused clippy for `splot-core`/`splot-decode`.
- [x] 5.3 Run status/repo gates: `openspec validate --all --no-interactive`, `cargo xtask feature-status`, `cargo xtask check-feature-status`, `cargo xtask check-decoder-support`, `cargo xtask check-decoder-conformance-coverage`, `cargo xtask check-dependency-direction`, `cargo xtask check-concurrency-policy`, and `cargo xtask ci`.

## 6. Archive And PR

- [x] 6.1 Archive the OpenSpec change, add archive agent log, and rerun required gates after archive.
- [ ] 6.2 Commit with a Conventional Commit subject, push, and open a ready PR that is not draft.
- [ ] 6.3 Wait for green CI, latest-head Codex review approval or clean verdict, Claude/human comments addressed, and zero unresolved threads before merge.
