# Tasks

## Writer (additive — no model change)
- [x] `write/error.rs`: add `WriteError::NonCanonicalFilmGrain { what: &'static str }`.
- [x] `write/film_grain.rs`: `write_film_grain(writer, fg)` inverting `parse_film_grain` (§ 5.14 /
      § 5.18.10.2), canonicalizing the per-array bit widths (minimal in-range width fitting every
      value; recompute increments from cumulative values; re-bias AR coeffs). Reject-before-write for
      the decidable inconsistencies (non-monotonic points, count-vs-len, gated-Option, forced-false
      flags, derived sub_x/sub_y/monochrome, models-vs-update_flags, no-fitting-width); reproduce
      tolerated values verbatim. Re-export in `write/mod.rs`.
- [x] `write/dispatch.rs`: route `ParsedObu::FilmGrain` to the new writer + the generic tail; reject a
      non-empty passthrough; drop it from the `Unimplemented` arm; update the doc counts (ten written
      / four remaining).

## Tests and proof
- [x] `film_grain.rs` writer tests: round-trips (parse a hand-built payload → write → reparse →
      assert_eq) for empty update_flags, monochrome, full chroma with Y/Cb/Cr points + AR coeffs at
      several ar_coeff_lag, chroma_scaling_from_luma, clip+mc_identity, and values spanning the
      bit-width range (exercising the canonicalization); reject tests for each decidable invariant. A
      dispatch round-trip test. A `roundtrip_obu_bytes` fuzz smoke confirming no over-rejection.

## Matrix and docs
- [x] `AV2-5.14-FILM-GRAIN` write `todo` → `done` (+ note); `ENC-BITSTREAM-WRITER` note: four unwritten
      types remain. Regenerate `docs/FEATURE-STATUS.md` (explicit `--output`).

## Checks
- [x] `cargo xtask ci` and `openspec validate film-grain-writer --strict`
