## 1. Single-picture CoreSeqView constructor

- [x] 1.1 Add `CoreSeqView::new_minimal_intra_single_picture(max_frame_width, max_frame_height) -> Option<Self>` applying exactly the eight § 5.4.x single-picture inferences over `new_minimal_intra`, preserving `#[non_exhaustive]` and the `1..=2^16` `None` domain.

## 2. Parse-backed FrameHeaderCore assembler

- [x] 2.1 Serialize the canonical 64x64 / `base_q_idx == 255` single-picture CLK § 5.18.2 body (`BitWriter`, one write per element; `order_hint` omitted, explicit `allow_screen_content_tools`, single-superblock tile info).
- [x] 2.2 Add the self-contained `build_minimal_intra_clk_core() -> Result<(FrameHeaderCore, CoreSeqView), MinimalIntraCoreError>` that builds the matched 64x64 single-picture view (referencing sequence header 0) internally and parses the body against it to an `IntraHeaderComplete` core, returning the `(core, seq)` pair so the body and view cannot be mis-paired; with the typed body/parse/`Seq` error.
- [x] 2.3 Un-gate the `pub(crate)` `init_core_from_prefix` / `parse_core_body` re-export and re-export the assembler from `frame`.

## 3. Tests

- [x] 3.1 A field-delta test proves the eight single-picture inferences differ from `new_minimal_intra` and the rest is inherited; out-of-range maxima yield `None`.
- [x] 3.2 A round-trip test proves the assembled core is `IntraHeaderComplete` with the derived facts (Key, 64x64, `order_hint_lsb == 0`, `refresh_frame_flags == 3`, immediate-output), and `write_frame_header_core` re-emits a stream reparsing to an equal core (the conformance oracle).

## 4. Tracking and verification

- [x] 4.1 Add `ENC-FRAME-HEADER-CORE-ASSEMBLER` to the implementation matrix and refresh generated status/coverage docs.
- [x] 4.2 Keep tracking honest: no claim of a tile-group OBU, a frame, a packet, `receive_packet` output, CLI success, or Baseline Encoder Profile v1.
- [x] 4.3 Run OpenSpec validation, focused core tests, feature-status checks, and `cargo xtask ci`.
