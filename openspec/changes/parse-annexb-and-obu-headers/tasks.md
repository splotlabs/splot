# Tasks

## Implementation

- [x] `read_leb128` (§ 4.11.6).
- [x] `read_obu_header` / `read_obu_header_from_slice` (§ 5.2.2).
- [x] `ObuType` + layer-id newtypes + § 5.2.1 predicates.
- [x] `parse_annex_b_obus` / `parse_annex_b_obus_partial` (Annex B).

## Tests and proof

- [x] Positive/negative/EOF unit tests per module.
- [x] `parsers_never_panic` proptest.
- [x] CLI fixtures under `tests/fixtures/`.
- [x] Proof recorded in the matrix rows.

## Checks

- [x] `cargo xtask ci`
