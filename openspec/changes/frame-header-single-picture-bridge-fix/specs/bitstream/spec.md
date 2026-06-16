# bitstream delta: frame-header-single-picture-bridge-fix

Corrects the single-picture `IsBridge` parse path of
`AV2-5.18.2-FRAME-HEADER-INFO`.

## ADDED Requirements

### Requirement: single-picture bridge frame-header parsing

The frame-header core parser SHALL parse a single-picture `OBU_BRIDGE_FRAME`
(active `single_picture_header_flag == 1`) on the § 5.18.2 `FrameIsIntra` reads
that still terminate on the shared `IsBridge` early-return arm, NOT on the full
intra structure cluster. Concretely it SHALL, after `bridge_frame_ref_idx`, read
`bridge_frame_overwrite_flag` (mirror :4423), then the OVERWRITE-GATED
`refresh_frame_flags` (per § 6.17.2 + AVM, NOT the § 5.18.2 KEY-arm literal — see the
contradiction note): when `bridge_frame_overwrite_flag == 0` it SHALL be inferred
`1 << bridge_frame_ref_idx` with no bits read; when `== 1` it SHALL be read (the AVM
bridge arm — `has_refresh_frame_flags` + `frame_to_refresh` on the
`enable_short_refresh_frame_flags` path, else `f(NumRefFrames)`). It SHALL then read
the non-override `frame_size()` (mirror :4567, no bits for a `cur_mfh_id == 0`
bridge), `screen_content_params()` (mirror :4569), and `intrabc_params()` (mirror
:4571).
It SHALL also consume the decidable `film_grain_config()` tail of the `IsBridge`
early-return arm (mirror :5011 / § 5.18.10.1): `apply_grain` is inferred from
`single_picture_header_flag` + `immediate_output_frame == 1` (mirror :8169-8171),
and when `film_grain_params_present` is set it reads `fgm_id` f(3) + `grain_seed`
f(16) — the last modeled frame-header bits, decidable without reference state — so
`consumed_bits` covers the mandatory frame-header syntax. When
`film_grain_params_present` is unknown (a bounded sequence-header stop),
`apply_grain` is undecidable, so the parser SHALL stop before the grain read. It
SHALL then stop with `InterStop::BruInactiveOrBridgeReturn` and report
`FrameHeaderParseStatus::UnsupportedUntilFeature`, preserving the parsed prefix on
`core.inter`. It SHALL NOT read `disable_cdf_update` and SHALL NOT enter the
`quantization_params()` / `segmentation_params()` / deblocking / cdef / ccso
cluster or the § 5.18.2 intra tail (those are inferred from the referenced frame on
the `IsBridge` arm). An EOF inside the modeled prefix OR the film-grain tail SHALL
be reported as the facts-preserving
`FrameHeaderParseStatus::StoppedInsideInterControl`.

CONTRADICTION: § 5.18.2 syntax would read `refresh_frame_flags` unconditionally on
the `if ( FrameType == KEY_FRAME )` arm (:4429-4445), but § 6.17.2 semantics
(`06-syntax-structures-semantics.md` :4522-4524) states the `overwrite == 0`
inference, and AVM (`decodeframe.c:8394-8422`) implements the overwrite-gated
reading. Per the maintainer decision the parser follows § 6.17.2 + AVM so it matches
the reference decoder. AVM additionally reads two `bridge_frame_max_width`/`_height`
frame-size fields the § 5.18.2 `FrameIsIntra` `frame_size()` does not (splot follows
§ 5.18.2 there); dav2d does not model the path. Any byte-exact / round-trip claim
SHALL be gated on AVM differential confirmation.

#### Scenario: single-picture bridge reads its prefix then stops at the bridge return

- **WHEN** an `OBU_BRIDGE_FRAME` is parsed whose active sequence header has
  `single_picture_header_flag == 1`
- **THEN** the parser reads `bridge_frame_overwrite_flag`, the overwrite-gated
  `refresh_frame_flags`, the non-override `frame_size()`, `screen_content_params()`,
  and `intrabc_params()`, records `InterStop::BruInactiveOrBridgeReturn` on
  `core.inter`, and reports `FrameHeaderParseStatus::UnsupportedUntilFeature`

#### Scenario: overwrite == 0 infers refresh_frame_flags without reading bits

- **WHEN** a single-picture bridge has `bridge_frame_overwrite_flag == 0`
- **THEN** `refresh_frame_flags` is inferred `1 << bridge_frame_ref_idx` with no
  bits read (per § 6.17.2 + AVM), and the next field parsed is the non-override
  `frame_size()`

#### Scenario: single-picture bridge does not read the intra structure cluster

- **WHEN** a single-picture bridge frame header is parsed
- **THEN** `disable_cdf_update` is not read and `core.intra_tail`,
  `core.tile_info`, and `core.quantization_params` stay `None` (the pre-fix bug
  read them and reached a bogus `IntraHeaderComplete`)

#### Scenario: single-picture bridge consumes the decidable film-grain tail

- **WHEN** a single-picture bridge is parsed whose active sequence header has
  `film_grain_params_present == 1`
- **THEN** the parser consumes `fgm_id` f(3) + `grain_seed` f(16) (apply_grain
  inferred 1) so `consumed_bits` covers them, then stops with
  `InterStop::BruInactiveOrBridgeReturn`

#### Scenario: truncation inside the single-picture bridge prefix or grain tail is preserved

- **WHEN** the OBU payload ends inside the modeled single-picture-bridge prefix or
  inside the mandatory film-grain tail
- **THEN** the fields parsed before the EOF are preserved on `core.inter` and the
  parser reports `FrameHeaderParseStatus::StoppedInsideInterControl`
