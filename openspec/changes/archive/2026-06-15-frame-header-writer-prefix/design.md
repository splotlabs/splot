# Design: frame-header-writer-prefix

## Context

`parse_frame_header_prefix` (`crates/splot-core/src/headers/frame/mod.rs:186`) opens
`frame_header_info()` (§ 5.18.2). It reads at most two `uvlc` fields — `cur_mfh_id` (skipped,
inferred `0`, for a bridge frame) and, when `cur_mfh_id == 0`, `seq_header_id_in_frame_header`
— and derives every other `FrameHeaderPrefix` field (`isFirst`, `isKeyFrame`, `IsBridge`,
`IsRegular`, `startCVS`, `referenced_sequence_header_id`) from `obu_type` /
`first_picture_in_tu`. It then stops with `FrameHeaderPrefixStatus::ActivationFieldsOnly`.

## Decisions

- **Invert the full prefix, not just the intra subset.** The prefix is frame-type-agnostic
  (the parser produces it for bridge / `cur_mfh_id == 0` / `cur_mfh_id > 0`), so the faithful
  inverse round-trips all of them. The intra-only restriction (`cur_mfh_id == 0`,
  `frame_is_intra`) is a `frame_header_core` concern and is enforced by the composing slice
  (#4i), not here.
- **Reject-before-write covers the derived fields.** The `is_*` / `startCVS` fields are not
  written, but a model whose derived fields disagree with the `obu_type` derivation is not
  parser-reachable, so `check_frame_header_prefix_encodable` re-derives them and rejects a
  mismatch with `WriteError::NonCanonicalFrameHeader { what }` before any bit. It also rejects
  a bridge frame carrying a non-zero `cur_mfh_id`, and a `seq_header_id_in_frame_header` /
  `referenced_sequence_header_id` presence that disagrees with the `cur_mfh_id == 0` gate.
- **Local derive mirrors.** `derive_key_frame` / `derive_is_regular` are private in
  `crate::headers::frame`, and the writer mission keeps the parser read-only, so the writer
  re-declares them locally (two small `matches!` over `ObuType`). The
  `mfh_zero_round_trips_across_obu_types` test sweeps every frame-bearing `obu_type`, so a
  drift in either local mirror that rejected a parser-legal prefix would fail the round-trip;
  the per-flag reject tests guard the other direction (accepting a non-canonical flag).
- **No premature core stub.** This slice ships only the prefix writer; it does not add an
  `Unimplemented`-returning `write_frame_header` stub (dead code). The composing entry lands
  with #4i.

## Testing

Deterministic round-trip + byte-exact tests across every frame-bearing `obu_type` and both
`FirstPictureInTU` values, the CLK-withheld (`startCVS == None`) case, `cur_mfh_id > 0`, the
bridge inference, and an out-of-range `seq_header_id`. One reject test per
`NonCanonicalFrameHeader` path (asserting `bit_len() == 0`). A round-trip property test over
random `cur_mfh_id` / `seq_header_id` / `obu_type`.
