# encoder-tools delta: film-grain-writer

## ADDED Requirements

### Requirement: film grain OBU writer

`splot-core` SHALL provide a writer that serializes a parsed `film_grain_obu()` (§ 5.14, model syntax
§ 5.18.10.2) back to bytes — the inverse of `parse_film_grain` — so the complete-OBU dispatch
round-trips this OBU type instead of returning `Unimplemented`. Because the model does not store the
wire bit widths or the per-point increments, the writer SHALL canonicalize: it SHALL choose the
smallest in-range bit width that encodes every scaling-point increment, scaling value, and AR
coefficient, recompute the increments from the cumulative point values, and re-bias the AR
coefficients. Semantic round-trip (model equality) SHALL hold; byte-exactness is not guaranteed. The
writer SHALL be reject-before-write and SHALL never panic on a constructed model, rejecting the
decidable inconsistencies (non-monotonic point values, a value that fits no in-range width, count or
gated-`Option` mismatches, the forced-false flag relationships, and the derived
`sub_x`/`sub_y`/`monochrome`/`models` agreements).

#### Scenario: a parsed film grain OBU round-trips

- **WHEN** a parsed `film_grain_obu()` (any combination of slots, scaling points, and AR coefficients)
  is written by the dispatch and the bytes are reparsed
- **THEN** the reparsed `FilmGrainObu` SHALL equal the original.

#### Scenario: a non-canonical constructed model is rejected, not panicked

- **WHEN** the writer is given a `FilmGrainObu` the parser could never produce (a non-monotonic point,
  a count / gated-`Option` / derived-field inconsistency, or a value that fits no in-range width)
- **THEN** it SHALL return a typed `WriteError` and write no bit, never panicking.
