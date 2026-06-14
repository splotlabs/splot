## Context

`splot-decode` currently stops at deterministic stream planning. It can traverse
raw Annex B / IVF bytes with safe limits and select closed-loop-key frame
candidates, but it does not enter a tile-group payload. `splot-core` now has two
pieces needed for the next step:

- § 5.20.1 tile-payload framing records via
  `headers::tile_group::parse_tile_group_framing`.
- § 8.2 symbol decoder primitives via `symbol::SymbolDecoder`.

AV2 § 5.20.1 then calls `init_symbol(tileSize)`, `decode_tile()`, and
`exit_symbol()` for each non-bridge tile, and § 8.3 maps CDF-coded syntax
elements to tile-local CDF arrays. Full `decode_tile()` immediately expands into
block syntax, CDF bank ownership, coefficient parsing, prediction, transform,
reconstruction, and reference state. That is too large for one PR and too early
for a hash-output success path.

This change therefore creates the narrow runtime boundary that future tile
syntax can plug into: a `splot-decode` step that derives a borrowed plan for the
minimal single-tile closed-loop-key case, proves the eligible tile byte slice
that would be passed to `init_symbol(tileSize)`, and returns a structured
unsupported diagnostic at the first unimplemented `decode_tile()` / § 8.3
boundary. It must not run `exit_symbol()` because no block syntax has consumed
symbols yet.

## Goals / Non-Goals

**Goals:**

- Add a new Feature ID, `DECODE-TILE-PAYLOAD-BOUNDARY`, for this decoder-support
  step.
- Add an internal `splot-decode` module for tile payload planning / decode
  boundary checks, expected as `crates/splot-decode/src/tile_payload.rs`.
- Reuse existing `splot-core` `TileFraming` / `TileGroupFraming` and the
  `SymbolDecoder` initialization contract instead of reimplementing framing or
  entropy arithmetic.
- Enforce `DecodeLimits::max_tile_count` and
  `DecodeLimits::max_tile_payload_bytes` before iterating or handing tile bytes
  to the symbol decoder.
- Return a `decode/unsupported-feature` diagnostic owned by matrix row
  `tile-payload-decode`, with Feature ID `DECODE-TILE-PAYLOAD-BOUNDARY`, for the
  first unsupported `decode_tile()` / § 8.3 syntax-element CDF selection point.
- Limit the first source-backed plan to the minimal tier: selected base layer,
  `OBU_CLOSED_LOOP_KEY`, complete intra first tile group, one tile, one tile
  group, and bounded payload size.
- Keep runtime output behavior unchanged: `splot decode` still exits
  unsupported after byte planning unless a future change wires this boundary into
  an actual decode path.
- Preserve the concurrency boundary: any future caller runs this from
  `splot-decode` and, if parallelized later, only inside
  `DecodeContext::pool().install(...)`; `splot-core` and `splot-recon` stay
  scheduler-free.

**Non-Goals:**

- No implementation of § 5.20.2-§ 5.20.10 block syntax.
- No multi-tile or multi-tile-group runtime support.
- No § 8.3 CDF-array selection implementation beyond naming it as the
  unsupported boundary.
- No Tile/Saved CDF bank structs, CDF initialization/copyback/averaging, or
  `frame_end_update_cdf()`.
- No pixel prediction, transform, coefficient reconstruction, frame hashes,
  Y4M output, reference refresh, film grain, or encoder RDO API.
- No `splot-cli` output-success path.
- No AVM/dav2d source, snippets, binaries, submodules, dependencies, wrappers,
  scripts, build probes, CI jobs, runtime process execution, local absolute
  paths, or mandatory reference-tool tests.

## Decisions

1. Put the boundary in `splot-decode`, not `splot-core`.

   `splot-core` owns spec syntax primitives and already provides tile framing
   and symbol arithmetic. The decision to treat a tile as unsupported runtime
   decode behavior is decoder policy, not core parsing. Keeping this in
   `splot-decode` avoids making `splot-core` know about decoder diagnostics,
   support rows, or runtime limits.

2. Add a borrowed plan type separate from `DecodeStreamPlan`.

   `DecodeStreamPlan` is metadata-only and should stay useful for source
   planning. The tile boundary needs borrowed payload bytes and exact source
   spans, so it should use a separate crate-private shape such as
   `DecodeTilePayloadPlan<'a>` / `DecodeTileWorkUnit<'a>`. The plan should keep
   deterministic order and source provenance: source kind, OBU index/offset,
   optional IVF frame context, selected layer, tile number, tile row/column or
   MI range when available, payload offset, and payload length.

3. Enforce the minimal tier before tile planning.

   The first implementation should not try to infer full frame state from
   partial data. It should only produce tile work for base-layer
   `OBU_CLOSED_LOOP_KEY` candidates with a complete intra first tile group, one
   tile, one tile group, and a bounded nonzero payload. Missing sequence/header
   facts, multiple tiles, non-first tile groups, bridge/BRU paths, and out-of-tier
   OBU roles should return structured unsupported metadata.

4. Initialize symbol state only as a handoff proof.

   For each non-bridge tile with nonzero `tileSize`, the boundary proves the
   exact tile-data slice and may construct `SymbolDecoder::with_base_and_config`
   with `CdfUpdateMode::Disabled` when `disable_cdf_update` is true. It then
   immediately returns the unsupported `decode_tile()` / § 8.3 result. Full
   `exit_symbol()` validation depends on how many bits `decode_tile()` consumed,
   so this change should not call it or claim trailing-bit validation beyond the
   existing framing-provable zero-size tile defect.

5. Use `decode/unsupported-feature` rather than new diagnostic codes.

   The condition is an unsupported AV2 feature boundary, not malformed input and
   not a resource limit. `DecodeUnsupportedStructure` already carries matrix row,
   feature id, spec section, reason, OBU type, and byte offset for planner-level
   unsupported structures. This change should either extend that detail shape
   carefully or add a parallel runtime unsupported detail while keeping the same
   stable rule id and registry entry.

6. Keep bridge and BRU behavior explicit but unsupported.

   The minimal decoder tier is all-intra and non-bridge. If the boundary is
   asked to handle bridge tiles, inactive BRU tiles, or inter/TIP-only tile
   semantics, it should return structured unsupported metadata instead of
   silently assuming they behave like ordinary non-bridge tiles.

7. Treat `frame_end_update_cdf()` and `decode_frame_wrapup()` as named residuals.

   § 5.20.1 invokes those when `tg_end == NumTiles - 1`. This change may record
   whether the tile group reaches the final-tile-group boundary, but must not
   claim CDF copyback, frame wrapup, output, or reference-state support.

## Risks / Trade-offs

- [Risk] A boundary helper can be mistaken for supported tile decode. ->
  Mitigation: keep matrix status `partial`, make the returned diagnostic cite
  `tile-payload-decode`, and document that block syntax/reconstruction remains
  unsupported.
- [Risk] Synthetic tests might validate only toy inputs. -> Mitigation: test
  positive handoff, zero/truncated/out-of-range resource-limit edges, bridge/BRU
  unsupported paths, and explicit deferral of `exit_symbol()` / CDF copyback
  with small self-contained byte slices.
- [Risk] Duplicating § 5.20.1 framing logic would create drift from
  `splot-core`. -> Mitigation: consume `TileGroupFraming` / `TileFraming`
  records and leave existing parser tests as the framing authority.
- [Risk] Wiring into full byte planning can require more frame facts than the
  current `DecodeStreamPlan` carries. -> Mitigation: keep the first API explicit
  and borrowed; return unsupported when complete minimal-tier facts are absent
  instead of guessing.
- [Risk] Future parallel decode could accidentally bypass PR #101. ->
  Mitigation: no direct Rayon/crossbeam/thread usage in this change, and
  document that any future parallel caller must run through `DecodeContext`'s
  `splot_parallel::WorkerPool`.
- [Risk] The exact § 8.3 CDF bank shape is broad and not settled. ->
  Mitigation: stop at the named unsupported CDF-selection boundary and avoid a
  public CDF-bank API that would constrain future encoder/recon design.

## Migration Plan

1. Add the boundary API and tests in `splot-decode`, scoped to the minimal
   single-tile case.
2. Update docs/matrices/generated status to mark `tile-payload-decode` as
   `partial` with the new Feature ID and proof.
3. Keep CLI runtime behavior unsupported; no user-facing success migration is
   required.
4. Archive the OpenSpec delta in the same PR.

## Open Questions

- Whether the runtime unsupported detail can stay crate-private or requires a
  public `DecodeDiagnosticDetails` variant should be decided during
  implementation after inspecting CLI JSON rendering impact.
- Local AVM/dav2d evidence is useful for later § 8.3/default-CDF work, but this
  boundary can proceed without running reference tools because it does not claim
  block syntax or reconstructed output behavior.
