## 1. Planning And Matrix Setup

- [x] 1.1 Add or update `DECODE-MINIMAL-INTRA-RECONSTRUCTION-FRONTIER` in `docs/IMPLEMENTATION-MATRIX.toml` with spec sections, notes, proof placeholders, and this OpenSpec change.
- [x] 1.2 Update `docs/DECODER-SUPPORT-MATRIX.toml` and roadmap wording to describe the narrow reconstructed minimal runtime handoff without changing broad partial rows to supported.

## 2. Runtime Reconstruction Handoff

- [x] 2.1 Replace the minimal runtime's direct filled-plane construction with a crate-private handoff that builds `DecodedFrameInfo`, allocates `CurrentFrameWorkspace<u8>`, predicts the traced luma DC block, materializes neutral chroma through checked workspace writes, and freezes the workspace.
- [x] 2.2 Preserve existing minimal tier guards, resource-limit checks, and unsupported-feature diagnostics before output construction.
- [x] 2.3 Keep hash and Y4M adapters unchanged except for consuming the reconstructed frame returned by the minimal runtime.

## 3. Tests

- [x] 3.1 Add focused minimal runtime tests proving the reconstructed frame visible Y/U/V samples are all 128 and come through the workspace handoff.
- [x] 3.2 Keep the existing expected hash digest and exact Y4M byte tests unchanged.
- [x] 3.3 Re-run mutation, resource-limit, malformed-source, and out-of-tier tests that prove no failed stream publishes output.

## 4. Documentation And Generated Status

- [x] 4.1 Update `docs/DECODER-ROADMAP.md` and matrix notes to state this is only the traced minimal DC reconstruction frontier.
- [x] 4.2 Regenerate `docs/FEATURE-STATUS.md`, `docs/SPEC-COVERAGE.md`, and `docs/DECODER-SUPPORT-STATUS.md` as required by the touched matrices.
- [x] 4.3 Run `cargo xtask feature-status`, `cargo xtask check-feature-status`, and `cargo xtask check-decoder-support`.

## 5. Review And Gates

- [x] 5.1 Run subagent planning/review for spec exactness, architecture, untrusted-input safety, and output determinism; record pass/block decisions.
- [x] 5.2 Run targeted gates: `openspec validate minimal-intra-reconstruction-frontier --strict`, `cargo test -p splot-decode runtime_hash --locked`, `cargo test -p splot-decode runtime_y4m --locked`, and relevant `splot-recon` workspace/intra tests.
- [x] 5.3 Run acceptance gates: `openspec validate --all --no-interactive`, `cargo xtask ci`, `cargo xtask check-feature-status`, and `cargo xtask check-decoder-support`.

## 6. Archive And PR

- [x] 6.1 Archive the OpenSpec change with `openspec archive minimal-intra-reconstruction-frontier --yes` and commit the archive in the implementation branch.
- [x] 6.2 Re-run the targeted and acceptance gates after archive.
- [ ] 6.3 Open a ready, non-draft PR with spec sections, matrix rows, diagnostics, tests, local evidence, exclusions, and reviewer decisions.
- [ ] 6.4 Wait for green CI, latest-head Codex clean/approval, and zero live unresolved review threads before squash merge.
