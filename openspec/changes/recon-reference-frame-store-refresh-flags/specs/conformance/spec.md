## ADDED Requirements

### Requirement: reconstruction reference-frame store refresh-mask fuzz coverage

The `recon_reference_frame_store_bytes` cargo-fuzz target SHALL extend its
bounded public-API operation sequences to cover `ReferenceRefreshMask` and
`ReferenceFrameStore<F>::refresh_slots_with` under the existing
`CONF-RECON-REFERENCE-FRAME-STORE-FUZZ` evidence row, without parsing AV2
bitstreams or claiming AV2 reference-update conformance.

#### Scenario: fuzz target exercises refresh-mask typed paths

- **WHEN** the fuzz target receives arbitrary bytes
- **THEN** it normalizes those bytes into valid and invalid refresh masks,
  store capacities, slots, and payload metadata
- **AND** it exercises mask construction, slot containment, selected-slot
  iteration, zero-mask no-ops, successful refreshes, replacement returns, and
  valid-mask-but-out-of-capacity failures through public typed return paths

#### Scenario: refresh operation matches an oracle

- **WHEN** a valid mask is applied to a valid store by the fuzz target
- **THEN** occupied count, emptiness, selected slot contents, replacement
  returns, non-selected slot preservation, and ascending selected-slot order
  match a bounded oracle model after the operation

#### Scenario: invalid masks and capacity failures do not mutate

- **WHEN** arbitrary bytes produce a mask with bits above the 16-slot ceiling or
  a valid mask that selects a slot outside the active store capacity
- **THEN** construction or refresh returns the appropriate typed error
- **AND** the fuzz target verifies that no producer-side mutation and no store
  mutation occurs for out-of-capacity refresh attempts
