# Proposal: fix the single-picture bridge frame-header parse path

## Feature IDs

- `AV2-5.18.2-FRAME-HEADER-INFO` (the `frame_header_info()` core parser)

## Why

`parse_core_body` mis-models a single-picture `OBU_BRIDGE_FRAME` (a frame whose
active sequence header has `single_picture_header_flag == 1`). AV2 § 5.18.2
(mirror `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-18-2`) runs the
`if ( single_picture_header_flag )` branch (:4131-4142) BEFORE the
`if ( IsBridge ) FrameType = INTER_FRAME` else-arm (:4205), so a single-picture
bridge is forced to `KEY_FRAME` / `FrameIsIntra = 1` / `immediate_output_frame = 1`
and travels the *intra* (`FrameIsIntra`) reads — but `IsBridge` is still set, so it
ends on the shared `IsBridge` early-return arm (:4971/:5045), NOT the full intra
structure cluster.

The pre-fix parser routed it through the generic `parse_intra_tail` /
`parse_intra_structures`, which:

1. never read `bridge_frame_overwrite_flag` (the spec reads it at :4423, guarded
   only by `if ( IsBridge )`);
2. read `disable_cdf_update` f(1) (the spec's `IsBridge` arm at :4971 takes the
   `if` branch, so the :5039 `else` that reads it is never taken);
3. read the full `quantization_params()` / `segmentation_params()` /
   `delta_q_params()` / deblocking / cdef / ccso cluster and the § 5.18.2 intra
   tail — but the `IsBridge` arm (:4983-5083) reads a zero-bit `tile_info()`,
   INFERS `base_q_idx = RefBaseQIdx[bridge_frame_ref_idx]` from the referenced
   frame, and forces the whole cluster off with no bits.

So the parser read the wrong bits and reached a bogus `IntraHeaderComplete`. Its
own invariant (`parse_intra_tail`: "the intra path always has `TipFrameMode ==
TIP_FRAME_DISABLED` and `!IsBridge`") is violated for a single-picture bridge
(`IsBridge == 1`).

Discovered while building the #4i frame-header writer (PR #165); the writer was
fixed to reject all bridges. This is the parser-side fix.

## What Changes

1. `parse_core_body` routes a single-picture frame that is also `IsBridge` to a
   new `parse_single_picture_bridge_tail` instead of `parse_intra_tail`.
2. The new path reads the § 5.18.2 `FrameIsIntra` prefix —
   `bridge_frame_overwrite_flag` f(1) (:4423), then the **overwrite-gated**
   `refresh_frame_flags` (per § 6.17.2 + AVM, see the Differential note: `overwrite
   == 0` → inferred `1 << bridge_frame_ref_idx`, no bits; `overwrite == 1` → read it,
   the AVM bridge `has_refresh_frame_flags`/`frame_to_refresh` short arm or
   `f(NumRefFrames)`), the non-override `frame_size()` (:4567, default dims, no bits),
   `screen_content_params()` (:4569) and `intrabc_params()` (:4571), and the
   decidable § 5.18.10.1 `film_grain_config()` tail of the `IsBridge` early-return
   arm (:5011) — `apply_grain` is inferred from `single_picture_header_flag` +
   `immediate_output_frame == 1` (mirror :8169-8171), reading `fgm_id` f(3) +
   `grain_seed` f(16) when `film_grain_params_present` — then stops at the
   reference-derived `base_q_idx` with `InterStop::BruInactiveOrBridgeReturn`,
   reporting `FrameHeaderParseStatus::UnsupportedUntilFeature` and preserving the
   parsed prefix on `core.inter`. It reuses `parse_frame_size`,
   `parse_screen_content_params_full`, `parse_intrabc_params_full`,
   `parse_film_grain_config`, and `finish_inter_control` (the same
   EOF→`StoppedInsideInterControl` machinery the non-single bridge uses). The
   film-grain tail is consumed (not exposed — the bridge stays unsupported coverage)
   so `consumed_bits` covers the mandatory frame-header syntax and a truncation there
   (or in the refresh) is reported as a truncation rather than a silent coverage stop
   (codex PR review); when `film_grain_params_present` is unknown the parse stops
   before the grain read, like the intra / SEF tails.
3. The buggy test `frame_header_core_single_picture_bridge_takes_intra_key_path`
   (whose premise was the bug) is replaced; positive, data-dependent, EOF, and
   film-grain (read + truncation) tests are added.

## Differential note (a spec-internal contradiction)

The single-picture bridge is a degenerate corner where the **normative spec is
internally inconsistent**:

- **refresh_frame_flags**: § 5.18.2 *syntax* would read it unconditionally on the
  `if ( FrameType == KEY_FRAME )` arm (:4429-4445; a single-picture bridge has
  `FrameType == KEY_FRAME`, so the `else if ( IsBridge && !bridge_frame_overwrite_flag )`
  inferred arm at :4489 is unreachable). But § 6.17.2 *semantics*
  (`06-syntax-structures-semantics.md` :4522-4524) states — unconditionally — that
  `bridge_frame_overwrite_flag == 0` means `refresh_frame_flags` is **not present**
  and is inferred `1 << bridge_frame_ref_idx`; **AVM** (`decodeframe.c:8394-8422`)
  implements exactly that overwrite-gated reading. Per the maintainer decision
  (codex PR review) splot follows **§ 6.17.2 + AVM** (overwrite-gated): a validator
  must match the reference decoder so it does not misparse a real AVM-encoded
  `overwrite == 0` stream (which would otherwise eat `NumRefFrames` phantom bits and
  desync the rest of the header).
- **frame size**: AVM's `setup_frame_size` reads two `bridge_frame_max_width`/`_height`
  fields the § 5.18.2 `FrameIsIntra` `frame_size()` does not; splot follows § 5.18.2
  (no frame-size bits) here — a remaining documented divergence.
- **dav2d** does not implement the single-picture bridge at all (the
  `single_picture` / `bridge` tokens appear only in its AVM-instrumentation
  patches), so it is not an oracle for this feature.

This corner therefore needs AVM differential confirmation before any
byte-exact / round-trip claim; the implementation and matrix row record the
divergence.

## Non-goals

- No reference-frame-state modeling: `base_q_idx = RefBaseQIdx[refIdx]` / `DeltaQ`
  stay unmodeled (reference-derived, no bits), so the frame stays an
  `UnsupportedUntilFeature` coverage stop. The decidable `film_grain_config()` tail
  IS consumed for bit-accuracy, but its parsed value is not exposed (no new field /
  inspector view) — that exposure, if wanted, is a follow-up.
- No writer change beyond updating the now-stale `reject_single_picture_bridge`
  test/comment (the writer still rejects all bridges; revisiting its gate is a
  separate follow-up).
- No new diagnostic; the (non-truncated) stop stays on the silent coverage side
  (`UnsupportedUntilFeature`), consistent with the non-single bridge.

## Acceptance criteria

- [ ] A single-picture bridge reads `bridge_frame_overwrite_flag` + the
      overwrite-gated `refresh_frame_flags` (inferred `1 << bridge_frame_ref_idx`
      when `overwrite == 0`; read when `== 1`) + non-override `frame_size()` +
      `screen_content_params()` + `intrabc_params()` + the decidable
      `film_grain_config()` tail, then stops with
      `InterStop::BruInactiveOrBridgeReturn` and
      `FrameHeaderParseStatus::UnsupportedUntilFeature`; `consumed_bits` covers the
      mandatory frame-header syntax (incl. `fgm_id` / `grain_seed` when grain present).
- [ ] It never reads `disable_cdf_update` and never enters the quant/segmentation/
      loop-filter cluster; `core.intra_tail` / `core.tile_info` /
      `core.quantization_params` stay `None`.
- [ ] An EOF inside the modeled prefix OR the grain tail preserves the parsed facts
      and reports `StoppedInsideInterControl`.
- [ ] `cargo xtask ci` is green; the matrix proof records the new tests.
