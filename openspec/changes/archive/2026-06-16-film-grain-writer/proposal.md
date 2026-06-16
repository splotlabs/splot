# Change: film-grain-writer

## Feature IDs

- `AV2-5.14-FILM-GRAIN` (write: `todo` → `done`)
- `ENC-BITSTREAM-WRITER` (the fifth of the unwritten OBU-type body writers)

## Why

Continue moving the parser-modeled OBU types from `Unimplemented` to round-trippable in the
complete-OBU dispatch. `film_grain_obu()` (§ 5.14, model syntax § 5.18.10.2) is the next target. Unlike
the prior writers it requires a **canonicalization**: the model is lossy with respect to the wire
format (it stores the cumulative scaling-point `value`/`scaling` and the de-biased AR coefficients, but
not the per-array bit widths `point_value_increment_bits` / `point_scaling_bits` / `ar_coeff` bit
width, nor the per-point increments), so the writer re-derives the minimal in-range bit widths and
re-computes the wire values. Semantic round-trip holds (the widths are not in the model's `PartialEq`);
byte-exactness is not guaranteed and is documented.

## What changes

- **Writer** (`crates/splot-core/src/write/film_grain.rs`, new; additive, no model change):
  `write_film_grain(writer, fg)` — the inverse of `parse_film_grain` + `parse_film_grain_model` +
  `read_scaling_points` + `read_ar_coeffs` (§ 5.14 / § 5.18.10.2), with private helpers that
  **canonicalize** the bit widths: pick the smallest `point_*_value`-increment / `point_*_scaling` /
  `ar_coeff` width in its descriptor's range that fits every value, recompute per-point increments
  from the cumulative values, and re-bias the AR coefficients.
  - **Reject-before-write** (scratch-writer; never panics): non-monotonic scaling-point values; a
    value that fits no in-range width; every count-vs-Vec-length and gated-`Option`-vs-gate mismatch;
    the `monochrome` / `chroma_scaling_from_luma` / `mc_identity` forced-false relationships; the
    derived `sub_x`/`sub_y`/`monochrome`-vs-`chroma_idc` agreement; the `models`-vs-`update_flags`
    slot agreement; and field-width rejects.
  - **Reproduce-verbatim** the parser-tolerated values (e.g. an out-of-range `chroma_idc` the parser
    preserves, `num_*_points` up to 15) so a parsed model always round-trips.
- **Dispatch** (`write/dispatch.rs`): route `ParsedObu::FilmGrain` to the new writer + the generic
  tail (FilmGrain is not extensible) instead of `Unimplemented`; it carries no passthrough. Four
  types remain unwritten.
- **Error** (`write/error.rs`): add `WriteError::NonCanonicalFilmGrain { what }`.

## Validator impact

None.

## Non-goals

- No writers for the other four unwritten OBU types; no model change; no public `encode` command.

## Impact

- Crate: `crates/splot-core` (additive `write::film_grain` + one `WriteError` variant + the dispatch
  arm).
- Docs: `docs/IMPLEMENTATION-MATRIX.toml` (`AV2-5.14-FILM-GRAIN` write `done` + `ENC-BITSTREAM-WRITER`
  note) + regenerated `docs/FEATURE-STATUS.md`.
