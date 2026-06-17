# Change: quantization-matrix-writer

## Feature IDs

- `AV2-5.13-QUANTIZATION-MATRIX` (write: `todo` → `done`)
- `ENC-BITSTREAM-WRITER` (the **last** of the unwritten OBU-type body writers; after it the
  complete-OBU dispatch writes all 14 OBU payload types)

## Why

Complete the writer surface: `quantizer_matrix_obu()` (§ 5.13 / § 5.4.11) is the final OBU type
without a body writer. It is the hardest because the parsed model is **lossy versus the wire**: it
stores the fully *decoded* `UserDefinedQmPlane::values` (the final coefficients, each `1..=255`), not
the wire deltas, and the wire format has four optional compressions — `qm_8x8_is_symmetric`,
`qm_4x8_is_transpose_of_8x4`, `qm_copy_from_previous_plane`, and the `quant2 == 0` coefficient-repeat —
that the parser collapses into `values`.

## What changes

- **Writer** (`crates/splot-core/src/write/quantizer_matrix.rs`, new; additive, no model change):
  `write_quantizer_matrix(writer, qm)` — the inverse of `parse_quantizer_matrix` and
  `user_defined_qm`, **canonicalizing** like the § 5.14 film-grain writer. Because every optional
  compression is just that — optional — and every decoded coefficient is in `1..=255`, the writer
  emits the **long form**: each per-plane skip flag (`qm_copy_from_previous_plane`,
  `qm_8x8_is_symmetric`, `qm_4x8_is_transpose_of_8x4`) is written as `0`, and every cell is written as
  one `svlc()` `quant_delta` in the AV2 2D diagonal scan order (§ 5.20.7.30), recomputing the minimal
  in-range delta (`-128..=127`, § 6.4.11) that drives the running `quant` to the target coefficient.
  This decodes back to the exact `values`, so the semantic round-trip holds; byte-exactness is not
  guaranteed (the original may have used a shorter compressed form), exactly like film grain.
  - **Reject-before-write** (scratch-writer; never panics): a `num_planes` that disagrees with
    `qm_chroma_info_present_flag`; a `levels` list whose length / per-element `level` disagrees with the
    `qm_bit_map` set bits; an `is_default` flag that disagrees with the `matrices` `Option`; a
    `matrices` set whose length / order disagrees with `Fundamental_Tx_Size`; a plane count, dimension,
    or value-count mismatch; and a coefficient of `0` (the parser never decodes a `0` — `quant2 == 0`
    is the repeat sentinel, not a stored coefficient — so it is not representable).
- **Dispatch** (`write/dispatch.rs`): route `ParsedObu::QuantizationMatrix` to the new writer + the
  generic (non-extensible) tail instead of `Unimplemented`; it carries no passthrough. The
  `Unimplemented` dispatch arm is removed — every `ParsedObu` variant now has a body writer.
- **Error** (`write/error.rs`): add `WriteError::NonCanonicalQuantizationMatrix { what }`.

## Validator impact

None.

## Non-goals

- No model change; no public `encode` command; no frame-level quantization-matrix syntax (this is the
  OBU-level `quantizer_matrix_obu()` only).

## Impact

- Crate: `crates/splot-core` (additive `write::quantizer_matrix` + one `WriteError` variant + the
  dispatch arm).
- Docs: `docs/IMPLEMENTATION-MATRIX.toml` (the `AV2-5.13-QUANTIZATION-MATRIX` write rows +
  `ENC-BITSTREAM-WRITER` note: all OBU-type body writers landed) + regenerated `docs/FEATURE-STATUS.md`.
