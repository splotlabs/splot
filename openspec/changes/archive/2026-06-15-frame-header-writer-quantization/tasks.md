# Tasks

## Writers (additive — no model change)
- [x] `write/frame_quant.rs`: `write_read_delta_q`, `write_quantization_params`,
      `write_setup_qm_params`, `write_delta_q_params`, `write_lossless_info`, each with an
      up-front `check_*_encodable` (reject-before-write).
- [x] Document the four redundant-encoding canonical forms (read_delta_q zero,
      qm_uv_same_as_y, equal_ac_dc_q chroma-DC, qm_index reverse-lookup).
- [x] Expose `get_qindex_ignore_delta_q` + `ceil_log2` as `pub(crate)`; register the module +
      re-export the writers in `write/mod.rs`. No model field / `WriteError` variant added.

## Tests and proof
- [x] Round-trip tests across every branch + the canonicalization edges; one reject test per
      `WriteError` path (`bit_len()==0`), incl. constructed-model panic edges (qm_index
      no-match, out-of-range su/f(4), `pic_qm_num_minus_1` over-wide, hostile max_segments); a
      round-trip property test per parser.

## Matrix and docs
- [x] Advance `write` `todo -> done` on `AV2-5.18.6-QUANTIZATION` (modeled intra surface),
      with proof + the canonicalization note. Regenerate `docs/FEATURE-STATUS.md`.

## Checks
- [x] `cargo xtask ci` and `openspec validate frame-header-writer-quantization --strict`
