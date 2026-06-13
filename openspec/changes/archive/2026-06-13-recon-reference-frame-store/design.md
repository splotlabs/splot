## Context

`splot-recon` currently provides immutable decoded frame and plane model types.
The decoder roadmap already names AV2 § 7.23 reference-frame storage as an
encoder-reuse milestone, but the matrix row is still `todo` and no source API
can hold frame payloads in reference slots. The next safe slice is a runtime
container only: no bitstream traversal, no reference refresh process, and no
prediction.

## Goals / Non-Goals

**Goals:**

- Add typed reference slot identifiers and a bounded `ReferenceFrameStore<F>` in
  `splot-recon`.
- Store immutable caller-owned frame payload values by replacement into
  validated slots.
- Expose capacity, occupancy, immutable lookup, clearing, and stable slot-order
  iteration.
- Keep all errors typed through `ReconError`.
- Update matrix/docs/OpenSpec with self-contained tests.

**Non-Goals:**

- No AV2 reference refresh semantics, frame selection, motion compensation, or
  decoded byte-consuming path.
- No `splot-decode` dependency and no `DecodeLimits` use from `splot-recon`.
- No AVM/dav2d integration, fixtures, scripts, wrappers, CI hooks, or local path
  metadata.
- No new dependencies or crate dependency graph changes.

## Decisions

1. The store lives in `crates/splot-recon/src/reference.rs`.

   Keeping the API in `splot-recon` makes it reusable by future decode and
   encoder closed-loop code without creating a dependency from reconstruction
   primitives back into `splot-decode`.

2. Slots are represented by `ReferenceSlot`.

   A typed slot avoids exposing raw integers at public boundaries. The slot is a
   zero-based repository/runtime index into the store, not a claim that all AV2
   reference-frame semantics have landed.

3. Capacity is explicit and bounded by constructor validation.

   `ReferenceSlot::MAX_SLOTS` is `16`, matching the AV2 § 3
   `NUM_REF_FRAMES` constant that motivates § 7.23 storage.
   `ReferenceSlot::new(index)` rejects indices outside that source-backed
   ceiling. `ReferenceFrameStore::with_capacity(capacity)`
   rejects zero capacity and capacity above the same ceiling. This keeps the
   model predictable for tests while leaving caller policy limits such as
   `max_reference_slots` in `splot-decode`.

4. Storage is immutable-frame replacement.

   `put(slot, frame)` replaces the frame in a slot and returns the previous
   frame if any. `take(slot)` clears a slot and returns its frame. Borrowing is
   immutable through `get(slot)` and ascending slot-order `entries()`. The
   payload type is generic so future reconstructed-reference payloads do not
   need to fabricate output-emission metadata.

5. The API does not model AV2 refresh or lifetime semantics.

   Future byte-consuming decode code must translate parsed AV2 reference update
   state into store operations and apply `DecodeLimits` before allocation. This
   change only provides the safe container.

## Risks / Trade-offs

- [Risk] A generic store can be mistaken for complete AV2 § 7.23 behavior. →
  Mitigation: docs, matrix notes, and public comments explicitly say this is a
  runtime storage model only.
- [Risk] A fixed maximum slot count could be read as full reference semantics. →
  Mitigation: the maximum is only the slot-index ceiling motivated by
  `NUM_REF_FRAMES = 16`; active sequence reference counts, `RefValid`,
  refresh masks, and output scheduling remain future decoder responsibilities.
- [Risk] Cloning frames can be costly. → Mitigation: store operations move owned
  frames and return borrows; clone behavior only comes from caller payloads and
  tests.
