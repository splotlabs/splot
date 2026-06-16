# encoder-tools delta: content-interpretation-writer

## ADDED Requirements

### Requirement: content interpretation OBU writer

`splot-core` SHALL provide a writer that serializes a parsed `content_interpretation_obu()` (§ 5.15)
back to bytes — the inverse of `parse_content_interpretation` (including the shared `timing_info()`)
— so the complete-OBU dispatch round-trips this OBU type instead of returning `Unimplemented`. The
writer SHALL reproduce parser-tolerated values (the `ci_reserved_2bit` and the reserved color /
aspect-ratio / scan idc values that the validator flags but the parser preserves) VERBATIM rather
than rejecting them, so a parsed model always round-trips. It SHALL be reject-before-write and SHALL
never panic on a constructed model, rejecting only the strictly decidable structural inconsistencies
(an `extended_sar` presence that disagrees with `ci_aspect_ratio_idc == 255`, a color-primaries
presence that disagrees with `ci_color_description_idc == 0`, and any field value outside its
descriptor's domain).

#### Scenario: a parsed content interpretation OBU round-trips

- **WHEN** a parsed `content_interpretation_obu()` (any combination of present sub-structs, including
  reserved idc and reserved-bit values) is written by the dispatch and the bytes are reparsed
- **THEN** the reparsed `ContentInterpretation` SHALL equal the original, byte-exact on the canonical
  subset.

#### Scenario: a non-canonical constructed model is rejected, not panicked

- **WHEN** the writer is given a `ContentInterpretation` the parser could never produce (an
  `extended_sar` / color-primaries gate inconsistency, or an out-of-range field)
- **THEN** it SHALL return a typed `WriteError` and write no bit, never panicking.
