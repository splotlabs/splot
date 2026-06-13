## ADDED Requirements

### Requirement: Reference frame store runtime model

The repository SHALL provide a source-backed `splot-recon` reference-frame-store
runtime model for future decoder and encoder closed-loop reuse. The model SHALL
store immutable caller-owned frame payload values in typed zero-based
`ReferenceSlot` positions without requiring output-emission metadata.
`ReferenceSlot::MAX_SLOTS` SHALL be `16`, matching the AV2 § 3
`NUM_REF_FRAMES` slot ceiling that motivates § 7.23 storage, while active
sequence reference counts and `RefValid` state remain future decoder
responsibilities. The model SHALL validate slot construction, store capacity,
and slot bounds before access; support replacement, clearing, immutable lookup,
occupancy reporting, and stable slot-order iteration; and report failures
through typed `ReconError` values. The model SHALL cite AV2 § 3 for the slot
ceiling and AV2 § 7.23 as the reference-frame-storage motivation while making
clear that AV2 reference refresh, prediction, frame selection, output
scheduling, and byte-consuming decode semantics remain future work.

#### Scenario: Store rejects invalid capacity

- **WHEN** a caller constructs a `ReferenceSlot` above `ReferenceSlot::MAX_SLOTS`
  or creates a reference-frame store with zero capacity or capacity above the
  source-backed slot ceiling
- **THEN** construction returns a typed `ReconError` and no store is created

#### Scenario: Store validates slot bounds

- **WHEN** a caller reads, writes, or clears a slot outside the store capacity
- **THEN** the operation returns a typed `ReconError` without panicking

#### Scenario: Store replaces immutable frames

- **WHEN** a caller puts an immutable frame payload into an empty valid slot and
  then puts another frame into the same slot
- **THEN** the first operation reports no previous frame
- **AND** the second operation returns the previous immutable frame payload
- **AND** later lookup returns the replacement frame

#### Scenario: Store iterates occupied slots in slot order

- **WHEN** a store contains frames in multiple non-contiguous slots
- **THEN** iteration returns only occupied slots in ascending `ReferenceSlot`
  order with immutable frame borrows

#### Scenario: Runtime model does not claim full AV2 refresh semantics

- **WHEN** a reader checks the decoder roadmap and support matrix
- **THEN** the reference-frame-store row states that the source-backed API is a
  safe runtime storage model only
- **AND** byte-consuming decode, AV2 reference refresh semantics,
  `RefValid`/output scheduling, `decode/resource-limit` emission, AVM/dav2d
  invocation, and CI reference-tool requirements remain unsupported
