## 1. Temporal-delimiter OBU primitive

- [x] 1.1 Add `encode_temporal_delimiter_obu() -> Result<Vec<u8>, WriteError>`: the no-extension `OBU_TEMPORAL_DELIMITER` header (inferred mlayer 0, global xlayer 31), empty payload, Annex B framing; re-export it from `frame`.

## 2. Tests

- [x] 2.1 A round-trip test: the result is the canonical `[0x01, 0x08]` and reparses as exactly one `OBU_TEMPORAL_DELIMITER` with an empty payload.

## 3. Tracking and verification

- [x] 3.1 Add `ENC-TEMPORAL-DELIMITER-OBU` to the implementation matrix and refresh generated status/coverage docs.
- [x] 3.2 Keep tracking honest: a single OBU primitive, not a temporal unit or a decodable stream; no claim of a sequence header, an IVF stream, a packet, `receive_packet` output, CLI success, or Baseline Encoder Profile v1.
- [x] 3.3 Run OpenSpec validation, focused core tests, feature-status checks, and `cargo xtask ci`.
