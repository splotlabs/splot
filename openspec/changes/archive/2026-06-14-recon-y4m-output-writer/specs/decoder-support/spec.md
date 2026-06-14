## ADDED Requirements

### Requirement: Reconstruction Y4M output writer

The decoder support model SHALL provide a source-backed Y4M writer in
`splot-recon` for caller-supplied `DecodedFrame<T>` values. The writer SHALL be
tracked by Feature ID `RECON-Y4M-OUTPUT-WRITER`, SHALL use AV2-derived decoded
output facts from § 6.4.1, § 6.17.4.4, § 7.21.1, and § 7.21.2, and SHALL treat
Y4M container syntax and chroma tags as repository-owned output policy outside
the AV2 specification. The writer SHALL NOT read AV2 bitstreams, select layers,
decode tile payloads, reconstruct pixels, apply film grain, schedule output
order, refresh references, invoke AVM/dav2d, or change `splot decode` CLI
runtime behavior.

#### Scenario: Stream header uses visible output format

- **WHEN** a caller builds a Y4M stream header from a decoded frame and a valid
  nonzero frame rate
- **THEN** the header uses the frame's visible luma width and height
- **AND** it uses progressive `Ip` output
- **AND** it derives the Y4M chroma tag from the frame's `BitDepth` and
  `PixelFormat`
- **AND** it does not use coded padding, reference-store dimensions, OBU bytes,
  IVF timestamps, output index, AV2 metadata, AVM, or dav2d to construct the
  header

#### Scenario: Invalid frame rate is rejected

- **WHEN** a caller supplies a zero frame-rate numerator or denominator
- **THEN** the Y4M API rejects the configuration with a typed error
- **AND** no Y4M stream header or frame bytes are written

#### Scenario: Frame payload uses visible rows

- **WHEN** a caller writes a decoded frame to Y4M
- **THEN** the frame payload serializes only cropped visible output samples
- **AND** storage stride, coded padding, reference-frame padding, and output
  metadata are excluded
- **AND** non-monochrome frames write Y bytes, then U bytes, then V bytes
- **AND** monochrome frames write only Y bytes

#### Scenario: Sample byte serialization is pinned

- **WHEN** a decoded frame has 8-bit output samples
- **THEN** each visible sample is written as one byte
- **WHEN** a decoded frame has 10-bit output samples
- **THEN** each visible sample is written as two little-endian bytes without
  normalization or scaling

#### Scenario: Stream rejects mismatched frames before payload output

- **WHEN** a caller tries to append a frame whose visible size, bit depth, or
  pixel format differs from the Y4M stream header
- **THEN** the writer returns a typed stream-parameter mismatch error
- **AND** it does not write `FRAME\n` or any frame payload bytes for the
  mismatched frame

#### Scenario: Writer errors are propagated

- **WHEN** the caller-provided output writer returns an I/O error while receiving
  the Y4M stream header, frame header, or frame payload
- **THEN** the Y4M writer returns that I/O error without panicking

#### Scenario: Runtime decode output remains unsupported

- **WHEN** a reader checks the decoder roadmap and support matrix after this
  writer is implemented
- **THEN** the `output-y4m` row states that source-backed Y4M writing exists only
  for caller-supplied decoded frames
- **AND** runtime `splot decode -o` Y4M output, byte-consuming decode,
  reconstruction algorithms, output scheduling, film-grain synthesis, AVM/dav2d
  invocation, and CI reference-tool requirements remain unsupported

## MODIFIED Requirements

### Requirement: Minimal decode tier contract

The decoder support docs SHALL define the first supported decode tier before any
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
  output samples are the first runtime success artifact
- **AND** runtime `splot decode` Y4M output remains unsupported until a
  byte-consuming decode/output row wires the `splot-recon` Y4M writer to real
  decoded frames and tests the CLI output path
- **AND** source-backed Y4M writing over caller-supplied `DecodedFrame<T>` values
  MAY be tracked separately by the `output-y4m` row without claiming runtime
  decode support
- **AND** streams outside the tier SHALL fail with structured
  `decode/unsupported-feature` diagnostics that identify the blocking matrix row
  where possible, while limit overflow or configured-limit excess SHALL use the
  emitted `decode/resource-limit` diagnostic when surfaced through `splot decode`

#### Scenario: Contract remains non-executable until implementation

- **WHEN** `minimal-decode-tier-contract` is still contract-only
- **THEN** the decoder support matrix marks the row as `partial`
- **AND** the row records self-contained docs/OpenSpec proof commands
- **AND** the row does not claim source implementation, runtime byte
  consumption, stream traversal, layer selection, reconstruction, runtime frame
  hashes, runtime Y4M output, fixture support, fuzz coverage, emitted new
  diagnostics, or mandatory AVM/dav2d execution
