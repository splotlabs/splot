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
2. The new path reads exactly the spec-mirror prefix on the `FrameIsIntra` arm —
   `bridge_frame_overwrite_flag` f(1) (:4423), the `KEY_FRAME` arm
   `refresh_frame_flags` (:4429-4445, `f(NumRefFrames)`, read unconditionally),
   the non-override `frame_size()` (:4567, default dims, no bits),
   `screen_content_params()` (:4569) and `intrabc_params()` (:4571) — then stops
   at the `IsBridge` early-return arm (:4971) with
   `InterStop::BruInactiveOrBridgeReturn`, reporting
   `FrameHeaderParseStatus::UnsupportedUntilFeature` and preserving the parsed
   prefix on `core.inter`. It reuses `read_refresh_frame_flags`, `parse_frame_size`,
   `parse_screen_content_params_full`, `parse_intrabc_params_full`, and
   `finish_inter_control` (the same EOF→`StoppedInsideInterControl` machinery the
   non-single bridge uses).
3. The buggy test `frame_header_core_single_picture_bridge_takes_intra_key_path`
   (whose premise was the bug) is replaced; positive, data-dependent, and EOF
   tests are added.

## Differential note (spec mirror vs references)

splot follows the NORMATIVE committed spec mirror. The single-picture bridge is a
degenerate corner where the references disagree on bit layout:

- **AVM** (`av2/decoder/decodeframe.c`) keys the refresh ladder on OBU type, not
  `FrameType`, so it gates the `f(NumRefFrames)` refresh on
  `bridge_frame_overwrite_flag` (overwrite == 0 → `refresh = 1 << bridge_ref_idx`,
  no `f(NumRefFrames)`), and its `setup_frame_size` reads two
  `bridge_frame_max_width`/`_height` fields the spec `frame_size()` does not. So an
  AVM-encoded single-picture bridge can differ in total header length.
- **dav2d** does not implement the single-picture bridge at all (the
  `single_picture` / `bridge` tokens appear only in its AVM-instrumentation
  patches), so it is not an oracle for this feature.

This corner therefore needs AVM differential confirmation before any
byte-exact / round-trip claim; the implementation and matrix row record the
divergence.

## Non-goals

- No reference-frame-state modeling of the bridge tail (`base_q_idx =
  RefBaseQIdx[refIdx]`, `tile_info()`, `film_grain_config()`); the parse stops at
  the early-return arm exactly like the non-single bridge.
- No writer change (the #4i writer already rejects bridges; revisiting that is a
  separate follow-up).
- No new diagnostic; the stop stays on the silent coverage side
  (`UnsupportedUntilFeature`), consistent with the non-single bridge.

## Acceptance criteria

- [ ] A single-picture bridge reads `bridge_frame_overwrite_flag` + the `KEY`
      `refresh_frame_flags` + non-override `frame_size()` + `screen_content_params()`
      + `intrabc_params()`, then stops with `InterStop::BruInactiveOrBridgeReturn`
      and `FrameHeaderParseStatus::UnsupportedUntilFeature`.
- [ ] It never reads `disable_cdf_update` and never enters the quant/segmentation/
      loop-filter cluster; `core.intra_tail` / `core.tile_info` /
      `core.quantization_params` stay `None`.
- [ ] An EOF inside the modeled prefix preserves the parsed facts and reports
      `StoppedInsideInterControl`.
- [ ] `cargo xtask ci` is green; the matrix proof records the new tests.
