# validator spec delta

## ADDED Requirements

### Requirement: Metadata persistence and cancellation lifetime

`splot-validate` SHALL track active metadata per `(obu_xlayer_id, metadata_type)` within
a coded video sequence and apply the AV2 v1.0.0 § 6.16.3 `muh_persistence_idc` and
`muh_cancel_flag` semantics, including cross-layer propagation via the sequence header's
layer-dependency maps.

#### Scenario: cancel clears active metadata for the layer

- **GIVEN** a metadata unit of a type with BASIC persistence active for an extended layer
- **AND** a later metadata unit of the same type with `muh_cancel_flag == 1` for that layer
- **WHEN** the validator runs
- **THEN** the metadata SHALL no longer be considered active for that layer.

#### Scenario: global persistence ignores cancel

- **GIVEN** a metadata unit with `muh_persistence_idc == GLOBAL_PERSISTENCE`
- **WHEN** a later `muh_cancel_flag == 1` of the same type is observed
- **THEN** the global metadata SHALL remain active (the cancel is a no-op, § 6.16.3).

### Requirement: Scan-type CVS-wide consistency

`splot-validate` SHALL enforce the AV2 v1.0.0 § 6.16.10 cross-OBU scan-type constraints:
`mps_source_scan_type_idc` / `mps_pic_struct_type` consistency with the
content-interpretation `ci_scan_type_idc`, and the requirement that
`mps_pic_struct_type` stays within a single permitted group for all pictures of the CVS.

#### Scenario: pic-struct group changes within a CVS

- **GIVEN** two scan-type metadata units in the same CVS whose `mps_pic_struct_type`
  values fall into different Table 6.18 groups
- **WHEN** the validator runs
- **THEN** it SHALL emit a CVS-consistency error.

### Requirement: Decoded-frame-hash verification

`splot-validate` SHALL verify `metadata_decoded_frame_hash` (§ 6.16.13) against the
decoded output samples once a decoder is available.

#### Scenario: hash mismatch

- **GIVEN** a decoded frame and a `metadata_decoded_frame_hash` whose recomputed MD5 over
  the output samples differs from the signaled value
- **WHEN** the validator runs (with a decoder)
- **THEN** it SHALL emit a hash-mismatch error.

### Requirement: Metadata placement inside coded frame units

`splot-validate` SHALL validate that prefix metadata (`metadata_is_suffix == 0`) appears
before the frame data and suffix metadata (`metadata_is_suffix == 1`) after it within a
coded frame unit (AV2 v1.0.0 § 7.3.3 / § 7.3.4), once frame-header and tile-group parsing
locate the frame-data boundary.

#### Scenario: suffix metadata before frame data

- **GIVEN** a coded frame unit whose suffix metadata appears before the frame data
- **WHEN** the validator runs (with frame/tile parsing)
- **THEN** it SHALL emit a placement error.
