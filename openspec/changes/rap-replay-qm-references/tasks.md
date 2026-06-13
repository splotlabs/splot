# Tasks

## Matrix and docs

- [x] Update `docs/IMPLEMENTATION-MATRIX.toml` `AV2-7.3.8-HLS-AVAILABILITY` notes (QM RAP
      replay wired; the umbrella RAP residual closed).
- [x] Regenerate `docs/FEATURE-STATUS.md`.

## Implementation

- [x] Add `RapHlsKey::QmLevel { level }` with `family` ("quantizer matrix level"),
      `family_section` ("7.3.8.9"), `describe` arms, and the external-HLS suppression arm.
- [x] `note_resend` per level a QM OBU makes available in `check_quantizer_matrix` — each
      `qm.levels` entry, and EVERY level on a `qm_bit_map == 0` reset-to-defaults.
- [x] `frame_qm_reference_checks` returns the linearly-available referenced levels
      (`!poisoned && available.is_some()`, under Disabled), disjoint from the linear checks.
- [x] `FrameRapReferences` carries `qm_levels`; `observe_frame_bearing_obu` buffers each as a
      RAP reference governed by the frame's extended layer.

## Tests and proof

- [x] Negative: level sent before a RAS random access point (surviving its reset), referenced
      by an INTRA_ONLY frame, not resent -> `hls/unavailable-at-random-access-point`.
- [x] Positive: level resent after the random access point -> silent.
- [x] Positive: a `qm_bit_map == 0` reset-to-defaults after the random access point counts as
      a resend for every level.
- [x] Disjointness: a never-sent level fires only `frame-header/qm-level-unavailable`.
- [x] Suppression: a Provided external-HLS mode suppresses the QM replay.
- [x] Add proof commands to the matrix row.

## Checks

- [x] `cargo fmt --all -- --check`
- [x] `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`
- [x] `cargo test --workspace --all-targets --locked`
- [x] `cargo xtask check-feature-status`
- [x] `cargo xtask ci`
