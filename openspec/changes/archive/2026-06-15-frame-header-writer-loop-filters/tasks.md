# Tasks

## Writers (additive — no model change)
- [x] `write/frame_filters.rs`: `write_deblocking_filter_params`, `write_gdf_params`,
      `write_cdef_params`, each with an up-front `check_*_encodable` (reject-before-write,
      `bit_len() == 0` on every reject).
- [x] Extract the `gdf_per_block` coded/inferred gate into `pub(crate)
      gdf_per_block_is_coded` in `filtering.rs`; refactor `parse_gdf_params` to call it
      (behavior-preserving). Register the module + re-export the three writers in
      `write/mod.rs`. No model field / `WriteError` variant added.
- [x] Document the reversible derivations / canonical forms: `DfDeltaQ` raw recovery, the
      `gdf_per_block` gate, `cdef_*_pri_zero`, and the `cdef_*_sec_strength` `3 <-> 4` remap.

## Tests and proof
- [x] Round-trip tests across every branch (deblocking: coded_lossless, MFH-update vs direct
      arm, num_planes 1/3, df_delta_q present/absent/inferred-`i==1`, various dfParBits; gdf:
      disabled / single-picture-inferred / read-enable / gdf_per_block coded vs inferred;
      cdef: disabled / single-picture / each `CdefOnSkipTxfm` arm / num_planes 1/3 / y_pri
      zero+nonzero / the sec-strength remap edge / multiple strength sets); one reject test per
      `NonCanonicalFrameHeader` path + the `BitWidthTooLarge` path (`bit_len() == 0`), incl.
      constructed-model panic edges (hostile `df_par_bits_minus_2`, `CdefDamping`/`CdefStrengths`
      out of range, `strengths.len()` mismatch, `y_sec == 3`, `y_pri >= 16`, gdf idx `>= 4`,
      Option presence on the wrong branch). A round-trip property test per parser + a
      never-panics-on-constructed-models proptest per structure.

## Matrix and docs
- [x] Advance the `write` stage on `AV2-5.18.5-FILTERING` (deblocking) and
      `AV2-5.18.7-SEGMENTATION-TILING` (gdf/cdef); both stay `partial` (interp-filter §5.18.5.1
      is inter-path; lr/ccso land in #4g). Regenerate `docs/FEATURE-STATUS.md`.

## Checks
- [x] `cargo xtask ci` and `openspec validate frame-header-writer-loop-filters --strict`
