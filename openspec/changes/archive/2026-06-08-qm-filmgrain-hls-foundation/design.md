# Design: QM + Film Grain HLS Foundation

## Overview

Two standalone HLS OBU parsers plus their validator state:

```text
OBU_QUANTIZATION_MATRIX -> quantizer_matrix_obu() -> QM level availability + §6.12 checks
OBU_FILM_GRAIN          -> film_grain_obu()      -> film-grain slot availability + §6.13 checks
```

The parsers are syntax-complete for the targeted OBUs. The validator implements the
local conformance checks that do not need full frame-header state.

## Modules

- `crates/splot-core/src/bitio.rs` — `BitReader::read_svlc()` (§ 4.11.4), built on the
  existing `read_uvlc()` so the EOF and `leadingZeros >= 32` bounds propagate unchanged.
- `crates/splot-core/src/headers/quantizer_matrix.rs` — `QuantizerMatrixObu` types, the
  shared `user_defined_qm()` helper, the AV2 2D diagonal scan, and
  `parse_quantizer_matrix()`.
- `crates/splot-core/src/headers/film_grain.rs` — `FilmGrainObu` / `FilmGrainModel`
  types and `parse_film_grain()` / `film_grain_model()`.
- Dispatch in `crates/splot-core/src/obu.rs`; inspect views in
  `crates/splot-cli/src/commands/inspect.rs`; validator state and checks in
  `crates/splot-validate/src/context.rs`.

## User-defined QM helper

`user_defined_qm(level, t, plane)` is shared syntax used by `quantizer_matrix_obu()`.
It is **not** a new direct `sequence_header_obu()` child call. The three fundamental
transforms `Fundamental_Tx_Size[3] = { TX_8X8, TX_8X4, TX_4X8 }` are filled in AV2 2D
diagonal scan order (`get_scan(txSz, TX_CLASS_2D)`), handling plane-copy, 8x8 symmetry,
4x8 transpose-from-8x4, `svlc()` deltas, and the `quant2 == 0` coefficient repeat. The
scan / transform-size derivation is taken from the AV2 spec (§ 5.20.7.30, § 9 tables)
and cross-checked against the AVM oracle `obu_qm.c`; no AV1 scan or transform tables are
copied.

## HLS state and the coded-frame window

The validator tracks, per coded-frame window:

- the quantizer-matrix `qm_bit_map` levels seen and whether any QM OBU has appeared
  (a `qm_bit_map == 0` reset is only conformant as the first QM OBU); and
- the film-grain slots updated.

It also records monotonic per-level / per-slot availability (layer identity, data /
chroma format) as foundation for the deferred frame-reference checks; this phase reads
that availability only to cite the conflicting definition in a duplicate diagnostic.

The validator does not model exact coded-frame-unit boundaries. It resets the window at
every frame-bearing OBU and at each global temporal-delimiter boundary. This
over-resets relative to AVM's reset-before-tile-group point, so it can only drop a
duplicate detection (a documented false negative), never raise a false positive on a
conformant stream.

## Diagnostics

- `qm/duplicate-reset-between-frames` (§ 6.12)
- `qm/duplicate-level-between-frames` (§ 6.12)
- `film-grain/update-flags-zero` (§ 6.13)
- `film-grain/chroma-idc-out-of-range` (§ 6.13)
- `film-grain/duplicate-slot-in-coded-frame-unit` (§ 6.13)

Frame-reference diagnostics (`qm/unavailable-level`, `film-grain/unavailable-model`,
…) are reserved for a later frame-header phase.

## Inspector output

Inspect JSON summarizes QM and film-grain OBUs without dumping large arrays by default:
level / plane / default status and per-transform shape labels for QM; update flags,
chroma idc, updated slots, and per-model point/AR-lag counts for film grain.

## Risks

- The AV2 QM scan order is easy to get wrong; small permutation/fill tests and the AVM
  cross-check mitigate this.
- Coded-frame-unit boundary approximation could cause false positives; the window is
  reset conservatively so only false negatives are possible.
- The film-grain model syntax is long; typed arrays and structured EOF tests keep the
  parser panic-free.
