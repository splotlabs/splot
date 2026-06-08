# Change: QM + Film Grain HLS Foundation

## Summary

Implement AV2 Quantizer Matrix (§ 5.13) and Film Grain (§ 5.14) OBU parsing, the
`svlc()` descriptor (§ 4.11.4) and the shared `user_defined_qm()` helper (§ 5.4.11)
they require, local validation (§ 6.12 / § 6.13), inspector output, and the per-level
/ per-slot HLS availability state that future frame-reference checks will consume.

## Motivation

After the sequence, MFH, LCR/atlas, OPS, and BRT foundations, the remaining standalone
high-level-syntax OBUs that `open_bitstream_unit()` dispatches before metadata/padding
and frame expansion are Quantizer Matrix and Film Grain. Both establish reusable HLS
state that frame syntax references later (`using_qmatrix` / `qm_*`, `apply_grain` /
`fgm_id`). Implementing them now continues validator coverage without forcing a full
frame-header or tile-payload parser.

## Scope

In scope:

- `svlc()` descriptor parsing (`BitReader::read_svlc`).
- The shared `user_defined_qm(level, t, plane)` helper, including the AV2 2D diagonal
  scan and the three fundamental transform shapes.
- `quantizer_matrix_obu()` and `film_grain_obu()` / `film_grain_model()` parsers.
- `ParsedObu` variants, OBU dispatch, and compact inspect JSON summaries.
- Validator/HLS state for QM levels and film-grain model slots, plus the local § 6.12
  and § 6.13 diagnostics.
- Fixtures and tests.

Out of scope:

- the full frame-header parser;
- frame quantization-reference checks for `using_qmatrix` / `qm_*`;
- frame film-grain-reference checks for `apply_grain` / `fgm_id`;
- the `QmProtected` key-frame reset (it depends on key-frame frame-header state);
- tile-group payload parsing, metadata OBUs, Annex A/E conformance;
- encoder/bitstream writer and the AVM differential harness.

## Compatibility

This change is additive to parser and validator coverage. It preserves existing public
APIs and extends the `ParsedObu` enum (`#[non_exhaustive]`) and the inspect record in a
backward-compatible way.
