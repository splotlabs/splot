# Design: sequence timing consistency and HLS availability

## Architecture

The change keeps the existing crate split and dependency direction:

```text
splot-cli -> splot-validate -> splot-core
splot-cli -> splot-core
```

No parser or validator logic moves into the CLI. The work lands as two bounded,
spec-traceable slices:

- **PR A — content interpretation OBU + timing consistency.** Parse
  `content_interpretation_obu()` (AV2 §5.15) far enough to reach `timing_info()`
  (AV2 §5.4.12), wire it into the OBU dispatcher and inspector, and add
  cross-embedded-layer timing-consistency diagnostics (AV2 §6.4.12).
- **PR B — HLS availability store.** Add `ValidationOptions` with an optional
  caller-provided external-HLS set, an in-band availability store, and the
  multi-frame-header → sequence-header reference check (AV2 §7.3.8).

## Parser layering (PR A)

`content_interpretation_obu()` is a new child module under `headers/`
(`headers/content_interpretation.rs`). It reuses `parse_timing_info()` from
`headers/sequence.rs` rather than re-deriving timing syntax.

A new `rg(n)` descriptor (AV2 §4.11.10, Rice-Golomb) is added to `bitio.rs` with
focused tests, because `ci_color_description_idc` is coded `rg(2)`. The descriptor
is panic-free: it returns a typed `Error::InvalidRg` when the unary prefix does not
terminate within 32 bits (the spec requires the descriptor never return a value
less than 0).

The parser is **complete**: every field in the §5.15 table is read, including the
`ci_color_description`, `ci_chroma_sample_position`, and `ci_aspect_ratio_info`
branches. It never silently skips payload bits; a fully parsed CI OBU then has its
§5.2.1 payload tail (`obu_extension_flag` + `trailing_bits`, since the OBU is
extensible) validated by the validator and dispatcher.

## State model (PR A timing, PR B availability)

`ValidatorContext` gains:

- `content_interpretations: BTreeMap<(ExtendedLayerId, EmbeddedLayerId), ContentInterpretationRecord>`
  — one record per `(obu_xlayer_id, obu_mlayer_id)` within the modeled
  coded-video-sequence scope, holding a payload fingerprint, the present
  `TimingInfo` (if any), and the source offset.
- `HlsAvailabilityStore` (PR B) — in-band availability of sequence headers
  (consumed by the multi-frame-header reference check).

`ValidationOptions { external_hls: ExternalHlsMode }` (PR B) defaults to
`Disabled`. The existing `Validator::validate_bytes` is preserved and delegates to
a new `validate_bytes_with_options(data, &ValidationOptions::default())`, so the
public API does not break.

## Scope boundary: the CLK/OLK activation blocker

> Exact CLK/OLK-driven sequence-header activation requires parsing the activating
> frame header and its `seq_header_id_in_frame_header`. This change may model
> availability and improve bounded checks, but it must not claim exact activation
> until `AV2-5.18-FRAME-HEADER` lands.

Several §7.3.x semantics are rooted in the closed/open-loop key (CLK/OLK) frame
header, which references and *activates* a `seq_header_id` and marks coded-video-
sequence (CVS) and random-access-point (RAP) boundaries. Because the activating
CLK *follows* its sequence header in OBU order, none of the following can be
decided exactly at the OBU level, and the validator keeps sound-over-complete
approximations (it never rejects a conformant stream):

- Exact CVS / RAP boundaries (`AV2-7.3.6-CODED-EXTENDED-LAYER-UNIT`). The existing
  conservative per-xlayer reset on the CLK is retained for the fingerprint and
  timing/CI state.
- "HLS OBUs must be resent at each random access point" (AV2 §7.3.8.1). The in-band
  availability store is therefore kept **monotonic** (never cleared): an object
  seen earlier in the bitstream stays available, so the validator never falsely
  reports an object "unavailable". The cost is a missed-resend false negative,
  which is the intentional sound-over-complete bias.
- Frame-header references to sequence headers (`seq_header_id_in_frame_header`),
  to multi-frame headers (`cur_mfh_id`), and to film-grain/QM objects remain
  out of scope until the frame-header parser exists.

Timing consistency and repeated-CI identity (PR A) use the per-xlayer CVS reset so
a legal reconfiguration in a later CVS is not flagged; the comparison is scoped to
embedded layers within the same extended layer's modeled CVS, which is a sound
subset of the spec's "across all embedded layers" requirement.

## Diagnostics

PR A (cross-embedded-layer timing, §6.4.12; repeated CI, §6.14):

```text
content-interpretation/reserved-bits-nonzero            (warning; §6.14)
content-interpretation/chroma-sample-position-out-of-range (error; §6.14)
content-interpretation/aspect-ratio-idc-out-of-range     (error;  §6.14)
content-interpretation/repeated-ci-not-identical         (error;  §6.14)
sequence-header/timing-display-tick-mismatch             (error;  §6.4.12)
sequence-header/timing-time-scale-mismatch               (error;  §6.4.12)
sequence-header/timing-equal-picture-interval-mismatch   (error;  §6.4.12)
sequence-header/timing-num-ticks-mismatch                (error;  §6.4.12)
```

`ci_reserved_2bit` is mapped to a **warning**, not an error: AV2 §6.14 says it
"must be set to 0" but also that "the value shall be ignored by a decoder", so a
non-zero value is a producer anomaly rather than a hard decode-breaking conformance
violation. Strict mode still escalates warnings to a failing verdict.

The chroma-sample-position (`<= 5`) and aspect-ratio-idc (`<= 16` when `!= 255`)
checks are hard errors (both are "requirement of bitstream conformance" in §6.14).
The repeated-CI check compares parsed §6.14 *information* (a weaker requirement than
the sequence header's bit-identity in §7.3.8). It excludes the decoder-ignored
`ci_reserved_2bit`, and compares the color description and aspect ratio by their
**derived** values — `splot-core` normalizes the §6.14 Table 6.13 color presets and
the §5.15 aspect tables (`ColorDescription::derived` / `AspectRatioInfo::derived_sar`)
— so an alias-equivalent re-encoding (a preset vs. the equivalent explicit triple or
SAR) is not flagged while genuinely different color or aspect information is. Color
and aspect are compared only when present in both OBUs; a present-vs-absent
difference is left unflagged because the absent side defaults to unspecified values
that could alias the present one (a sound-over-complete false negative, never a
false-positive).

PR B (HLS availability, §7.3.8):

```text
mfh/sequence-header-unavailable   (error;   §7.3.8.6)  — concrete MFH reference case
hls/external-hls-disabled          (warning; §7.3.8.1)  — advisory when external HLS is off
hls/unavailable-sequence-header                          — reserved for the generic
                                                            frame-header reference path
                                                            (blocked on AV2-5.18-FRAME-HEADER)
```

`hls/unavailable-sequence-header` is the generic id reserved for a frame-header
`seq_header_id_in_frame_header` reference; it is **not emitted yet** because that
reference path requires the frame-header parser. It is documented here and in the
matrix notes so the namespace is stable when the generic path lands.

## Non-goals (unchanged from the proposal)

- No frame-header parser, tile parser, or entropy decoder.
- No fabricated activation semantics: where availability or timing depends on
  syntax not yet parsed, the check stays bounded/partial rather than guessing.
- No external-HLS assumptions unless the caller supplies the objects explicitly
  (default `ExternalHlsMode::Disabled`).

## Testing strategy

Parser (`splot-core`): all-flags-false CI, timing-present CI, reserved-bits
non-zero, EOF in the fixed header, EOF inside timing, aspect-ratio extended-SAR
path, chroma-sample-position path, and a `read_rg` positive/negative/EOF set, plus
the existing never-panic proptests extended to the CI parser.

Validator (`splot-validate`): same/different timing across two embedded layers for
each timing field, repeated non-identical CI, MFH referencing an available vs.
missing sequence header, default options not assuming external HLS, and the
external-HLS-provided acceptance path.
