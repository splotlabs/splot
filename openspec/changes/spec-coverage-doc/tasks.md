# Tasks: spec-coverage-doc

## Implementation

- [x] Rework `cargo xtask spec-coverage --format markdown` to render the
      per-spec-section coverage document (`coverage_markdown`): chapter
      grouping, numeric-aware `section_sort_key`, status glyphs
      (`coverage_glyph`), mirror hyperlinks via `parse_mirror_index` with a
      tolerant plain-text fallback, a diagnostics-count column from
      `proof.diagnostics`, and a tail section for rows without spec sections.
- [x] Add `--output` to the `spec-coverage` subcommand (mirrors
      `feature-status`).
- [x] Generalize the `docs/FEATURE-STATUS.md` byte gate into
      `check_generated_doc` and gate `docs/SPEC-COVERAGE.md` the same way from
      `check-feature-status` (and therefore `cargo xtask ci`).

## Docs

- [x] Commit the generated `docs/SPEC-COVERAGE.md`.
- [x] Link it from `README.md` (Feature tracking) and add the regenerate step
      to `docs/FEATURE-TRACKING.md`.
- [x] Update the `XTASK-FEATURE-STATUS` matrix row notes/proof and regenerate
      `docs/FEATURE-STATUS.md`.

## Tests and proof

- [x] Unit tests: `section_sort_key_orders_numerically_and_annexes_last`,
      `coverage_glyphs_cover_every_allowed_status`,
      `mirror_index_parses_numeric_and_annex_rows`,
      `coverage_markdown_renders_linked_rows_and_sectionless_tail`,
      `coverage_markdown_falls_back_to_plain_text_without_mirror`.
- [x] `cargo xtask ci` passes with the committed document.
