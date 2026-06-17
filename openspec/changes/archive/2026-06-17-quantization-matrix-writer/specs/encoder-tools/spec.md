# encoder-tools delta: quantization-matrix-writer

## ADDED Requirements

### Requirement: quantizer matrix OBU writer

`splot-core` SHALL provide a writer that serializes a parsed `quantizer_matrix_obu()` (§ 5.13 /
§ 5.4.11) back to bytes — the inverse of `parse_quantizer_matrix` — so the complete-OBU dispatch
round-trips this OBU type instead of returning `Unimplemented`, completing the writer surface for all
OBU payload types. Because the parsed model stores only the decoded coefficients (each `1..=255`), not
the wire deltas or the optional `qm_8x8_is_symmetric` / `qm_4x8_is_transpose_of_8x4` /
`qm_copy_from_previous_plane` / coefficient-repeat compressions, the writer SHALL canonicalize to the
long form — every skip flag `0`, one `svlc()` `quant_delta` per cell in 2D diagonal scan order — so the
re-emission decodes to the same coefficients (a semantic round-trip; byte-exactness is not guaranteed).
It SHALL be reject-before-write and SHALL never panic on a constructed model, rejecting the decidable
inconsistencies (a `num_planes` vs `qm_chroma_info_present_flag` disagreement, a `levels` list that
disagrees with the `qm_bit_map` set bits, an `is_default` vs `matrices` disagreement, a transform or
plane count / dimension / value-count mismatch, and a coefficient of `0`).

#### Scenario: a parsed quantizer matrix OBU round-trips

- **WHEN** a parsed `quantizer_matrix_obu()` — the reset OBU, a default level, or a user-defined level
  exercising the symmetric / transpose / copy / coefficient-repeat decode paths — is written by the
  dispatch and the bytes are reparsed
- **THEN** the reparsed `QuantizerMatrixObu` SHALL equal the original (a semantic round-trip on the
  decoded coefficients; byte-exactness is not guaranteed for the canonicalized long form).

#### Scenario: a non-canonical constructed model is rejected, not panicked

- **WHEN** the writer is given a `QuantizerMatrixObu` the parser could never produce (a num-planes,
  set-bit-derived level, is-default, transform, plane-shape, value-count, or zero-coefficient
  inconsistency)
- **THEN** it SHALL return a typed `WriteError` and write no bit, never panicking.
