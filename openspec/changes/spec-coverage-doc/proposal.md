# Change: spec-coverage-doc

## Feature IDs

- `XTASK-FEATURE-STATUS`

## Why

The canonical status surface (`docs/IMPLEMENTATION-MATRIX.toml` rendered as
`docs/FEATURE-STATUS.md`) is ordered by matrix insertion, keyed by Feature ID,
and uses word statuses — answering "is § 5.4.4 parsed?" requires knowing the ID
convention and decoding a 14-column ledger. Hand-written per-section status
prose (e.g. the `docs/SPEC-MAPPING.md` module table) drifts because nothing
enforces it. The 2026-06-10 documentation audit found that table materially
wrong while the matrix was correct.

This change adds one generated, spec-section-ordered coverage document so any
spec item can be looked up by section number, with glyph status columns and a
diagnostics count, gated against drift exactly like `docs/FEATURE-STATUS.md`.

## Scope

- Spec sections: none (tooling / documentation generation; no AV2 syntax change).
- Crates/modules: `xtask/src/feature_status.rs`, `xtask/src/main.rs`,
  generated `docs/SPEC-COVERAGE.md`, links from `README.md` and
  `docs/FEATURE-TRACKING.md`.

## Non-goals

- No matrix schema change (a `syntax_element` field is a possible follow-up).
- No change to `docs/FEATURE-STATUS.md` rendering.
- No removal of hand-written status prose (that is a separate docs cleanup).

## Acceptance criteria

- [x] `cargo xtask spec-coverage --format markdown --output docs/SPEC-COVERAGE.md`
      writes a per-(section, feature) table grouped by spec chapter, ordered
      numerically (5.2 before 5.10), with annexes last.
- [x] Section cells hyperlink into the committed spec mirror when the section
      resolves via `docs/spec/av2/1.0.0/index.md`, and fall back to plain text
      when it does not.
- [x] Features with empty `spec_sections` are listed in a tail section so all
      matrix rows are accounted for.
- [x] `cargo xtask check-feature-status` fails when the committed
      `docs/SPEC-COVERAGE.md` no longer matches its render.
- [x] Unit tests cover section ordering, glyph mapping, index parsing,
      fallback rendering, and render determinism.
