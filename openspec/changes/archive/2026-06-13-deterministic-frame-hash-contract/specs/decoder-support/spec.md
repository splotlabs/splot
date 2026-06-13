## ADDED Requirements

### Requirement: Deterministic decoded-frame hash contract

The repository SHALL document a deterministic decoded-frame hash contract before
any decoder output row, Y4M output row, fixture manifest, or encoder roundtrip
expectation is marked supported. The contract SHALL be tracked by
`DOC-DETERMINISTIC-FRAME-HASH-CONTRACT`, SHALL cite AV2 § 5.17.12, § 6.16.13,
§ 7.21.1, § 7.21.2, and § 7.21.7, and SHALL remain contract-only until source
computes hashes from decoded output with self-contained tests.

#### Scenario: Contract defines sample byte stream

- **WHEN** a reader checks the hash contract
- **THEN** it defines frame order as zero-based AV2 § 7.21 output order after
  supported stream/layer selection
- **AND** it defines the future hash input as cropped output samples, excluding
  allocation padding, stride bytes, and codec metadata fields
- **AND** it defines luma dimensions as `w x h` and chroma dimensions as
  `((w + subX) >> subX) x ((h + subY) >> subY)` using the values from the AV2
  output process
- **AND** it defines sample traversal as row-major raster order within each
  plane and plane order as Y, then U, then V for non-monochrome output

#### Scenario: Contract defines byte representation and algorithm

- **WHEN** a reader checks how samples become hash bytes
- **THEN** the contract states that 8-bit samples are encoded as one byte
- **AND** samples with bit depth greater than 8 are encoded as two bytes in
  little-endian order
- **AND** the initial repository-owned algorithm is
  `splot-dfh-sha256-v1`, a SHA-256 digest over the canonical AV2 sample-byte
  serialization
- **AND** AV2 `hash_type = 0` MD5 remains a separate future
  `METADATA_TYPE_DECODED_FRAME_HASH` interop verification path
- **AND** other AV2 `hash_type` values remain reserved by AOMedia and are not
  `splot` hash variants

#### Scenario: Contract defines grain and variant labels

- **WHEN** a reader checks the supported hash variant
- **THEN** the default future `splot` frame hash is the raw decoded output
  variant corresponding to AV2 `has_grain = 0`
- **AND** any future post-film-grain hash MUST be explicitly labeled as a
  separate variant before being treated as supported
- **AND** the contract states that film-grain-capable hashes require the
  § 7.21.7 film-grain synthesis process to be implemented and tested

#### Scenario: Contract remains non-executable until implementation

- **WHEN** `deterministic-frame-hash` is still contract-only
- **THEN** the decoder support matrix marks the row as `partial`
- **AND** the matrix row records self-contained docs/OpenSpec proof commands
- **AND** the row does not claim source emission, runtime hash computation, Y4M
  support, or mandatory AVM/dav2d execution
