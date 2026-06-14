## Context

The merged `decode-context-tile-payload-handoff` change proved that
`DecodeContext` can run the existing crate-private tile-payload boundary inside
the context-owned `splot_parallel::WorkerPool`. The remaining gap is that no
decode path derives `TilePayloadBoundaryInput` from parser output. Tests build
the payload bytes, tile grid, frame facts, and framing data by hand.

The next slice remains plan-only. It derives one minimal closed-loop-key,
complete-intra, first-and-only tile-group boundary from source-backed parser
facts, then stops at the existing structured unsupported `decode_tile()` boundary.

Relevant AV2 anchors:

- § 5.2.1 for OBU sizing and payload boundaries.
- § 5.18.1 and § 6.17.1 for first-header / copied-header continuity.
- § 5.18.2 for `FrameIsIntra`, `IsBridge`, `disable_cdf_update`, `use_bru`,
  and `bru_inactive` preconditions.
- § 5.18.6.1 for `base_q_idx`.
- § 5.18.7.2, § 5.18.7.3, and § 6.17.7.2 for tile layout, MI starts,
  `context_update_tile_id`, and `TileSizeBytes`.
- § 5.19 and § 6.18 for tile-group structure, `headerBytes`, payload size, and
  tile-group range continuity.
- § 5.20.1 and § 6.19.1 for per-tile payload framing.
- § 5.20.2.1, § 8.2.2, § 8.2.4, and § 8.3 for the reached-but-unsupported tile
  syntax and symbol/CDF boundary.

## Goals / Non-Goals

**Goals:**

- Add a crate-private adapter that derives tile-payload boundary input from a
  planned frame candidate and borrowed `splot-core` parser output.
- Treat parser output as hostile because several parser structs expose public
  fields; validate metadata and byte containment before slicing.
- Expose `FrameHeaderCore::disable_cdf_update` as an `Option<bool>` parser fact,
  instead of hardcoding CDF update mode in `splot-decode`.
- Preserve PR #101 concurrency: the derived boundary runs through
  `DecodeContext` and `splot_parallel::WorkerPool`.
- Preserve PR #113 / PR #114 carry-forward: do not reintroduce duplicated
  Annex B/IVF parsing or unsupported/limit precedence regressions.

**Non-Goals:**

- No public tile-payload API.
- No CLI decode success path.
- No continuation tile-group support through guessed "last header" state.
- No inter, TIP, bridge, BRU, multi-tile, or multi-tile-group support beyond
  honest unsupported stops.
- No `decode_tile()`, recursive block syntax, `exit_symbol()` after real
  syntax, CDF copyback/averaging mutation, reconstruction, hashes, Y4M output,
  output scheduling, reference refresh, or `decode_frame_wrapup()`.
- No `splot-decode -> splot-recon` dependency edge.
- No AVM/dav2d source, snippets, binaries, dependencies, build probes, wrappers,
  scripts, CI jobs, required `xtask` commands, or mandatory tests.

## Decisions

1. Derive from borrowed parser output, not `DecodeStreamPlan` alone.

   `DecodeStreamPlan` carries deterministic order and provenance metadata, but
   it does not own the borrowed OBU payload slice or the payload start offset
   needed to derive § 5.20 input safely. The adapter will accept the selected
   `DecodePlannedObu` only as provenance and will validate it against the
   borrowed `ObuEnvelope`.

   Alternative rejected: re-slice raw bytes from `DecodeStreamPlan` metadata.
   That can mismatch metadata and bytes, especially with forged parsed input.

2. Keep the adapter crate-private and plan-only.

   The API should live near `tile_payload` in `splot-decode`, return a
   crate-private plan/error, and call the existing context handoff. It should
   not widen public `DecodeStreamPlan` or public diagnostic contracts before a
   runtime decode path exists.

   Alternative rejected: expose public tile-payload planning APIs now. That
   would freeze unstable tile facts before runtime decode derives them.

3. Separate output lifetime from local framing facts.

   The current tile boundary uses one lifetime for payload bytes and the
   borrowed `TileGroupFraming`. A derivation adapter may build `TileGroupFraming`
   locally, call the boundary immediately, and return a plan that borrows only
   the tile payload bytes. The implementation should split the lifetime so the
   returned `DecodeTilePayloadPlan` is tied to payload bytes, not to local
   framing storage.

4. Expose `disable_cdf_update` from `splot-core`.

   The frame-header core parser already reads the intra-path bit. The adapter
   needs that fact to set symbol/CDF update policy, so `FrameHeaderCore` should
   carry it as `Option<bool>`. Hardcoding `false` would be spec dishonest.

5. Reject undecidable or unsupported state explicitly.

   The first bridge only accepts a first tile group with
   `FrameHeaderParseStatus::IntraHeaderComplete`, `frame_is_intra == Some(true)`,
   `is_bridge == false`, one tile, one tile group, complete § 5.19 structure,
   exact § 5.20 payload containment, and parser-derived tile/quant/CDF facts.
   Anything else returns a local crate-private error or the existing
   tile-boundary unsupported metadata.

## Risks / Trade-offs

- Parsed inputs can be forged because parser structs expose public fields:
  mitigate with checked equality and containment validation before slicing.
- Byte arithmetic can overflow while deriving `payload_base` or ranges:
  mitigate with checked add/convert helpers and typed local errors.
- The adapter can accidentally overclaim runtime decode support:
  mitigate by keeping it crate-private, documenting the unsupported boundary,
  and leaving CLI behavior unchanged.
- Multi-tile and continuation groups are useful soon but stateful:
  mitigate by naming them future work until validator-equivalent frame/layer
  pairing state is threaded into decode.
- AVM/dav2d evidence could be mistaken for a requirement:
  mitigate by adding no reference-tool code or manifests and stating that no
  local reference run is required for this plan-only bridge.
