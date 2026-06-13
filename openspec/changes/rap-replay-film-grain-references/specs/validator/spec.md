# validator delta: rap-replay-film-grain-references

Advances `AV2-7.3.8-HLS-AVAILABILITY` (and closes residual (a) on
`AV2-5.18.10-FILM-GRAIN-STRUCTURES`) by wiring film-grain model references into the
§ 7.3.8.1 random-access-point availability replay.

## ADDED Requirements

### Requirement: film-grain references participate in the random-access-point replay

The validator SHALL replay film-grain model references (`apply_grain == 1`, `fgm_id`)
through the § 7.3.8.1 random-access-point availability tracker
(docs/spec/av2/1.0.0/07-decoding-process.md#s-7-3-8-1): a film-grain OBU send is recorded as
a `RapHlsKey::FilmGrain(slot)` resend event, and a frame's linearly-available film-grain
reference is buffered and resolved at temporal-unit completion against every governing
random access point, firing `hls/unavailable-at-random-access-point` when no qualifying
resend is visible from a start point. The replay predicate stays disjoint from the linear
`frame-header/film-grain-model-unavailable` check (only linearly-available references are
buffered), and the film-grain family is inexpressible by `ExternalHlsSet`, so any Provided
external-HLS mode suppresses the film-grain replay (blanket inexpressible-kind policy).

#### Scenario: model dropped at a random access point

- **WHEN** a film-grain OBU defines slot `s` before a random access point, the slot is never
  resent in or after that random access point, and a later frame applies grain referencing
  `fgm_id == s`
- **THEN** an error diagnostic `hls/unavailable-at-random-access-point` naming the film-grain
  model family (§ 7.3.8.1) is produced

#### Scenario: model resent at the random access point stays silent

- **WHEN** the film-grain model for the referenced slot is resent in or after the governing
  random access point's temporal unit
- **THEN** no `hls/unavailable-at-random-access-point` diagnostic is produced for it

#### Scenario: replay is disjoint from the linear check

- **WHEN** a frame applies grain referencing a slot no film-grain OBU has ever defined
- **THEN** only the linear `frame-header/film-grain-model-unavailable` fires and the replay
  stays silent

#### Scenario: external-HLS suppression

- **WHEN** validation runs under any Provided external-HLS mode
- **THEN** the film-grain random-access-point replay does not fire (the model may be supplied
  by external means)

## MODIFIED Requirements

(none)

## REMOVED Requirements

(none)
