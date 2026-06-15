# Tasks

## Model + parser
- [x] Add `pub ccso_offset_idx: Vec<u8>` to `CcsoPlaneParams`; drop its `Copy` derive.
- [x] `parse_ccso_params`: collect the `ccso_offset_idx` `tu(7)` loop into the new field in
      `(d0, d1, band)` read order instead of discarding; no other parse-behavior change.

## Consumers
- [x] `inspect.rs`: surface the values in `CcsoPlaneParamsView` (`--json`, omitted when empty).

## Tests and proof
- [x] Assert the surfaced values on the existing `ccso_plane_*` offset tests + a dedicated
      `ccso_offset_idx_values_surface_in_iteration_order` test pinning the `(d0, d1, band)` order
      with distinct `tu(7)` values.

## Matrix and docs
- [x] Note the `ccso_offset_idx` surfacing on `AV2-5.18.7-SEGMENTATION-TILING` (parse fidelity;
      row stays `partial`) with the new test as proof.

## Checks
- [x] `cargo xtask ci` and `openspec validate ccso-offset-index-model --strict`
