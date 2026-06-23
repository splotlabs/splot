# Change: decode-ac0ej3-sequence-chroma-frontier

## Feature IDs

- `DECODE-AC0EJ3-SEQUENCE-CHROMA-FRONTIER`

## Why

`ac0ej3.ivf` carries sequence-level chroma prediction capability flags such as
CfL and possibly MHCCP. These flags are valid sequence configuration, not proof
that the first decoded block has already used unsupported chroma prediction. The
runtime currently rejects `enable_cfl_intra` during sequence validation, before it
can report the next structural key-frame frontier.

The decoder must still fail closed before it can skip §5.20.5.6 `is_cfl`,
`UV_CFL_PRED`, or MHCCP-related mode-info syntax. This change moves the chroma
tool gate to the pre-tile runtime boundary, after parse-only key-frame header
frontier checks and before any tile mode-symbol decode or `DecodedFrame<u8>`
allocation.

## Scope

- Spec sections: AV2 v1.0.0 § 5.20.5.6 and § 5.4.5.
- Crates/modules: `crates/splot-decode/src/runtime_minimal.rs` and focused
  runtime/CLI regressions.
- CLI/docs/tests: local `ac0ej3` diagnostic regression, sequence chroma-tool
  fail-closed tests, implementation matrix, decoder support matrix, and generated
  status/coverage docs.

## Non-goals

- No CfL prediction, MHCCP prediction, `is_cfl` entropy decode, `read_cfl_alphas`,
  or chroma mode expansion.
- No 10-bit reconstruction, 10-bit output serialization, reference storage,
  loop filtering, CDEF, restoration, deblocking, or successful `ac0ej3` decode.
- No admission of extra leading payload OBUs into decode.

## Acceptance criteria

- [ ] `validate_sequence` accepts sequence-level CfL/MHCCP capability flags as
      parsed configuration.
- [ ] Any stream that would otherwise reach tile mode-info with CfL/MHCCP enabled
      rejects before tile-symbol decode using `DECODE-AC0EJ3-SEQUENCE-CHROMA-FRONTIER`.
- [ ] The local `ac0ej3.ivf` regression advances past the former sequence CFL gate
      and reaches the key-frame header frontier without producing output.
- [ ] Existing 8-bit conformance fixtures remain byte-identical.
- [ ] Feature tracking, OpenSpec, and generated docs are updated.
