## 1. Planning And Boundaries

- [x] 1.1 Validate proposal, design, spec delta, and agent log for `tile-square-split-context-derivation`.
- [x] 1.2 Record planning, implementation, testing, documentation, and review-agent findings in `agent-log.md`.

## 2. Square Split Context Derivation

- [x] 2.1 Add crate-private `SquareSplitContextInput` and borrowed `MiSizes` grid handling.
- [x] 2.2 Add bounded AV2 § 8.3.2 `do_square_split` context derivation.
- [x] 2.3 Return `TileCdfSelector::DoSquareSplit` and preserve existing square-split row bounds.
- [x] 2.4 Keep syntax reads, partition decisions, `read_partition()`, and `decode_tile()` out of scope.

## 3. Tests

- [x] 3.1 Add math tests for above-only, left-only, both-neighbor, no-neighbor, and `BLOCK_256X256` contexts.
- [x] 3.2 Add bounds tests for invalid `bSize`, invalid `PlaneStart`, coordinate underflow, missing grid rows, missing grid columns, and invalid grid block-size entries.
- [x] 3.3 Add selector-row handoff tests proving derived square-split selectors index expected generated-default rows.

## 4. Documentation And Status

- [x] 4.1 Update `docs/IMPLEMENTATION-MATRIX.toml`, `docs/DECODER-SUPPORT-MATRIX.toml`, and `docs/DECODER-ROADMAP.md`.
- [x] 4.2 Regenerate decoder support/status and any affected generated docs.
- [x] 4.3 Confirm no AVM/dav2d source, snippets, binaries, submodules, dependencies, wrappers, scripts, CI jobs, required `xtask` commands, or mandatory tests were added.

## 5. Validation And Review

- [x] 5.1 Run focused `splot-decode` tile-payload tests and clippy.
- [x] 5.2 Run `openspec validate tile-square-split-context-derivation --strict`.
- [x] 5.3 Run `cargo xtask check-feature-status`, `cargo xtask check-decoder-support`, `cargo xtask check-decoder-conformance-coverage`, and `cargo xtask ci`.
- [x] 5.4 Complete correctness/spec, safety, and performance/data-layout review-agent passes; fix or document every finding in `agent-log.md`.

## 6. Archive And PR

- [x] 6.1 Archive the completed OpenSpec change and verify the delta folded into `openspec/specs/`.
- [ ] 6.2 Commit, push, and open a ready PR.
- [ ] 6.3 Wait for CI green and latest-head Codex review completion before any merge action.
