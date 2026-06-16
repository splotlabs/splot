## ADDED Requirements

### Requirement: Reference frame store refresh-mask storage helper

The decoder support model SHALL track `ReferenceRefreshMask` and
`ReferenceFrameStore<F>::refresh_slots_with` under Feature ID
`RECON-REFERENCE-FRAME-STORE` and decoder support row
`reference-frame-store`. The helper SHALL accept an already-derived
`refresh_frame_flags`-style mask bounded by AV2 §3 `NUM_REF_FRAMES == 16`,
SHALL validate every selected bit against the store capacity before mutation,
SHALL treat a zero mask as a no-op, SHALL visit selected slots in ascending
slot order, SHALL store one caller-produced immutable payload handle per
selected slot, SHALL return replaced payloads without requiring `F: Clone`, and
SHALL cite AV2 §5.18.2, §6.17.2, and §7.23 as mask-derivation and storage-loop
motivation without claiming full reference-frame update semantics.

#### Scenario: matrix records storage-only refresh-mask support

- **WHEN** `cargo xtask check-decoder-support` validates the decoder support
  matrix
- **THEN** row `reference-frame-store` remains linked to Feature ID
  `RECON-REFERENCE-FRAME-STORE`
- **AND** it records `ReferenceRefreshMask` and `refresh_slots_with` as
  source-backed `splot-recon` storage API evidence
- **AND** it does not claim parsing or inferring `refresh_frame_flags`,
  `NumRefFrames`/`ActiveNumRefFrames` derivation, `RefValid`, key/switch-frame
  validity, CLK reset, output buffers, show-existing behavior, counters,
  order hints, dimensions, crop metadata, CDFs, film grain, CCSO, segment IDs,
  global motion, motion vectors, byte-consuming decode, resource diagnostics,
  AVM/dav2d invocation, or full AV2 §7.23 conformance

#### Scenario: refresh-mask helper is transactional before payload production

- **WHEN** a caller applies a valid mask whose selected bits exceed a smaller
  reference-frame store capacity
- **THEN** the helper returns a typed `ReconError`
- **AND** no payload producer is called
- **AND** existing store contents and occupancy remain unchanged

#### Scenario: explicit sharing keeps zero-copy ownership visible

- **WHEN** a caller refreshes multiple selected slots with handles to the same
  immutable decoded-frame payload
- **THEN** the caller supplies one handle per selected slot, such as by calling
  `SharedFrame::share()` in the producer
- **AND** the store does not clone frame payloads or require `F: Clone`
