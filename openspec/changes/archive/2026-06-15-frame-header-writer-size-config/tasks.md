# Tasks

## Model + parser surfacing (maintainer-approved exception)
- [x] `config.rs`: add `IntrabcParams` + `parse_intrabc_params_full` (`Some`-iff-bit-present);
      `parse_intrabc_params` -> thin `bool` wrapper; remove the unused
      `parse_screen_content_params` `bool` wrapper.
- [x] `info.rs`: `FrameHeaderCore` gains `force_integer_mv` + `intrabc`; the intra path uses the
      `_full` parsers. `consumed_bits` unchanged; existing fields preserved.
- [x] `mod.rs`: re-export `IntrabcParams` + `pub(crate)` re-exports of the `_full` parsers /
      `parse_frame_size`. Bump the `info.rs` source-line allowance with rationale.

## Writers
- [x] `write/frame_config.rs`: `write_frame_size` (§ 5.18.4.1), `write_screen_content_params`
      (§ 5.18.3.3), `write_intrabc_params` (§ 5.18.3.4), each with an up-front
      `check_*_encodable` validating every field/gate before any bit.
- [x] Register the module + re-export the writers in `write/mod.rs`.

## Tests and proof
- [x] Byte-exact round-trip unit tests across every branch; one reject test per `WriteError`
      path (asserting `bit_len() == 0`); round-trip property tests; a full-surfacing parser test.

## Matrix and docs
- [x] Advance `write` `todo -> partial` on `AV2-5.18.4-FRAME-SIZE` and
      `AV2-5.18.3-FRAME-CONFIGURATION`, with proof + the model-extension note. Regenerate
      `docs/FEATURE-STATUS.md`.

## Checks
- [x] `cargo xtask ci` and `openspec validate frame-header-writer-size-config --strict`
