# Design: ccso-offset-index-model

## Context

`parse_ccso_params` (§ 5.18.7.12) reads, per CCSO-enabled plane, a triple-nested loop of
`ccso_offset_idx` `tu(7)` values over `0 <= d0, d1 < maxEdgeInterval` and `0 <= band < maxBand`,
where `maxEdgeInterval` is `1` for `ccso_bo_only` else `CCSO_INPUT_INTERVAL - ccso_edge_clf`
(`3` or `2`), and `maxBand = 1 << ccso_max_band_log2` (up to `128` for the `bo_only` `f(3)`
arm). These values were read and discarded; `CcsoPlaneParams` modeled everything around them but
not the values themselves.

## Decisions

- **Model extension, not a writer-side reject.** The maintainer's standing full-byte-exact
  decision (#4b / #4c) applies: read-and-discarded bits that a writer needs are surfaced in the
  model rather than rejected. The alternative — rejecting `ccso_planes == 1` in the writer —
  would leave the common CCSO case unwritable, a far larger functional hole than the LR
  Wiener-bank residual (which is genuinely unmodeled, not merely discarded).
- **Flat `Vec<u8>` in read order.** The values are stored as a single `(d0, d1, band)`-ordered
  `Vec<u8>` per plane rather than a nested structure. The writer re-derives `maxEdgeInterval` and
  `maxBand` from the same `ccso_bo_only` / `ccso_edge_clf` / `ccso_max_band_log2` it already
  reconstructs, so it iterates the flat list in the identical order — no shape needs to be
  stored. The length (`maxEdgeInterval^2 * maxBand`, `<= 9 * 128`) is bounded and re-derivable,
  so the writer can validate it.
- **`Copy` drop is contained.** `CcsoPlaneParams` was the only `Copy` type in the cluster
  (`CcsoParams` already owns a `Vec`). Every external consumer (the validator, inspect) borrows
  it, and the parser builds-and-moves it, so dropping `Copy` touches nothing but the derive.
- **Parse behavior is otherwise unchanged.** The loop still reads exactly the same bits in the
  same order; only the discard becomes a `push`. The consumed-bit count, the EOF behavior, and
  every existing diagnostic are identical.

## Testing

The existing `ccso_plane_bo_only_reads_offsets`, `ccso_plane_full_arm_reads_ext_filter_and_edge_clf`,
and `ccso_quant_step_zero_suppresses_edge_clf` tests gain assertions on the surfaced values, and a
dedicated `ccso_offset_idx_values_surface_in_iteration_order` test pins the `(d0, d1, band)`
ordering with distinct `tu(7)` values (`[0, 1, 2, 7]`, including the `tu(7)` all-ones terminal).
The existing CCSO never-panics proptest continues to exercise the collection on arbitrary input.
