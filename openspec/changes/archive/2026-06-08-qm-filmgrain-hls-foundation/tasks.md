# Tasks: QM + Film Grain HLS Foundation

## Pre-work

- [x] Confirm branch includes the OPS/BRT merge.
- [x] Read `AGENTS.md`, feature-tracking docs, and the implementation-matrix schema.
- [x] Validate this OpenSpec change (`openspec validate qm-filmgrain-hls-foundation --strict`).
- [x] Update `docs/IMPLEMENTATION-MATRIX.toml` planned rows.

## Descriptor / helper syntax

- [x] Implement `BitReader::read_svlc()` (§ 4.11.4).
- [x] Add `svlc()` unit + property tests.
- [x] Implement the shared `user_defined_qm(level, t, plane)` helper (§ 5.4.11) with
      the AV2 2D diagonal scan derived from the spec / AVM (no AV1 tables).
- [x] Add tests for plane copy, 8x8 symmetry, 4x8 transpose, coefficient repeat, and EOF.
- [x] Update the `AV2-4.11.4-SVLC` and `AV2-5.4.11-USER-QM` matrix rows.

## Quantizer matrix

- [x] Add QM core types (`QuantizerMatrixObu`, `QuantizerMatrixLevel`, …).
- [x] Implement `quantizer_matrix_obu()` (§ 5.13).
- [x] Wire OBU dispatch and the `ParsedObu::QuantizationMatrix` variant.
- [x] Add the inspect JSON summary (shapes only, no matrix dumps).
- [x] Extend validator HLS state for QM levels.
- [x] Add the `qm/duplicate-reset-between-frames` diagnostic (§ 6.12).
- [x] Add the `qm/duplicate-level-between-frames` diagnostic (§ 6.12).
- [x] Add core / validator / CLI tests and the `quantizer-matrix.av2` fixture.
- [x] Update the `AV2-5.13-QUANTIZATION-MATRIX` matrix row.

## Film grain

- [x] Add film-grain core types (`FilmGrainObu`, `FilmGrainModel`, …).
- [x] Implement `film_grain_obu()` (§ 5.14).
- [x] Implement `film_grain_model()` (§ 5.18.10.2).
- [x] Wire OBU dispatch and the `ParsedObu::FilmGrain` variant.
- [x] Add the inspect JSON summary (counts only, no array dumps).
- [x] Extend validator HLS state for film-grain slots.
- [x] Add the `film-grain/update-flags-zero` diagnostic (§ 6.13).
- [x] Add the `film-grain/chroma-idc-out-of-range` diagnostic (§ 6.13).
- [x] Add the `film-grain/duplicate-slot-in-coded-frame-unit` diagnostic (§ 6.13).
- [x] Add core / validator / CLI tests and the `film-grain.av2` fixture.
- [x] Update the `AV2-5.14-FILM-GRAIN` matrix row.

## Final checks

- [x] Run formatter, clippy, targeted tests, workspace tests, and the xtask
      feature-status checks.
- [x] Regenerate `docs/FEATURE-STATUS.md`.
- [x] Document the deferred frame-reference checks in the PR summary.
