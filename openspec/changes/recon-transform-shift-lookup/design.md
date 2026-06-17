## Context

The § 7.15.4 outer 2D inverse transform leaves `rowShift` / `colShift`
caller-resolved. They come from the verbatim § 7.15.4 `Transform_Shift` constant,
indexed by `txSz`. `splot-recon` models transform shape by the original
`(log2W, log2H)` base-2 log dimensions (not a `txSz` enum), consistent with the
existing `inverse_transform_2d` / `inverse_transform_2d_outer` signatures.

## Decisions

- **Key the lookup on `(log2W, log2H)`, not `txSz`.** The spec accesses
  `Transform_Shift[txSz]`, but `splot-recon` has no `txSz` enum and the existing
  transform primitives already take log2 dimensions. The § 9.2
  `Tx_Width_Log2` / `Tx_Height_Log2` tables prove `(log2W, log2H)` uniquely
  identifies every `TX_SIZES_ALL` ordinal, so a `(log2W, log2H)`-keyed lookup is
  exact and consistent with the crate convention. Its result drops straight into
  the existing `InverseTransform2dOuter` `row_shift` / `col_shift` fields. A
  `tx_size_log2_dims_keys_are_distinct` test pins the uniqueness invariant.

- **Hand-write the constant; do not route it through `gen-tables`.**
  `Transform_Shift` is a § 7.15.4 process-body constant and is **not** present in
  the generated `all_tables.h` § 9 attachment (only `Tx_Width_Log2`,
  `Tx_Height_Log2`, `Tx_Size_Sqr`, and `Tx_Size_Sqr_Up` are). It is therefore a
  hand-written, spec-cited constant, transcribed verbatim from
  `07-decoding-process.md#s-7-15-4` (lines 10610-10636). The `(log2W, log2H)` key
  table mirrors the § 9.2 values, which `splot-recon` cannot reach through
  `splot-core` under the one-way dependency rule — the same constraint that makes
  the § 7.15 transforms take caller-resolved log2 dimensions rather than looking
  them up from `txSz`.

- **Store the two spec tables as separate, parallel `txSz`-indexed arrays.** The
  `Transform_Shift` shifts and the `(log2W, log2H)` keys are kept as two arrays
  indexed by the same `txSz` ordinal (mirroring `Transform_Shift[txSz]` and
  `Tx_Width_Log2[txSz]`), so each is auditable against its own spec source and a
  pairing error is caught by the spot-value tests.

- **New `transform_params` module.** This is the first of the § 7.15.4
  parameter derivations; `get_transform_1d_type`, a `resolve_2d_transform_params`
  helper, and the DPCM-direction selection are the planned next bricks, so a
  dedicated module is the long-term home rather than growing
  `inverse_transform_2d_outer.rs`.

- **Strictly additive / no-output-change.** No existing path is rewired, so the
  minimal flat-intra fixture snapshots are byte-identical. Correctness is proven
  by unit tests against the verbatim spec table (a real end-to-end coefficient
  decode is many bricks away).

## Risks / Trade-offs

- **Transcription risk** on the 25-row table and the `(log2W, log2H)` keys —
  mitigated by an independently-transcribed spot-value test and the spec-mirror
  gate.
- **Orientation risk** (row vs col, width vs height): `Transform_Shift` is
  transpose-symmetric (every `TX_WxH` and `TX_HxW` share their shifts), so the
  tuple **order** `(rowShift, colShift) = ([0], [1])` is what matters; the
  spot-value tests pin the order against asymmetric pairs like `(7, 10)`.
