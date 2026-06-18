## 1. Dependency Boundary

- [x] 1.1 Add `splot-recon` as a direct dependency of `splot-encode`.
- [x] 1.2 Add a private `splot-encode` boundary marker that references
  `splot-recon` without changing public API or runtime encode behavior.
- [x] 1.3 Verify `send_frame`, `receive_packet`, and `flush` still return
  `Error::Unimplemented`.

## 2. Policy, Docs, and Tracking

- [x] 2.1 Update dependency-direction enforcement and unit tests to accept exactly
  `splot-encode -> splot-recon` and reject broader graph changes.
- [x] 2.2 Update `AGENTS.md`, `docs/ARCHITECTURE.md`, and encoder roadmap/gap docs
  for the approved dependency edge.
- [x] 2.3 Add `ENC-RECON-DEPENDENCY` to `docs/IMPLEMENTATION-MATRIX.toml` with proof
  commands and regenerate status/coverage outputs.
- [x] 2.4 Keep OpenSpec deltas aligned with the implemented dependency contract.

## 3. Verification

- [x] 3.1 Run `cargo xtask check-dependency-direction`.
- [x] 3.2 Run `cargo xtask check-zero-copy-policy` and
  `cargo xtask check-concurrency-policy`.
- [x] 3.3 Run `cargo xtask feature-status`,
  `cargo xtask check-feature-status`, and
  `openspec validate --all --no-interactive`.
- [x] 3.4 Run `cargo xtask ci`.

## 4. Review and Merge Gate

- [x] 4.1 Run local correctness/spec, security/zero-copy,
  determinism/concurrency, and test/evidence reviews on the final local tree.
- [x] 4.2 Archive the OpenSpec change before merge.
- [ ] 4.3 Open/update the PR with a Flight Manifest and local evidence.
- [ ] 4.4 Obtain green GitHub checks plus GitHub Claude and GitHub Codex acceptance
  on the final HEAD before squash merge.
