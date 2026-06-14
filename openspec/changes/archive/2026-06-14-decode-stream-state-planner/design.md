## Context

The repository already parses raw Annex B and IVF-wrapped Annex B inputs in
`splot-core`. `splot-decode` owns `DecodeContext`, `DecodeRuntimeConfig`, and
`DecodeOptions`, but it has no parser handoff, stream planning, pixel
reconstruction, frame-hash computation, Y4M output, or byte-consuming decode
API.

PR #101 added the required concurrency model:

- each decode context owns exactly one `splot_parallel::WorkerPool`;
- future data-parallel decode work uses `splot_parallel::prelude` inside
  `WorkerPool::install`;
- direct Rayon/crossbeam/global-pool/ad-hoc worker usage is forbidden outside
  `splot-parallel`;
- observable output must be deterministic across thread counts.

This change builds the next boundary in `splot-decode`: a deterministic,
plan-only traversal over existing `splot-core` parsed stream data.

## Goals / Non-Goals

Goals:

- Add the first source-backed `splot-decode` stream planner API.
- Keep the API rooted in `DecodeContext` so the PR #101 runtime model is part
  of the design from the first decode planning step.
- Consume already parsed `splot_core::stream::ParsedBitstream` data, preserve
  bitstream/container order, and record only bounded metadata in the plan.
- Apply the resource limits this planner can honestly derive:
  `max_input_bytes`, `max_obus`, `max_ivf_frame_records`, and
  `max_frames_to_decode`.
- Select only the documented base layer for the minimal tier:
  `obu_xlayer_id == 0`, `obu_tlayer_id == 0`, and `obu_mlayer_id == 0` for
  non-global OBUs, while enforcing AV2 § 6.2.2 global/local xlayer constraints
  for OBU types that require or forbid `GLOBAL_XLAYER_ID`.
- Treat `OBU_CLOSED_LOOP_KEY` as the only frame candidate in this slice.
- Reject malformed parsed sources and unsupported structures transactionally:
  no partial plan is returned on error.
- Keep AVM/dav2d outside source, tests, build scripts, `xtask`, and CI.

Non-goals:

- No raw-byte `plan_bytes` or CLI file-reading path.
- No new fuzz target; that belongs with the first raw byte-consuming decode
  entry point.
- No pixel reconstruction, symbol decode, tile payload decode, deterministic
  hash digest, Y4M output, output-frame ordering, reference refresh, external
  HLS, multistream composition, or AVM/dav2d fallback.
- No new emitted CLI diagnostic or registry change.
- No `splot-recon` dependency or scheduler behavior.
- No payload parsing through `ObuEnvelope::payload_status()` in this slice.

## Decisions

1. Use a parsed-input API, not a raw-byte API.

   The current `splot-core` partial parsers materialize vectors before
   `splot-decode` can enforce `max_obus`. A raw-byte planner would therefore
   need bounded traversal/fuzz coverage in the same PR. To keep this slice
   PR-sized, the API accepts `ParsedBitstream` plus caller-supplied input length.
   It checks `max_input_bytes` before planner traversal, but does not claim to
   cap parser allocation before the parsed value exists.

2. Add `splot-core` to `splot-decode`, but not `splot-recon`.

   `splot-decode -> splot-core` is the approved future edge for runtime decode
   parser handoff. Reconstruction primitives and decoded-frame storage are not
   needed for stream planning, so `splot-recon` remains out of scope and
   pool-agnostic.

3. Keep the first planner serial and context-owned.

   Stream planning is order-sensitive and cheap compared with future tile and
   reconstruction work. The API lives on `DecodeContext` to ensure future
   parallel work has the right owner. This PR does not call direct Rayon,
   crossbeam, or any new runtime primitive.

4. Store plan metadata, not payload bytes.

   `DecodePlannedObu` records OBU index, source location, offsets, sizes,
   header, role, and optional IVF frame context. It does not expose raw payload
   slices or parsed payload trees. Future tile decode can add a bounded internal
   payload handoff when it also enforces tile-payload limits and fuzz coverage.

5. Keep CLI diagnostics stable.

   Library errors may carry unsupported reasons tied to matrix row
   `decode-stream-state`, but `splot decode` remains intentionally unsupported
   and still emits the existing `cli-decode-entrypoint` descriptor. A future CLI
   wiring PR must add a diagnostic adapter and registry update before rendering
   these library errors to users.

## Proposed API Shape

Public re-exports from `splot-decode`:

```rust
pub struct DecodeStreamInput<'a> {
    pub parsed: &'a splot_core::stream::ParsedBitstream<'a>,
    pub input_len_bytes: u64,
}

impl DecodeContext {
    pub fn plan_stream<'a>(
        &self,
        input: DecodeStreamInput<'a>,
        options: DecodeOptions,
    ) -> Result<DecodeStreamPlan<'a>>;
}

pub struct DecodeStreamPlan<'a>;
pub struct DecodeLayerSelection;
pub struct DecodePlannedObu<'a>;
pub struct DecodeIvfFrameContext;
pub enum DecodePlannedObuRole;
pub enum DecodeSourceIssue;
pub enum DecodeUnsupportedStructure;
```

`DecodeStreamPlan` exposes getters for format, selected layer, input length,
OBU count, frame-candidate count, ordered OBU metadata, and source warnings.
`DecodePlannedObu` exposes metadata getters only: OBU index, byte offset,
declared size, payload length, header, OBU type, role, and optional IVF frame
index/PTS/payload offset.

`DecodeError` gains typed variants for:

- `Limit { source: DecodeLimitError }`;
- `MalformedSource { issue: DecodeSourceIssue }`;
- `UnsupportedStructure { unsupported: DecodeUnsupportedStructure }`.

The unsupported structure metadata includes the stable rule id
`decode/unsupported-feature`, matrix row `decode-stream-state`, Feature ID
`DECODE-STREAM-STATE-PLANNER`, spec section, byte offset when known, and a
stable reason enum.

## Stream Classification

Accepted in this slice:

- raw Annex B parsed by `splot-core`;
- IVF frames whose payloads are parsed as Annex B by `splot-core`;
- temporal delimiter and padding OBUs as ordering markers;
- sequence header OBUs in the selected base layer;
- `OBU_CLOSED_LOOP_KEY` as a frame candidate in the selected base layer.

Rejected as unsupported in this slice:

- non-base temporal, embedded, or extended layer OBUs;
- invalid global/local xlayer bindings, including non-global
  `OBU_TEMPORAL_DELIMITER` / `OBU_MSDO` and global OBU types that § 6.2.2 does
  not permit;
- MSDO, layer configuration record, atlas segment, operating point set, and
  any multistream/external-HLS selection structure;
- open-loop key, leading/regular tile groups, switch, RAS, SEF, TIP, bridge,
  multi-frame header dependent frame paths, and any other frame-carrying OBU
  that is not a closed-loop key candidate;
- metadata, film grain, quantization matrix, content interpretation, buffer
  removal timing, reserved OBU types, and any syntax whose output effect is not
  part of the minimal planner tier.

Rejection is transactional: the planner returns an error and no partial plan.

## Risks / Trade-offs

- The parsed-input API does not bound memory used by `splot-core` before the
  planner runs. This is deliberate for this PR; docs and tests must not present
  it as a raw-byte decode boundary.
- A conservative unsupported set may reject streams the validator can parse.
  This is acceptable because the planner is the minimal decoder tier handoff,
  not a media-player compatibility layer.
- Keeping payload slices out of plan records means a future tile-decode PR will
  need an explicit bounded payload handoff. That is preferable to exposing
  payloads before the planner enforces tile limits.

## Migration Plan

1. Add the OpenSpec delta and validate it.
2. Add `splot-core` as a workspace dependency of `splot-decode`.
3. Implement `crates/splot-decode/src/stream_plan.rs` and wire it through
   `DecodeContext`.
4. Extend `DecodeError` with local planner error variants.
5. Add unit tests for raw/IVF ordering, transactionality, limits,
   unsupported structures, and thread-policy determinism.
6. Update docs, matrix, generated status, and feature tracking.
7. Run OpenSpec, decoder-support, feature-status, concurrency, dependency, and
   full CI gates.

Rollback is straightforward: remove the new module/API, remove the `splot-core`
dependency edge from `splot-decode`, and revert docs/OpenSpec updates. No data
migration or CLI behavior migration is introduced.

## Open Questions

- The first raw byte-consuming decode API remains intentionally deferred. It
  should either add bounded traversal in `splot-core` or a capped decode-side
  walker, then add the required decode fuzz target in the same change.
