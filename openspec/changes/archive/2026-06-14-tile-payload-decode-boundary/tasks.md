## 1. Planning And Branch Setup

- [x] 1.1 Record all planning subagent findings in `agent-log.md`.
- [x] 1.2 Validate the OpenSpec change with `openspec validate tile-payload-decode-boundary --strict`.
- [x] 1.3 Create a feature branch from current `origin/main` only after validation passes.

## 2. Decode Tile Payload Boundary

- [x] 2.1 Add Feature ID `DECODE-TILE-PAYLOAD-BOUNDARY` to `docs/IMPLEMENTATION-MATRIX.toml`.
- [x] 2.2 Add crate-private `crates/splot-decode/src/tile_payload.rs` using existing `splot-core` tile framing and § 8.2 symbol-decoder handoff contracts; do not export tile plan structs from `splot-decode::lib`.
- [x] 2.2a Model deterministic tile work units with source OBU/frame identity, tile number, tile row/column or MI range where available, payload offset/length, and frame/layer selection metadata for future encoder reconstruction.
- [x] 2.3 Enforce `DecodeLimits::max_tile_count` and `DecodeLimits::max_tile_payload_bytes` before retaining tile records, slicing payload bytes, or constructing symbol-decoder state.
- [x] 2.4 Use checked arithmetic for tile byte-range and absolute-offset derivations before any `usize` conversion or slice indexing.
- [x] 2.5 Return structured `decode/unsupported-feature` metadata for the `decode_tile()` / § 8.3 boundary with matrix row `tile-payload-decode`, Feature ID `DECODE-TILE-PAYLOAD-BOUNDARY`, tile number, byte offset, and stable reason.
- [x] 2.6 Keep bridge, BRU-inactive, inter-only, CDF copyback/averaging, frame wrapup, reconstruction, hashes, runtime Y4M, and reference refresh unsupported with explicit metadata rather than silent assumptions.

## 3. Tests And Robustness

- [x] 3.1 Add positive unit tests for a single nonzero tile and unsupported multi-tile minimal-tier gating that record tile index/order, payload byte range, `tileSize`, and unsupported boundary metadata.
- [x] 3.2 Add negative tests for zero-size non-bridge tiles, truncated/overflowing tile ranges, `tg_end < tg_start`, offset overflow, and huge constructed tile ranges; prove no panic and no unbounded allocation.
- [x] 3.3 Add limit tests for `max_tile_count`, `max_tile_payload_bytes`, exact-limit acceptance, and one-over-limit rejection.
- [x] 3.4 Add tests proving `exit_symbol()`, CDF copyback/averaging, frame wrapup, output, and reference mutation are deferred until real `decode_tile()` traversal exists.
- [x] 3.5 Prove deterministic tile work-unit metadata across `ThreadCount::Auto`, one worker, and a fixed positive worker count if the boundary is reachable through `DecodeContext`.
- [x] 3.6 Run focused checks: `cargo test -p splot-decode tile_payload --locked`, `cargo clippy -p splot-decode --all-targets --all-features --locked -- -D warnings`, and any touched-crate tests.

## 4. Docs, Matrix, And OpenSpec

- [x] 4.1 Update `docs/DECODER-SUPPORT-MATRIX.toml` row `tile-payload-decode` to `partial` with feature ID, module, tests, diagnostics, and non-goal notes.
- [x] 4.2 Update `docs/DECODER-ROADMAP.md` and `docs/DECODER-DIAGNOSTICS.md` for the new tile-payload unsupported boundary.
- [x] 4.3 Regenerate `docs/DECODER-SUPPORT-STATUS.md`, `docs/FEATURE-STATUS.md`, and `docs/SPEC-COVERAGE.md`.
- [x] 4.4 Verify no AVM/dav2d source, snippets, deps, wrappers, scripts, CI jobs, required tools, runtime process execution, or local absolute paths were introduced.

## 5. Review, Archive, And Gates

- [x] 5.1 Run mandatory review subagents: reviewer, security-reviewer, spec-conformance-reviewer, and encoder-impact-reviewer.
- [x] 5.2 Fix or explicitly close every review finding in `agent-log.md`.
- [x] 5.3 Run `openspec validate tile-payload-decode-boundary --strict`, archive the change, and verify `openspec/specs/` received the expected delta.
- [x] 5.4 Run final local gates: `openspec validate --all --no-interactive`, `cargo xtask check-feature-status`, `cargo xtask check-decoder-support`, `cargo xtask check-diagnostic-registry`, `cargo xtask check-conventional-commits`, and `cargo xtask ci`.

## 6. PR And Merge Discipline

- [ ] 6.1 Open a ready PR, not draft, with scope, non-goals, tests, matrix/docs, subagent sign-offs, local reference evidence summary, and AVM/dav2d boundary statement.
- [ ] 6.2 Wait for GitHub checks to pass.
- [ ] 6.3 Request `@codex review` and wait for Codex completion on the latest PR head, not just an `eyes` reaction.
- [ ] 6.4 After every code-changing push, request Codex review again and wait for completion on the new head before merging.
- [ ] 6.5 Merge only after green checks, latest-head Codex completion, archived OpenSpec, no unresolved threads, and exact head-SHA guard.
