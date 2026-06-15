# Tasks

## Implementation
- [x] Add `crates/splot-core/src/write/frame_header.rs` with `write_frame_header_prefix` +
      `check_frame_header_prefix_encodable` (validating every derived field + the `cur_mfh_id`
      / `seq_header_id` gates before any bit).
- [x] Add `WriteError::NonCanonicalFrameHeader { what }`.
- [x] Register the module + re-export `write_frame_header_prefix` in `write/mod.rs`.

## Tests and proof
- [x] Round-trip + byte-exact tests across every frame-bearing `obu_type`, both
      `FirstPictureInTU` values, the CLK-withheld case, `cur_mfh_id > 0`, the bridge
      inference, and an out-of-range `seq_header_id`.
- [x] One reject test per `NonCanonicalFrameHeader` path (asserting `bit_len() == 0`).
- [x] A round-trip property test over random `cur_mfh_id` / `seq_header_id` / `obu_type`.

## Matrix and docs
- [x] Advance `write` `todo -> partial` on `AV2-5.18.1-FRAME-HEADER-GENERAL`, with proof, and
      note the writer start on the `AV2-5.18-FRAME-HEADER` umbrella. Regenerate
      `docs/FEATURE-STATUS.md`.

## Checks
- [x] `cargo xtask ci` and `openspec validate frame-header-writer-prefix --strict`
