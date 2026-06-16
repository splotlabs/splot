# bitstream delta: frame-header-single-picture-bridge-fix

Corrects the single-picture `IsBridge` parse path of
`AV2-5.18.2-FRAME-HEADER-INFO`.

## ADDED Requirements

### Requirement: single-picture bridge frame-header parsing

The frame-header core parser SHALL parse a single-picture `OBU_BRIDGE_FRAME`
(active `single_picture_header_flag == 1`) on the § 5.18.2 `FrameIsIntra` reads
that still terminate on the shared `IsBridge` early-return arm, NOT on the full
intra structure cluster. Concretely it SHALL, after `bridge_frame_ref_idx`, read
`bridge_frame_overwrite_flag` (mirror :4423), the `FrameType == KEY_FRAME`
`refresh_frame_flags` (mirror :4429-4445, read unconditionally), the non-override
`frame_size()` (mirror :4567, no bits for a `cur_mfh_id == 0` bridge),
`screen_content_params()` (mirror :4569), and `intrabc_params()` (mirror :4571).
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

This follows the normative committed spec mirror
(`docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-18-2`). The single-picture bridge
is a corner where AVM and dav2d diverge from the mirror (AVM gates the
`refresh_frame_flags` read on `bridge_frame_overwrite_flag` and reads
`bridge_frame_max_width`/`_height` frame-size fields; dav2d does not model the
path), so any byte-exact / round-trip claim SHALL be gated on AVM differential
confirmation.

#### Scenario: single-picture bridge reads its prefix then stops at the bridge return

- **WHEN** an `OBU_BRIDGE_FRAME` is parsed whose active sequence header has
  `single_picture_header_flag == 1`
- **THEN** the parser reads `bridge_frame_overwrite_flag`, the `KEY_FRAME`
  `refresh_frame_flags`, the non-override `frame_size()`, `screen_content_params()`,
  and `intrabc_params()`, records `InterStop::BruInactiveOrBridgeReturn` on
  `core.inter`, and reports `FrameHeaderParseStatus::UnsupportedUntilFeature`

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
