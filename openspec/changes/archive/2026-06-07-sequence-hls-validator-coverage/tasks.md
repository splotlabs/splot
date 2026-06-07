# Tasks: sequence and HLS validator coverage

## 0. Pre-flight

- [x] Run `git status --short` and preserve user changes.
- [x] Run `cargo xtask feature-status --format table`.
- [x] Run `cargo xtask spec-coverage`.
- [x] Confirm all touched Feature IDs exist in `docs/IMPLEMENTATION-MATRIX.toml`.
- [x] Validate this OpenSpec change if the OpenSpec CLI is available.

## 1. Sequence child parser structs

- [x] Add typed structs for each implemented §5.4 child config.
- [x] Keep inferred values explicit when later validation needs them.
- [x] Add doc comments with AV2 section and Feature ID.
- [x] Avoid AV1 names and assumptions.

## 2. Shallow child parser implementation

- [x] Implement `AV2-5.4.3-SEQUENCE-PARTITION-CONFIG`.
- [x] Implement `AV2-5.4.5-SEQUENCE-INTRA-CONFIG`.
- [x] Implement `AV2-5.4.7-SEQUENCE-SCC-CONFIG`.
- [x] Implement `AV2-5.4.12-TIMING-INFO`.
- [x] Implement `AV2-5.4.13-SEQUENCE-DECODER-MODEL-INFO`.
- [x] Add positive, branch, and EOF tests.

## 3. Complex sequence child parser implementation

- [x] Implement or bound `AV2-5.4.6-SEQUENCE-INTER-CONFIG`. (implemented)
- [x] Implement or bound `AV2-5.4.8-SEQUENCE-TQ-ENTROPY-CONFIG`. (implemented)
- [x] Implement or bound `AV2-5.4.10-SEQUENCE-FILTER-CONFIG`. (implemented)
- [x] Implement or bound `AV2-5.4.2-SEQUENCE-TILE-CONFIG` and `tile_params`. (bounded at `tile_params()`)
- [x] Implement or bound `AV2-5.4.4-SEQUENCE-SEGMENT-CONFIG` / `AV2-5.4.9-SEGMENT-INFO`. (config implemented; `seg_info()` bounded)
- [x] Implement or bound `AV2-5.4.11-USER-QM`. (bounded; not reached from the sequence header)

## 4. Sequence semantics

- [x] Add diagnostics for timing zero values.
- [x] Add timing consistency checks across embedded layers where parseable. (not parseable this phase — `timing_info()` is not reached from `sequence_header_obu()`; moved to the `sequence-timing-hls-availability` change)
- [x] Add repeated activated sequence-header bit-identical check.
- [x] Strengthen `max_tlayer_id` / `max_mlayer_id` state tests.
- [x] Keep unparseable/future activation dependencies explicit.

## 5. HLS payload foundation

- [x] Implement temporal delimiter empty payload and state reset.
- [x] Implement MSDO parser and local §6.6 checks.
- [x] Implement multi-frame-header parser skeleton and local range checks.
- [x] Add HLS availability store for parsed sequence/MSDO/MFH objects. (per-`(xlayer, seq_header_id)` sequence-header fingerprints are stored; the full MSDO/MFH/external-HLS availability store is moved to the `sequence-timing-hls-availability` change)

## 6. OBU ordering and inspect

- [x] Add duplicate temporal delimiter diagnostic.
- [x] Add more HLS-prefix ordering tests.
- [x] Update `inspect --json` to show parsed sequence/HLS fields and unimplemented child feature IDs.
- [x] Add CLI integration tests or snapshots.

## 7. Matrix and docs proof

- [x] Update `docs/IMPLEMENTATION-MATRIX.toml` statuses and proof.
- [x] Regenerate `docs/FEATURE-STATUS.md`.
- [x] Update `STATUS.md` with implemented/stubbed items and command results.
- [x] Run `cargo xtask check-feature-status`.
- [x] Run `cargo xtask ci`.
