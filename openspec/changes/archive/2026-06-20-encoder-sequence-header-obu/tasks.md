## 1. Sequence-header model + OBU

- [x] 1.1 Add `build_minimal_intra_sequence_header()` parse-backing the committed conformance-vector 11-byte body into a `SequenceHeader`.
- [x] 1.2 Add `encode_minimal_intra_sequence_header_obu()` emitting the body+§5.2.3-tail payload (via `write_obu_payload`) under the no-extension `OBU_SEQUENCE_HEADER` header, in Annex B framing; add the `MinimalIntraSequenceHeaderError` typed error; re-export from `frame`.

## 2. Tests

- [x] 2.1 The payload round-trips byte-exact to the canonical body (body+tail).
- [x] 2.2 The OBU is byte-exact to the committed conformance vector's sequence-header OBU (`0c 04` + the 11-byte body) and reparses as exactly one `OBU_SEQUENCE_HEADER`.

## 3. Tracking and verification

- [x] 3.1 Add `ENC-SEQUENCE-HEADER-OBU` to the implementation matrix and refresh generated status/coverage docs.
- [x] 3.2 Keep tracking honest: a single OBU, not a temporal unit or a decodable stream; the frame OBU is not yet made consistent with this sequence header; no claim of an IVF stream, a packet, `receive_packet` output, CLI success, or Baseline Encoder Profile v1.
- [x] 3.3 Run OpenSpec validation, focused core tests, feature-status checks, and `cargo xtask ci`.
