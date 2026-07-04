## ADDED Requirements

### Requirement: Frame-level Wiener NS LR Unit Activity Summary
The tile partition traversal boundary SHALL report how many supported
frame-level Wiener NS LR units were consumed and how many selected
`RESTORE_WIENER_NONSEP` from the AV2 §5.20.10.5 `use_wiener_ns` symbol. A
`use_wiener_ns` value of zero SHALL be counted as an inactive `RESTORE_NONE`
unit, and a non-zero value SHALL be counted as an active
`RESTORE_WIENER_NONSEP` unit. The boundary SHALL preserve the existing
transactional CDF behavior: failed traversal attempts MUST NOT commit LR-unit
CDF mutations.

#### Scenario: Inactive frame-level units are reported
- **WHEN** a supported superblock-root LR frontier consumes frame-level Wiener
  NS units whose `use_wiener_ns` symbols all select zero
- **THEN** the frontier reports the consumed unit count
- **AND** it reports zero active Wiener NS units
- **AND** it commits the same CDF updates and symbol position as the existing LR
  syntax frontier

#### Scenario: Active frame-level units are reported
- **WHEN** a supported superblock-root LR frontier consumes a frame-level Wiener
  NS unit whose `use_wiener_ns` symbol selects non-zero
- **THEN** the frontier reports at least one active Wiener NS unit
- **AND** callers can fail closed before claiming loop-restoration
  reconstruction or output support

#### Scenario: Rejected LR paths stay transactional
- **WHEN** the LR frontier fails due to a resource limit, unsupported SDP plane
  range, unsupported LR variant, or invalid unit geometry
- **THEN** the work unit's tile CDF subset remains unchanged
- **AND** no inactive-or-active LR-unit support claim is made for that input
