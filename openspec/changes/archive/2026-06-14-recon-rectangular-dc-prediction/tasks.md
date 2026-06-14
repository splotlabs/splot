## 1. Planning And Feature Tracking

- [x] 1.1 Validate the OpenSpec change before implementation.
- [x] 1.2 Add or update the `RECON-INTRA-DC-RECTANGULAR-PREDICTION` matrix row and decoder support status entries.
- [x] 1.3 Record planning subagent findings in `agent-log.md`.

## 2. Reconstruction Implementation

- [x] 2.1 Add rectangular intra block geometry and allocation-free rectangular DC prediction APIs in `splot-recon`.
- [x] 2.2 Implement AV2 §7.13.3.22 approximate division for rectangular both-edge DC prediction without adding dependencies.
- [x] 2.3 Preserve existing square DC public APIs as compatible wrappers.
- [x] 2.4 Add current-frame workspace rectangular block writes, edge extraction, and rectangular DC prediction helpers.

## 3. Tests

- [x] 3.1 Add self-contained rectangular DC prediction tests for both-edge, left-only, above-only, no-edge, compatibility, and invalid input cases.
- [x] 3.2 Add workspace rectangular DC prediction tests proving in-storage edge extraction, writes, frozen-frame interop, and typed failures.
- [x] 3.3 Run focused `splot-recon` tests and concurrency/dependency checks.

## 4. Documentation And Review

- [x] 4.1 Update roadmap, decoder support matrix/status, implementation matrix, and OpenSpec artifacts to match the shipped behavior.
- [x] 4.2 Run mandatory review subagents and record sign-offs/findings in `agent-log.md`.
- [x] 4.3 Run `openspec validate recon-rectangular-dc-prediction --strict`, archive the change, and run the required local gates before commit/PR.
