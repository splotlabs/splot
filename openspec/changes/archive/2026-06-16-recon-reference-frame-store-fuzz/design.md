## Context

`splot-recon` owns a source-backed `ReferenceSlot` and
`ReferenceFrameStore<F>` storage model for future decoder and encoder
closed-loop reuse. The API is intentionally generic over caller-owned immutable
payloads so it does not couple reference storage to output emission metadata or
to a concrete decoded-frame type.

Existing unit tests cover slot construction, capacity validation, replacement,
clearing, immutable lookup, occupied-count tracking, slot-order iteration, and
shared-frame handle storage. This change adds fuzz coverage for the storage API
itself by driving bounded operation sequences against a simple oracle model. It
does not implement AV2 reference refresh or byte-consuming decode.

## Goals / Non-Goals

**Goals:**

- Add `recon_reference_frame_store_bytes` to the fuzz crate.
- Drive only public `splot-recon` `ReferenceSlot` and
  `ReferenceFrameStore<F>` APIs.
- Use a small non-Clone payload to keep the store's generic move/replacement
  contract exercised without copying frame data.
- Check invalid capacity and slot construction, valid-but-out-of-capacity slot
  errors, `contains_slot`, `get`, `put`, `take`, `clear`, `occupied`,
  `is_empty`, and ascending `entries` iteration.
- Keep the operation count and memory footprint bounded for CI fuzz smoke.

**Non-Goals:**

- Parsing AV2 bitstreams, invoking `splot-decode`, or changing CLI behavior.
- Modeling AV2 `NumRefFrames`, `ActiveNumRefFrames`, `RefValid`,
  `frame_to_refresh`, `refresh_frame_flags`, show-existing behavior, output
  scheduling, motion-field storage, or § 7.23 frame-store update semantics.
- Emitting `decode/resource-limit` or other decoder diagnostics.
- Using real `DecodedFrame` payloads, AVM/dav2d/ffmpeg, filesystem I/O,
  network I/O, subprocesses, or new dependencies.

## Decisions

1. Fuzz `ReferenceFrameStore<Payload>` directly.

   Rationale: The API is generic storage. A tiny non-Clone payload exercises the
   move/replacement contract more directly and with less resource risk than
   constructing decoded-frame payloads.

2. Maintain a separate metadata oracle.

   Rationale: The store owns non-Clone payloads, while the target still needs to
   compare expected slot contents after replacements and removals. A
   `Vec<Option<PayloadMeta>>` oracle stays copyable and avoids depending on
   internal store representation.

3. Normalize arbitrary bytes into a bounded operation sequence.

   Rationale: Fuzzer time should cover store state transitions instead of
   unbounded allocation or input-length-driven loops. The target caps operations
   and normalizes raw slots into both valid and invalid ranges.

4. Match typed errors, not display text.

   Rationale: `ReconError` variants and fields are the API contract. Error
   messages remain presentation details and should not create fuzz false
   positives.

## Risks / Trade-offs

- [Risk] The new row is mistaken for AV2 reference refresh support.
  Mitigation: name it as `recon`/storage fuzz and state that `RefValid`,
  `refresh_frame_flags`, output scheduling, and § 7.23 update semantics remain
  out of scope.

- [Risk] The oracle becomes more complex than the store under test.
  Mitigation: keep payload metadata tiny, model only `Vec<Option<PayloadMeta>>`,
  and assert simple public-state invariants after each operation.

- [Risk] Fuzz iterations grow with input length.
  Mitigation: cap operation count to a fixed maximum and default missing bytes
  to zero.
