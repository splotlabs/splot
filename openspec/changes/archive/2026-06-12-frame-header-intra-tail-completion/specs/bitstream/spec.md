# bitstream delta: frame-header-intra-tail-completion

Advances `AV2-5.18.8-TRANSFORM-CODING-MODES`, the film-grain frame-state
row, the § 5.18.9 intra arm, and `AV2-5.18.2-FRAME-HEADER-INFO` to a
complete intra frame header.

## ADDED Requirements

### Requirement: complete intra frame-header parsing

The frame-header core parser SHALL parse the remaining § 5.18.2 intra
tail — `read_tx_mode()` (§ 5.18.8.1), `frame_reference_mode()`
(§ 5.18.8.3, no bits on intra), `skip_mode_params()` (§ 5.18.8.2), the
intra-inferred `allow_bawp`/`allow_warpmv_mode`, `reduced_tx_set`, the
§ 5.18.9.1 intra arm of `global_motion_params()`, and
`film_grain_config()` (§ 5.18.10.1, reusing the § 5.14 film-grain-model
parser) — so an intra frame header parses to completion, the
show-existing-frame path included. An EOF inside the tail SHALL preserve
the already-parsed facts.

#### Scenario: intra header completes

- **WHEN** an intra frame header parses through its § 5.18.2 tail
- **THEN** the status reports a complete header and every tail structure
  is surfaced

#### Scenario: SEF completes

- **WHEN** a show-existing-frame header parses through film_grain_config
- **THEN** its status reports completion instead of stopping early

#### Scenario: EOF preserves facts

- **WHEN** the payload ends inside the new tail structures
- **THEN** the already-parsed frame facts survive and the status reports
  the truncation
