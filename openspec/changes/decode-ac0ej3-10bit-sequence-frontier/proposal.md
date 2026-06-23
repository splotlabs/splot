# Change: decode-ac0ej3-10bit-sequence-frontier

## Feature IDs

- `DECODE-AC0EJ3-10BIT-SEQUENCE-FRONTIER`

## Why

`ac0ej3.ivf` carries a profile-0, 10-bit, 4:2:0 sequence header. The runtime currently rejects that conformant sequence value during sequence validation, before it can report the next structural runtime gate. The decoder still stores and reconstructs minimal-runtime frames as `DecodedFrame<u8>`, so this change must not let 10-bit streams produce output through the existing 8-bit path.

## Scope

- Spec sections: AV2 v1.0.0 § 6.4.1 Table 6.3, § 7.21.2, § 7.23.
- Crates/modules: `crates/splot-decode/src/runtime_minimal.rs` and focused runtime/CLI regressions.
- CLI/docs/tests: local `ac0ej3` diagnostic regression, 10-bit fixture regression, implementation matrix, generated status/coverage docs.

## Non-goals

- No 10-bit reconstruction, reference storage, raw output, Y4M output, hash output, loop filtering, or oracle-visible 10-bit decode.
- No admission of extra leading payload OBUs beyond the existing fail-closed gate.
- No change to the successful 8-bit fixture subset.

## Acceptance criteria

- [ ] `validate_sequence` accepts AV2 Table 6.3 `bit_depth_idc == 0` as a parsed 10-bit sequence value.
- [ ] Any 10-bit stream that otherwise reaches the runtime decode boundary fails before output with `unsupported_reason = "unsupported_bit_depth"`.
- [ ] The local `ac0ej3.ivf` regression advances past the sequence bit-depth gate and reaches the next precise fail-closed gate.
- [ ] Existing 8-bit conformance fixtures remain byte-identical.
- [ ] Feature tracking, OpenSpec, and generated docs are updated.
