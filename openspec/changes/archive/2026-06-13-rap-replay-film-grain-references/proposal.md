# Change: rap-replay-film-grain-references

## Feature IDs

- `AV2-7.3.8-HLS-AVAILABILITY`
- `AV2-5.18.10-FILM-GRAIN-STRUCTURES`

## Why

The § 7.3.8.1 random-access-point (RAP) HLS-availability replay (`RapReplayTracker`,
`hls/unavailable-at-random-access-point`) already covers sequence headers, multi-frame
headers, operating point sets, layer-configuration records, and local atlas segments — but
not film-grain model references. The linear `frame-header/film-grain-model-unavailable`
check (§ 6.17.10.1 / § 7.3.8.8) reads the monotonic per-slot `FilmGrainState.available[]`,
which is never reset at a random access point, so it OVER-approximates presence and silently
UNDER-reports the random-access direction: a film-grain model sent before a random access
point and not resent in or after it is unavailable when a decoder starts there, yet the
linear test sees it present. This is residual (a) on `AV2-5.18.10-FILM-GRAIN-STRUCTURES` and
the film-grain half of the RAP-replay residual on `AV2-7.3.8-HLS-AVAILABILITY`.

Film-grain is the clean case to wire: availability is strictly monotonic (no reset/poison),
so "no qualifying resend visible from the random access point" soundly implies "unavailable
from that start" — § 6.12-style hidden-default availability does not exist for film grain (a
model only exists once a film-grain OBU defines its slot). The replay operates purely on
send-temporal facts and stays disjoint from the linear check (only linearly-available
references are buffered), exactly mirroring the existing sequence-header wiring.

## Scope

- Spec sections: § 7.3.8.1 (random-access-point availability), § 7.3.8.8 (film grain OBU
  availability).
- Crates/modules: `crates/splot-validate/src/context/rap_replay.rs` (new
  `RapHlsKey::FilmGrain` variant + family/section/describe + external-HLS suppression arm),
  `crates/splot-validate/src/context/film_grain.rs` (`note_resend` in `record_film_grain`;
  `frame_film_grain_reference_checks` surfaces the linearly-available slot),
  `crates/splot-validate/src/context/frame_headers.rs` /
  `frame_header_checks.rs` (note the frame's film-grain RAP reference).
- CLI/docs/tests: matrix notes updated (residual closed); no new `rule_id` (feeds the
  existing `hls/unavailable-at-random-access-point`).

## Non-goals

- Quantizer-matrix RAP references (`using_qmatrix`/`qm_*`) — wired in the follow-up
  `rap-replay-qm-references` change (QM's reset/poison interaction needs its own focused
  review; the `RapHlsKey` infra added here makes that follow-up small).
- The § 7.4.4 leading-temporal-unit content-identity divergence (a documented residual, not
  a diagnostic).
- No change to the linear `frame-header/film-grain-model-unavailable` check itself.

## Acceptance criteria

- [ ] `AV2-7.3.8-HLS-AVAILABILITY` notes updated (film-grain replay wired; QM remains
      future). `AV2-5.18.10-FILM-GRAIN-STRUCTURES` residual (a) closed.
- [ ] `RapHlsKey::FilmGrain` participates in family/section/describe and the external-HLS
      suppression (inexpressible kind → blanket-suppress under any Provided mode).
- [ ] Positive tests: a model resent in/after the random access point stays silent.
- [ ] Negative tests: a model sent only before the random access point and referenced after
      fires `hls/unavailable-at-random-access-point` (film grain family).
- [ ] Suppression: a Provided external-HLS mode suppresses the film-grain replay.
- [ ] Disjointness: an in-band-unavailable model fires only the linear
      `frame-header/film-grain-model-unavailable`, not the replay.
- [ ] `cargo xtask ci` passes.
