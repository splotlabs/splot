## ADDED Requirements

### Requirement: Container-aware bitstream entry point

`splot-core` SHALL expose a container-aware bitstream entry point that returns raw
Annex B streams and IVF-wrapped Annex B streams through a single typed result. The
entry point SHALL preserve the existing Annex B envelope parser behavior and SHALL
only add format detection and container metadata.

#### Scenario: Existing Annex B parser is unchanged

- **WHEN** callers invoke the raw Annex B parser directly
- **THEN** it SHALL continue to parse only length-delimited OBUs
- **AND** SHALL NOT require an IVF header or frame record.

#### Scenario: Container parser preserves offsets

- **WHEN** callers invoke the container-aware parser on an IVF file
- **THEN** parsed OBU byte offsets SHALL be relative to the original file
- **AND** SHALL NOT be rebased to frame-payload-local offsets.
