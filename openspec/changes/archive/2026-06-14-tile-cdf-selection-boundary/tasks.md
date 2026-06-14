## 1. Planning And Boundaries

- [x] 1.1 Validate the OpenSpec proposal, design, spec delta, and agent log for `tile-cdf-selection-boundary`.
- [x] 1.2 Record planning, reference, API, and spec-reader agent findings in `agent-log.md`.

## 2. Tile CDF Boundary Implementation

- [x] 2.1 Add crate-private `splot-decode` tile CDF boundary types, selector errors, CDF update policy facts, and copy/average policy calculation.
- [x] 2.2 Copy the supported CDF subset from generated `splot-core` default tables without hand-transcribing table contents.
- [x] 2.3 Add typed selector validation and closure-scoped mutable row access for `DoSplitCdf` and `DoSquareSplitCdf`.
- [x] 2.4 Attach the CDF boundary metadata to the existing tile payload work unit while preserving the structured unsupported `decode_tile()` stop.

## 3. Tests

- [x] 3.1 Add unit tests for default CDF copying, selector bounds, and no-panic typed selector errors.
- [x] 3.2 Add tests proving `SymbolDecoder::read_symbol` mutates selected rows only when CDF updates are enabled.
- [x] 3.3 Add tests for §8.2 copy/average policy calculation across single-tile, context-update-tile, and avg-CDF cases.
- [x] 3.4 Add deterministic `DecodeContext` worker-pool coverage for the tile CDF boundary across `auto`, `1`, and a fixed positive thread count.

## 4. Documentation And Status

- [x] 4.1 Update `docs/IMPLEMENTATION-MATRIX.toml`, `docs/DECODER-SUPPORT-MATRIX.toml`, and `docs/DECODER-ROADMAP.md` for `DECODE-TILE-CDF-SELECTION-BOUNDARY`.
- [x] 4.2 Regenerate decoder support/status docs and feature/spec status docs required by repo checks.
- [x] 4.3 Confirm no AVM/dav2d source, snippets, binaries, submodules, dependencies, wrappers, scripts, CI jobs, required `xtask` commands, or mandatory tests were added.

## 5. Validation And Review

- [x] 5.1 Run focused tests and lints for `splot-decode`.
- [x] 5.2 Run `openspec validate tile-cdf-selection-boundary --strict`.
- [x] 5.3 Run `cargo xtask feature-status`, `cargo xtask check-feature-status`, and `cargo xtask ci`.
- [x] 5.4 Complete required review-agent passes, fix or document every finding, and update `agent-log.md`.

## 6. Archive And PR

- [x] 6.1 Archive the completed OpenSpec change and verify the delta folded into `openspec/specs/`.
- [ ] 6.2 Commit, push, and open a ready PR with PR #113 review carry-forward accurately stated as fixed by PR #114.
- [ ] 6.3 Wait for CI green and latest-head Codex review completion before any merge action.
