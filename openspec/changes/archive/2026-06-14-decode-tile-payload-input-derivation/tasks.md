## 1. Planning And Boundaries

- [x] 1.1 Validate proposal, design, delta spec, and agent log for `decode-tile-payload-input-derivation`.
- [x] 1.2 Record planning, spec, API, security/performance, reference, PR #101, and PR #113/#114 carry-forward findings in `agent-log.md`.
- [x] 1.3 Create the implementation branch only after the OpenSpec change validates.

## 2. Parser Fact Exposure

- [x] 2.1 Add `disable_cdf_update: Option<bool>` to `FrameHeaderCore`.
- [x] 2.2 Set the field on intra and other modeled paths that read or derive it, while preserving `None` before the fact is reached.
- [x] 2.3 Add or update `splot-core` tests proving the parser exposes `disable_cdf_update` without changing existing frame-header behavior.

## 3. Tile Boundary Derivation

- [x] 3.1 Split tile-payload boundary lifetimes so returned plans borrow only payload bytes, not locally built framing storage.
- [x] 3.2 Add a crate-private derivation adapter in `splot-decode` that accepts planned provenance, borrowed OBU envelope bytes, parsed frame/tile facts, and limits.
- [x] 3.3 Validate candidate/envelope metadata, source containment, § 5.19 completeness, § 5.20 payload bounds, tile-count policy, and required parser facts before slicing.
- [x] 3.4 Call the existing `DecodeContext` tile-payload boundary handoff and preserve deterministic source/layer/tile metadata.

## 4. Tests

- [x] 4.1 Add positive raw Annex B and IVF derivation tests, including source offset/provenance preservation.
- [x] 4.2 Add negative tests for candidate/fact mismatch, absent parser facts, truncated § 5.19 structure, malformed § 5.20 framing, low tile-count/tile-payload limits, and unsupported paths.
- [x] 4.3 Add deterministic thread-policy tests for `auto`, `1`, and a fixed positive worker count.
- [x] 4.4 Add or update fuzz coverage for the byte-to-tile-boundary derivation path, or document a blocker before commit.

## 5. Documentation And Status

- [x] 5.1 Add/update `DECODE-TILE-PAYLOAD-INPUT-DERIVATION` in `docs/IMPLEMENTATION-MATRIX.toml`.
- [x] 5.2 Update `docs/DECODER-SUPPORT-MATRIX.toml`, `docs/DECODER-ROADMAP.md`, and generated decoder/feature/spec status docs.
- [x] 5.3 Confirm no AVM/dav2d source, snippets, binaries, submodules, dependencies, wrappers, scripts, CI jobs, required `xtask` commands, or mandatory tests were added.

## 6. Validation And Review

- [x] 6.1 Run focused `splot-core` and `splot-decode` tests for frame header and tile derivation.
- [x] 6.2 Run `cargo clippy` for touched crates with warnings denied.
- [x] 6.3 Run `cargo xtask check-feature-status`, `cargo xtask check-decoder-support`, `cargo xtask check-dependency-direction`, `cargo xtask check-concurrency-policy`, and `cargo xtask ci`.
- [x] 6.4 Complete required review-agent passes, fix or document every finding, and update `agent-log.md`.

## 7. Archive And PR

- [x] 7.1 Archive the completed OpenSpec change and verify the delta folded into `openspec/specs/`.
- [x] 7.2 Commit, push, and open a ready PR. Do not make it draft.
- [ ] 7.3 Wait for CI green and latest-head Codex review completion before any merge action. Treat `eyes` as in-progress, not green.
