## Context

`splot-recon` owns a source-backed `ReferenceSlot` and
`ReferenceFrameStore<F>` runtime model for future decoder and encoder
closed-loop reuse. The current store provides validated slot construction,
bounded capacity, `put`/`get`/`take`, `clear`, occupancy, and slot-order
iteration over generic immutable payload handles.

AV2 §7.23 updates reference-frame storage by iterating `i` from zero to
`NUM_REF_FRAMES - 1` and applying work when bit `i` of `refresh_frame_flags` is
set. The broad process also updates `RefValid`, output eligibility, metadata,
motion fields, CDFs, film grain, CCSO, counters, and reference frame planes.
This change intentionally models only the storage loop shape for callers that
already derived a refresh mask and already own immutable current-frame payload
handles.

## Goals / Non-Goals

**Goals:**

- Add a typed mask bounded by `ReferenceSlot::MAX_SLOTS`.
- Add a preflighted store helper that applies a valid mask atomically with
  respect to validation: no payload producer is called and no slot is mutated if
  any selected bit exceeds store capacity.
- Visit selected slots in ascending slot order.
- Keep payload ownership generic and avoid `F: Clone`.
- Return replaced payloads so callers can release or inspect old handles.
- Extend unit and fuzz coverage for valid masks, invalid masks,
  out-of-capacity masks, zero-mask no-ops, replacement returns, non-Clone
  payloads, and explicit `SharedFrame::share()` multi-slot storage.

**Non-Goals:**

- Parsing, inferring, or validating AV2 `refresh_frame_flags`.
- Modeling `NumRefFrames`, `ActiveNumRefFrames`, `RefValid`, key/switch-frame
  `first` handling, CLK reset, or reference poisoning.
- Output buffers, implicit output, show-existing behavior, `FrameCounter`,
  `RefCounter`, order hints, long-term IDs, dimensions, crop metadata, CDFs,
  film grain, CCSO, segment IDs, global motion, or motion vectors.
- Byte-consuming decode, runtime hash/Y4M output changes, resource diagnostics,
  AVM/dav2d invocation, new dependencies, or scheduler changes.

## Decisions

1. Reuse Feature ID `RECON-REFERENCE-FRAME-STORE`.

   Rationale: this is an additive storage API under the existing
   source-backed reference-frame store, not the validator-owned
   `AV2-7.23-REFERENCE-FRAME-UPDATE` semantics row.

2. Represent masks as `ReferenceRefreshMask(u32)`.

   Rationale: callers may naturally pass parsed or inferred mask values in a
   wider integer. Construction rejects any bit above the AV2 §3
   `NUM_REF_FRAMES` slot ceiling while preserving zero as a valid no-op.

3. Make the store helper caller-produced:

   `refresh_slots_with(mask, produce) -> Result<ReferenceRefreshOutcome<F>>`.

   Rationale: the store remains generic and never requires cloning. Callers that
   need the same decoded frame in multiple slots can explicitly call
   `SharedFrame::share()` from the producer for each selected slot.

4. Preflight capacity before production.

   Rationale: a mask can be valid for the global 16-slot ceiling but invalid
   for a smaller store. The helper validates all selected bits before invoking
   the producer so failures cannot partially mutate the store.

5. Extend the existing fuzz target.

   Rationale: `recon_reference_frame_store_bytes` already drives the public
   reference store API with a non-Clone payload oracle. Adding mask operations
   there keeps coverage tied to the same storage model.

## Risks / Trade-offs

- [Risk] The name "refresh" could be mistaken for full AV2 §7.23 support.
  Mitigation: API docs, OpenSpec, matrix notes, and roadmap text state that the
  helper accepts an already-derived mask and remains storage-only.
- [Risk] A valid 16-bit mask can still select slots outside a smaller store.
  Mitigation: `refresh_slots_with` returns a typed
  `ReferenceRefreshMaskOutOfBounds` error before calling the producer.
- [Risk] Returning replacements allocates a small vector.
  Mitigation: the maximum replacement count is bounded by
  `ReferenceSlot::MAX_SLOTS == 16`, and the API preserves old payload handles
  without adding a `Clone` bound.
