## ADDED Requirements

### Requirement: Minimal decode tier contract

The repository SHALL document the first intended `splot decode` success tier as
a repository-owned implementation subset before any runtime decode path, hash
output, Y4M output, or encoder roundtrip expectation is marked supported. The
contract SHALL use Feature ID `DOC-MINIMAL-DECODE-TIER-CONTRACT`, contract ID
`splot.decode.minimal_tier`, contract version `1`, and tier ID
`minimal-intra-8bit420-hash-v1`. The contract SHALL remain docs-only until
source implements the tier and self-contained tests prove it.

#### Scenario: Contract avoids Annex A conformance overclaim

- **WHEN** a reader checks the minimal decode tier contract
- **THEN** it states that the tier is a `splot` implementation-supported subset
  and not an Annex A level-conformant decoder claim
- **AND** it keeps current `splot decode` behavior as intentionally unsupported
  until runtime support lands

#### Scenario: Contract defines accepted input and layer shape

- **WHEN** a reader checks the minimal decode tier input boundary
- **THEN** it admits only Annex B length-delimited OBU input, including
  IVF/DKIF streams whose frame payloads are Annex B
- **AND** it requires one selected stream/layer with non-global
  `obu_xlayer_id == 0`, `obu_tlayer_id == 0`, inferred `obu_mlayer_id == 0`,
  and no temporal or embedded enhancement layer
- **AND** it excludes bare OBU streams, Y4M input, multistream composition,
  external HLS, MSDO, LCR, Atlas, OPS selection, sub-bitstream extraction, and
  any external decoder wrapper

#### Scenario: Contract defines sequence format and limits

- **WHEN** a reader checks the minimal decode tier sequence boundary
- **THEN** it requires `seq_profile_idc == 0` (`Main_420_10_IP0`) input
  further narrowed to `chroma_format_idc == 0`, `bit_depth_idc == 1`,
  `max_tlayer_id == 0`, `max_mlayer_id == 0`, `SeqMaxMlayerCnt == 1`, and
  `film_grain_params_present == 0`
- **AND** it requires frame dimensions, tile counts, decoded-frame bytes,
  reference-store bytes, hash bytes, and output bytes to pass `DecodeLimits`
  using checked arithmetic before allocation or output

#### Scenario: Contract defines accepted frame and tile shape

- **WHEN** a reader checks the minimal decode tier frame boundary
- **THEN** it accepts only closed-loop key-frame output whose parsed facts prove
  `obu_type == OBU_CLOSED_LOOP_KEY`, `FrameType = KEY_FRAME`, and
  `FrameIsIntra = 1`
- **AND** it requires inline frame headers with `cur_mfh_id == 0`,
  `frame_size_override_flag == 0`, `immediate_output_frame == 1`,
  `implicit_output_frame == 0`, and no sequence cropping window
- **AND** it requires a single tile with one first-and-only tile group
- **AND** it excludes open-loop key frames, RAS, switch, SEF/show-existing, TIP,
  bridge, inter frames, `INTRA_ONLY_FRAME`, multi-frame headers, multiple tiles,
  multiple tile groups, film grain application, quantizer-matrix-dependent
  decode, decoder-model scheduling, and unsupported tools without supported
  matrix rows and tests

#### Scenario: Contract defines success and rejection artifacts

- **WHEN** a future implementation proves a stream is inside the minimal tier
- **THEN** deterministic `splot-dfh-sha256-v1` frame hashes over cropped visible
  output samples are the first success artifact
- **AND** Y4M output remains unsupported until the `output-y4m` row is
  implemented and tested against the same cropped visible output samples
- **AND** streams outside the tier SHALL fail with structured
  `decode/unsupported-feature` diagnostics that identify the blocking matrix row
  where possible, while limit overflow or configured-limit excess SHALL use the
  planned `decode/resource-limit` diagnostic

#### Scenario: Contract remains non-executable until implementation

- **WHEN** `minimal-decode-tier-contract` is still contract-only
- **THEN** the decoder support matrix marks the row as `partial`
- **AND** the row records self-contained docs/OpenSpec proof commands
- **AND** the row does not claim source implementation, runtime byte
  consumption, stream traversal, layer selection, reconstruction, frame hashes,
  Y4M output, fixture support, fuzz coverage, emitted new diagnostics, or
  mandatory AVM/dav2d execution

## MODIFIED Requirements

### Requirement: Deterministic decoded-frame hash contract

The repository SHALL document a deterministic decoded-frame hash contract before
any decoder output row, Y4M output row, fixture manifest, or encoder roundtrip
expectation is marked supported. The contract SHALL be tracked by
`DOC-DETERMINISTIC-FRAME-HASH-CONTRACT`, SHALL cite AV2 § 5.17.12, § 6.16.13,
§ 7.21.1, § 7.21.2, and § 7.21.7, and SHALL remain contract-only until source
computes hashes from decoded output with self-contained tests.

#### Scenario: Contract defines sample byte stream

- **WHEN** a reader checks the hash contract
- **THEN** it defines frame order as the repository-owned zero-based emission
  index over frames emitted by the AV2 output processes after supported
  stream/layer selection
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
