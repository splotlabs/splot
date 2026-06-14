## 1. OpenSpec Planning

- [x] 1.1 Create proposal, design, and decoder-support delta for the concurrency contract.
- [x] 1.2 Validate the OpenSpec change with `openspec validate decoder-runtime-concurrency-contract --strict`.
- [x] 1.3 Record orchestration and subagent findings in `agent-log.md`.

## 2. Documentation And Matrix Sync

- [x] 2.1 Update `docs/DECODER-ROADMAP.md` with the decoder/reconstruction runtime concurrency contract.
- [x] 2.2 Update `docs/DECODER-SUPPORT-MATRIX.toml` with a `decode-runtime-context` foundation row and stale scaffold-note fixes.
- [x] 2.3 Update `docs/IMPLEMENTATION-MATRIX.toml` so `INFRA-DECODER-CRATE-SCAFFOLDING` no longer contradicts the current `splot-decode -> splot-parallel` edge.
- [x] 2.4 Regenerate `docs/DECODER-SUPPORT-STATUS.md`, `docs/FEATURE-STATUS.md`, and `docs/SPEC-COVERAGE.md` if their inputs changed.

## 3. Verification And Review

- [x] 3.1 Run `openspec validate decoder-runtime-concurrency-contract --strict`.
- [x] 3.2 Run decoder, feature, and concurrency drift checks.
- [x] 3.3 Run mandatory review passes and record findings/fixes in `agent-log.md`.
