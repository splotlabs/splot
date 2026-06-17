# Tasks

## Writer (additive — no model change)
- [x] `write/error.rs`: add `WriteError::NonCanonicalQuantizationMatrix { what: &'static str }`.
- [x] `write/quantizer_matrix.rs`: `write_quantizer_matrix(writer, qm)` inverting
      `parse_quantizer_matrix` + `user_defined_qm` (§ 5.13 / § 5.4.11), canonicalizing to the long
      form (skip flags = 0; one `svlc` delta per cell in 2D diagonal scan order, recomputing the
      minimal in-range `quant_delta`). Re-declare `diagonal_scan_2d` (parser-private) with a
      golden test. Reject-before-write (num_planes vs chroma flag, set-bit-derived levels, is_default
      vs matrices, transform count/order, plane count/dims/value-count, zero coefficient). Re-export
      in `write/mod.rs`.
- [x] `write/dispatch.rs`: route `ParsedObu::QuantizationMatrix` to the new writer + the generic
      (non-extensible) tail; reject a non-empty passthrough; remove the `Unimplemented` arm (every
      variant now has a writer); update the doc counts (all fourteen written).

## Tests and proof
- [x] `quantizer_matrix_tests.rs` (sibling `include!` file): round-trips (parse a hand-built payload →
      write → reparse → assert_eq) for the reset OBU, a default level, a user-defined level with a
      NON-FLAT matrix (distinct per-cell values so the scan order is load-bearing), the symmetric /
      transpose / copy / coefficient-repeat decode paths (each re-encoded to long form and still
      semantically equal), and multi-level / 3-plane cases; reject tests for each decidable invariant.
      A dispatch round-trip test + the `unimplemented_*` test removed (no unwritten types remain). A
      `roundtrip_obu_bytes` fuzz smoke confirming no over-rejection.

## Matrix and docs
- [x] `AV2-5.13-QUANTIZATION-MATRIX` write `todo` → `done` (+ note); `ENC-BITSTREAM-WRITER` note: all
      OBU-type body writers landed. Regenerate `docs/FEATURE-STATUS.md` (explicit `--output`).

## Checks
- [x] `cargo xtask ci` and `openspec validate quantization-matrix-writer --strict`
