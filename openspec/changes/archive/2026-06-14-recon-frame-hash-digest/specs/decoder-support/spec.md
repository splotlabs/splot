## MODIFIED Requirements

### Requirement: Decoded frame hash input serialization

The repository SHALL provide source-backed `splot-recon` APIs that serialize
canonical decoded-frame hash input bytes from a validated caller-supplied
`DecodedFrame<T>` and compute the repository-owned `splot-dfh-sha256-v1`
decoded-frame digest. The byte stream SHALL use identifier
`av2-output-samples-v1` and raw output variant `raw_intermediate_output`. The
serializer and digest computation SHALL follow AV2 § 6.16.13 sample-byte
conversion for the frame's modeled visible output rows: visible samples only, Y
then U then V plane order for non-monochrome frames, Y only for monochrome
frames, raster scan order within each plane, one byte per 8-bit sample, and
little-endian two-byte values for samples with bit depth greater than 8. The
serializer and digest computation SHALL exclude stride padding, backing
allocation padding, output index, frame dimensions, pixel format metadata, OBU
bytes, container metadata, and decoded-frame-hash metadata from the byte stream
and digest. The digest API SHALL expose stable algorithm, byte-stream, and
variant identifiers, raw 32-byte digest access, and lowercase hex formatting.
AV2 metadata MD5 verification, byte-consuming decode, output ordering,
film-grain synthesis, Y4M output, AVM/dav2d invocation, and CI reference-tool
requirements remain future work.

#### Scenario: Visible rows exclude padding

- **WHEN** a decoded frame stores non-visible padding or stride samples around a
  visible crop rectangle
- **THEN** hash input serialization writes only the visible samples in raster
  order
- **AND** padding and stride samples do not appear in the output bytes
- **AND** `splot-dfh-sha256-v1` is computed over the same visible bytes only

#### Scenario: Monochrome and chroma plane order

- **WHEN** a decoded frame is monochrome
- **THEN** hash input serialization writes only Y-plane bytes
- **AND** `splot-dfh-sha256-v1` hashes only those Y-plane bytes
- **WHEN** a decoded frame has chroma planes
- **THEN** hash input serialization writes Y bytes, then U bytes, then V bytes
- **AND** `splot-dfh-sha256-v1` hashes that same Y/U/V byte order

#### Scenario: Sample byte width follows bit depth

- **WHEN** a decoded frame has 8-bit output samples
- **THEN** hash input serialization writes one byte per visible sample
- **AND** `splot-dfh-sha256-v1` hashes those one-byte sample values
- **WHEN** a decoded frame has greater-than-8-bit output samples
- **THEN** hash input serialization writes each visible sample as two
  little-endian bytes
- **AND** `splot-dfh-sha256-v1` hashes those little-endian sample bytes

#### Scenario: Byte length matches emitted bytes

- **WHEN** a caller asks for the hash input byte length and writes the same frame
  to an in-memory byte buffer
- **THEN** the checked byte length equals the number of emitted bytes

#### Scenario: Writer errors are propagated

- **WHEN** the caller-provided writer returns an error while receiving hash
  input bytes
- **THEN** serialization returns that writer error without panicking

#### Scenario: Digest identifiers and hex formatting are stable

- **WHEN** a caller computes a decoded-frame digest
- **THEN** the digest reports contract identifier `splot.decoded_frame_hash`
- **AND** it reports contract version `1`
- **AND** it reports algorithm identifier `splot-dfh-sha256-v1`
- **AND** it is tied to byte-stream identifier `av2-output-samples-v1`
- **AND** it is tied to variant identifier `raw_intermediate_output`
- **AND** raw digest access returns exactly 32 bytes
- **AND** text formatting returns exactly 64 lowercase hexadecimal characters

#### Scenario: Digest matches canonical byte stream

- **WHEN** a caller computes a decoded-frame digest and also writes the same
  frame through the canonical hash-input serializer
- **THEN** the digest equals SHA-256 over the emitted canonical byte stream

#### Scenario: Runtime model does not claim decode output

- **WHEN** a reader checks the decoder roadmap and support matrix
- **THEN** the deterministic-frame-hash row states that source-backed hash input
  serialization and `splot-dfh-sha256-v1` digest computation exist for
  caller-supplied decoded frames
- **AND** AV2 metadata MD5 verification, byte-consuming decode, output ordering,
  film-grain synthesis, Y4M output, AVM/dav2d invocation, and CI reference-tool
  requirements remain unsupported
