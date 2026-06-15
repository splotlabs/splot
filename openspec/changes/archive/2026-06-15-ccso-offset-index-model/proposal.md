# Change: ccso-offset-index-model

## Feature IDs

- `AV2-5.18.7-SEGMENTATION-TILING` (advances the `ccso_params()` parse fidelity — the
  `ccso_offset_idx` values are now modeled instead of discarded; the row stays `partial`)
- `ENC-BITSTREAM-WRITER` (forward-plumbing: the surfaced values are what the upcoming
  § 5.18.7.12 CCSO writer needs to reproduce the structure byte-exactly)

## Why

The intra-path `ccso_params()` parser (§ 5.18.7.12) reads the per-plane `ccso_offset_idx`
`tu(7)` loop (`maxEdgeInterval * maxEdgeInterval * maxBand` values per CCSO-enabled plane) but
**discards** every value — `CcsoPlaneParams` stores none of them. A faithful CCSO *writer*
(the #4g slice of the frame-header writer) therefore cannot reproduce a plane with
`ccso_planes == 1` (the common CCSO case): the offset bits are unrecoverable from the model.

This is the same read-and-discarded-bits situation the maintainer resolved for #4b / #4c with a
**model extension for full byte-exactness**. The maintainer chose the same here: surface the
`ccso_offset_idx` values in the model + parser so the writer round-trips byte-exactly, rather
than rejecting `ccso_planes == 1`.

## What changes

- **Model** (`crates/splot-core/src/headers/frame/restoration.rs`): `CcsoPlaneParams` gains a
  `pub ccso_offset_idx: Vec<u8>` field — a flat `(d0, d1, band)`-ordered list of the `tu(7)`
  values (each `0..=7`), length `maxEdgeInterval^2 * maxBand`, empty when `ccso_planes == 0`.
  `CcsoPlaneParams` drops its `Copy` derive (it now owns a `Vec`); every consumer takes it by
  reference, so the blast radius is the one derive.
- **Parser** (`parse_ccso_params`): the `ccso_offset_idx` loop now collects the values into the
  new field in read order instead of discarding them. No other parse behavior changes; the
  consumed-bit count is identical.
- **inspect** (`crates/splot-cli/src/commands/inspect.rs`): `CcsoPlaneParamsView` surfaces the
  values in `--json` (omitted when the plane codes no offsets).

## Validator impact

None. The validator consumes `&CcsoParams` by reference and reads only the fields it already
checks (`ccso_ext_filter`, `ccso_max_band_log2`); no new diagnostics.

## Non-goals

- No § 5.18.7.12 CCSO *writer* (the next #4g slice consumes this surface).
- No § 5.18.7.11 LR writer (additive, lands alongside the CCSO writer).
- No change to the `read_wienerns_filter()` LR Wiener-bank residual (still an honest stop).

## Impact

- Crate: `crates/splot-core` (one model field + the parser collection), `crates/splot-cli`
  (inspect view). A maintainer-approved model extension (the read-only-parser mission constraint
  exception, like #4b / #4c).
- Docs: `docs/IMPLEMENTATION-MATRIX.toml`.
