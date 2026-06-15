## 1. Planning And Boundaries

- [x] 1.1 Validate the OpenSpec proposal, design, spec delta, and agent log for `tile-partition-cdf-selectors`.
- [x] 1.2 Record planning, implementation, testing, documentation, and review-agent findings in `agent-log.md`.

## 2. Tile Partition CDF Selector Expansion

- [x] 2.1 Extend `TileCdfRows` to own `DoExtPartitionCdf` and `DoUneven4wayPartitionCdf` rows copied from generated `splot-core` defaults.
- [x] 2.2 Add typed selector and error-reporting support for the two new CDF row families.
- [x] 2.3 Include the new rows in saved subset copy/average handling without claiming real frame-end CDF update support.
- [x] 2.4 Preserve crate-private scope, current public APIs, dependency direction, scheduler policy, and runtime unsupported behavior outside the CDF boundary.

## 3. Tests

- [x] 3.1 Expand unit tests for generated-default copying and no aliasing across all supported row families.
- [x] 3.2 Expand selector tests for valid `plane_start`/`ctx` edges and typed bounds errors for the new rows.
- [x] 3.3 Expand `SymbolDecoder::read_symbol(cdf)` handoff tests across supported selectors and CDF update modes.
- [x] 3.4 Expand saved copy/average tests so the supported subset includes the new rows.

## 4. Documentation And Status

- [x] 4.1 Update `docs/IMPLEMENTATION-MATRIX.toml`, `docs/DECODER-SUPPORT-MATRIX.toml`, and `docs/DECODER-ROADMAP.md` for the expanded boundary.
- [x] 4.2 Regenerate decoder support/status and feature status docs required by repo checks.
- [x] 4.3 Confirm no AVM/dav2d source, snippets, binaries, submodules, dependencies, wrappers, scripts, CI jobs, required `xtask` commands, or mandatory tests were added.

## 5. Validation And Review

- [x] 5.1 Run focused `splot-decode` tile-payload tests and clippy.
- [x] 5.2 Run `openspec validate tile-partition-cdf-selectors --strict`.
- [x] 5.3 Run `cargo xtask feature-status`, `cargo xtask check-feature-status`, `cargo xtask check-decoder-support`, and `cargo xtask ci`.
- [x] 5.4 Complete required implementation, testing, documentation, correctness, security, and performance review-agent passes; fix or document every finding in `agent-log.md`.

## 6. Archive And PR

- [x] 6.1 Archive the completed OpenSpec change and verify the delta folded into `openspec/specs/`.
- [ ] 6.2 Commit, push, and open a ready PR.
- [ ] 6.3 Wait for CI green and latest-head Codex review completion before any merge action.
