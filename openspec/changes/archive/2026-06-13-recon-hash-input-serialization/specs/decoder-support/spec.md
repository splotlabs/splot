## ADDED Requirements

### Requirement: Decoded frame hash input serialization

The repository SHALL provide a source-backed `splot-recon` API that serializes
canonical decoded-frame hash input bytes from a validated caller-supplied
`DecodedFrame<T>` without computing a digest. The byte stream SHALL use
identifier `av2-output-samples-v1` and raw output variant
`raw_intermediate_output`. The serializer SHALL follow AV2 § 6.16.13
sample-byte conversion for the frame's modeled visible output rows: visible
samples only, Y then U then V plane order for non-monochrome frames, Y only for
monochrome frames, raster scan order within each plane, one byte per 8-bit
sample, and little-endian two-byte values for samples with bit depth greater
than 8. The serializer SHALL exclude stride padding, backing allocation padding,
output index, frame dimensions, pixel format metadata, OBU bytes, container
metadata, and decoded-frame-hash metadata from the byte stream. The serializer
SHALL expose checked byte-length calculation and a writer-based output method,
while SHA-256 digest computation, AV2 metadata MD5 verification,
byte-consuming decode, output ordering, film-grain synthesis, Y4M output,
AVM/dav2d invocation, and new dependencies remain future work.

#### Scenario: Visible rows exclude padding

- **WHEN** a decoded frame stores non-visible padding or stride samples around a
  visible crop rectangle
- **THEN** hash input serialization writes only the visible samples in raster
  order
- **AND** padding and stride samples do not appear in the output bytes

#### Scenario: Monochrome and chroma plane order

- **WHEN** a decoded frame is monochrome
- **THEN** hash input serialization writes only Y-plane bytes
- **WHEN** a decoded frame has chroma planes
- **THEN** hash input serialization writes Y bytes, then U bytes, then V bytes

#### Scenario: Sample byte width follows bit depth

- **WHEN** a decoded frame has 8-bit output samples
- **THEN** hash input serialization writes one byte per visible sample
- **WHEN** a decoded frame has greater-than-8-bit output samples
- **THEN** hash input serialization writes each visible sample as two
  little-endian bytes

#### Scenario: Byte length matches emitted bytes

- **WHEN** a caller asks for the hash input byte length and writes the same frame
  to an in-memory byte buffer
- **THEN** the checked byte length equals the number of emitted bytes

#### Scenario: Writer errors are propagated

- **WHEN** the caller-provided writer returns an error while receiving hash
  input bytes
- **THEN** serialization returns that writer error without panicking

#### Scenario: Runtime model does not claim hash computation

- **WHEN** a reader checks the decoder roadmap and support matrix
- **THEN** the deterministic-frame-hash row states that source-backed hash input
  serialization exists
- **AND** SHA-256 digest computation, AV2 metadata MD5 verification,
  byte-consuming decode, output ordering, film-grain synthesis, Y4M output,
  AVM/dav2d invocation, and CI reference-tool requirements remain unsupported
