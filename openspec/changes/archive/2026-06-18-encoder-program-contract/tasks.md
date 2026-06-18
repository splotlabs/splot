# Tasks

> Status: in progress. Feature ID: `DOC-ENCODER-PROGRAM-CONTRACT`.

## Planning artifacts

- [x] Create the `encoder-program-contract` OpenSpec proposal.
- [x] Add design notes and spec deltas for `encoder-program`, `encoder-api`, and
  `encoder-tools`.
- [x] Record the formal Flight Manifest in the design.

## Documentation

- [x] Add `docs/ENCODER-GOAL.md` defining Baseline Encoder Profile v1.
- [x] Add `docs/ENCODER-ROADMAP.md` with phased implementation order and evidence
  gates.
- [x] Add `docs/ENCODER-GAP-AUDIT.md` recording source, writer, recon,
  conformance, dependency, and active-PR baselines.
- [x] Refresh the validator-roadmap encoder carve-out without changing validator
  priorities.
- [x] Mark `toy-intra-encoder-v0` as superseded/parked behind the new contract.

## Matrix and generated status

- [x] Add the `DOC-ENCODER-PROGRAM-CONTRACT` row.
- [x] Refresh generated feature/status docs if the matrix changes.

## Checks

- [x] `openspec validate --all --no-interactive`
- [x] `cargo xtask feature-status`
- [x] `cargo xtask check-feature-status`
- [x] `cargo xtask ci`
