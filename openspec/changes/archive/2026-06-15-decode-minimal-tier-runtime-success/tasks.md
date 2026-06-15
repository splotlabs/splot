## 1. Scope And Fixture Verification

- [x] 1.1 Verify the committed minimal intra IVF fixture that will satisfy `minimal-intra-8bit420-hash-v1`, or create a tiny source-backed fixture with manifest metadata.
- [x] 1.2 Record the accepted tier gates and rejection reasons before implementation: base layer, 8-bit 4:2:0, one closed-loop-key output frame, inline frame header, no crop, no film grain, one tile group, and one tile.
- [x] 1.3 Confirm the `splot-decode -> splot-recon` dependency edge before editing Cargo manifests, and document the approved graph change in the PR.
- [x] 1.4 Capture a source-backed tile/block/symbol trace for the selected fixture, or replace it with a narrower fixture whose active decode paths are fully understood.

## 2. Runtime Hash API

- [x] 2.1 Add a narrow `splot-decode` runtime hash entry point on `DecodeContext` that reuses `plan_bytes` before decoding.
- [x] 2.2 Add a `DecodeHashReport` model matching `splot.decode.hash_report` v1 with raw intermediate output variant metadata.
- [x] 2.3 Implement minimal-tier validation that fails closed with structured `decode/unsupported-feature` metadata for out-of-tier streams.
- [x] 2.4 Thread `DecodeLimits` through frame dimensions, decoded-frame bytes, output frame counts, output byte counts, tile count, and tile payload bytes before allocation or hash report construction.

## 3. Minimal Decode And Hash

- [x] 3.1 Derive runtime tile/frame facts from source-backed planner output instead of CLI-owned parsing.
- [x] 3.2 Implement only the traced §8.2 tile-symbol verification and all-flat reconstruction handoff needed by the selected minimal fixture.
- [x] 3.3 Build a `splot-recon::DecodedFrame` and compute `DecodedFrameHashInput::compute_hash()` for `raw_intermediate_output`.
- [x] 3.4 Prove runtime output ordering and hash values are deterministic across `--threads 1`, `--threads auto`, and a fixed positive `--threads N`.

## 4. CLI Behavior

- [x] 4.1 Dispatch `splot decode --output-format hash --json` to the runtime hash API and emit success JSON with exit code 0.
- [x] 4.2 Preserve diagnostic JSON/text rendering for malformed, resource-limit, and unsupported inputs with nonzero exit status.
- [x] 4.3 Preserve hash-mode no-touch semantics for absent and existing `-o` paths; do not add successful hash file output in this change.
- [x] 4.4 Preserve Y4M/raw runtime output as unsupported and no-touch.

## 5. Tests And Fuzz

- [x] 5.1 Add library tests for minimal hash success, tier rejection, malformed input, resource-limit failure, and thread determinism.
- [x] 5.2 Add CLI tests for hash JSON success, no-touch output paths, thread-deterministic hashes, and unchanged diagnostic failures.
- [x] 5.3 Add or update fixture manifest/reference-evidence tests for the committed minimal fixture.
- [x] 5.4 Add or update a fuzz target for any newly byte-consuming runtime decode surface, or record why existing `decode_plan_bytes` remains the only byte-consuming fuzz target.

## 6. Docs And Status

- [x] 6.1 Add `DECODE-MINIMAL-TIER-RUNTIME-SUCCESS` to `docs/IMPLEMENTATION-MATRIX.toml` with proof.
- [x] 6.2 Add or update decoder support rows without overclaiming broad decode, tile, CDF, intra, Y4M, film-grain, layer, or decoder-model support.
- [x] 6.3 Update `docs/DECODER-ROADMAP.md`, `docs/DECODER-FULL-CONFORMANCE.md`, and generated decoder/feature/spec status docs.
- [x] 6.4 Update `docs/LOCAL-REFERENCE-EVIDENCE.toml` only with portable metadata, with no AVM/dav2d integration.

## 7. Review And Gates

- [x] 7.1 Run implementation and test-writing subagents for the scoped runtime/API/test/docs work.
- [x] 7.2 Run independent correctness, security/reference, and performance/documentation reviews with written pass/block decisions.
- [x] 7.3 Run `openspec validate decode-minimal-tier-runtime-success --strict`.
- [x] 7.4 Run `cargo xtask feature-status`.
- [x] 7.5 Run `cargo xtask check-feature-status`.
- [x] 7.6 Run `openspec validate --all --no-interactive`.
- [x] 7.7 Run `cargo xtask check-decoder-support`.
- [x] 7.8 Run `cargo xtask check-decoder-conformance-coverage`.
- [x] 7.9 Run `cargo xtask ci`.

## 8. Archive And PR

- [x] 8.1 Archive the OpenSpec change with `openspec archive decode-minimal-tier-runtime-success --yes`.
- [x] 8.2 Re-run validation gates after archive.
- [x] 8.3 Commit, push, and open a ready PR, not a draft.
- [ ] 8.4 Wait for CI, Claude review, and the latest Codex connector review outcome after `@codex review`; address every finding before merge.
- [ ] 8.5 Squash merge only after checks are green, OpenSpec is archived, Codex has completed with no unaddressed findings, and no unresolved review threads remain.
