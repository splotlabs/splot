# Tasks

## Matrix and docs

- [x] Update `docs/IMPLEMENTATION-MATRIX.toml` `AV2-7.3.8-HLS-AVAILABILITY` notes (film-grain
      RAP-replay wired; QM remains the named residual) and `AV2-5.18.10-FILM-GRAIN-STRUCTURES`
      residual (a) (close it / cross-reference).
- [x] Regenerate `docs/FEATURE-STATUS.md`.

## Implementation

- [x] Add `RapHlsKey::FilmGrain(slot)` with `family` ("film grain model"), `family_section`
      ("7.3.8.8"), and `describe` ("fgm_id {slot}") arms.
- [x] Add the external-HLS suppression arm: `FilmGrain(_) => true` (inexpressible kind).
- [x] `note_resend(RapHlsKey::FilmGrain(slot), obu.header.extended_layer_id)` for each slot a
      film-grain OBU updates, in `record_film_grain`.
- [x] `frame_film_grain_reference_checks` returns the linearly-available slot (`Option<u8>`)
      so the caller can buffer the RAP reference, disjoint from the linear check.
- [x] In `observe_frame_bearing_obu` (after the active-sequence block, `&mut self`), note the
      film-grain RAP reference governed by the frame's extended layer.

## Tests and proof

- [x] Negative: model sent before a CLK random access point, referenced after, not resent ->
      `hls/unavailable-at-random-access-point` (film grain family).
- [x] Positive: model resent in/after the random access point -> silent.
- [x] Disjointness: a never-sent model fires only `frame-header/film-grain-model-unavailable`.
- [x] Suppression: Provided external-HLS mode suppresses the replay.
- [x] Add proof commands to the matrix row.

## Checks

- [x] `cargo fmt --all -- --check`
- [x] `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`
- [x] `cargo test --workspace --all-targets --locked`
- [x] `cargo xtask check-feature-status`
- [x] `cargo xtask ci`
