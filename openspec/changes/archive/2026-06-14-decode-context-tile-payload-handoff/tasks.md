## 1. Planning And Boundaries

- [x] 1.1 Validate the OpenSpec proposal, design, spec delta, and agent log for `decode-context-tile-payload-handoff`.
- [x] 1.2 Record planning, reference, API, spec, security/performance, and PR #113/#114 review carry-forward findings in `agent-log.md`.

## 2. Context Handoff Implementation

- [x] 2.1 Add a crate-private `DecodeContext` tile-payload planning method that calls the existing boundary inside `WorkerPool::install`.
- [x] 2.2 Replace the stale tile-payload module dead-code allowance with conditional, reasoned non-test allowances documenting that runtime decode fact derivation is still future work.
- [x] 2.3 Keep all tile-payload input, plan, work-unit, and error types crate-private; add no public tile-payload API.

## 3. Tests

- [x] 3.1 Update deterministic worker-pool tests to call the tile-payload boundary through `DecodeContext` rather than manually calling `ctx.pool().install(...)`.
- [x] 3.2 Preserve existing limit, malformed, unsupported, CDF, no-panic, and work-unit tests.
- [x] 3.3 Run focused `splot-decode` tests and concurrency/dependency checks.

## 4. Documentation And Status

- [x] 4.1 Update `docs/IMPLEMENTATION-MATRIX.toml`, `docs/DECODER-SUPPORT-MATRIX.toml`, and `docs/DECODER-ROADMAP.md` for `DECODE-CONTEXT-TILE-PAYLOAD-HANDOFF`.
- [x] 4.2 Regenerate decoder support/status docs and feature/spec status docs required by repo checks.
- [x] 4.3 Confirm no AVM/dav2d source, snippets, binaries, submodules, dependencies, wrappers, scripts, CI jobs, required `xtask` commands, or mandatory tests were added.

## 5. Validation And Review

- [x] 5.1 Run `openspec validate decode-context-tile-payload-handoff --strict`.
- [x] 5.2 Run `cargo test -p splot-decode tile_payload --locked`.
- [x] 5.3 Run `cargo clippy -p splot-decode --all-targets --all-features --locked -- -D warnings`.
- [x] 5.4 Run `cargo xtask check-feature-status`, `cargo xtask check-decoder-support`, `cargo xtask check-dependency-direction`, `cargo xtask check-concurrency-policy`, and `cargo xtask ci`.
- [x] 5.5 Complete required review-agent passes, fix or document every finding, and update `agent-log.md`.

## 6. Archive And PR

- [x] 6.1 Archive the completed OpenSpec change and verify the delta folded into `openspec/specs/`.
- [ ] 6.2 Commit, push, and open a ready PR. Do not make it draft.
- [ ] 6.3 Wait for CI green and latest-head Codex review completion before any merge action. Treat `eyes` as in-progress, not green.
