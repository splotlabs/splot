## ADDED Requirements

### Requirement: Minimal-tier runtime hash success

The decoder support model SHALL define a supported
`decode-minimal-tier-runtime-success` row tracked by Feature ID
`DECODE-MINIMAL-TIER-RUNTIME-SUCCESS` only when `splot decode` can verify the
documented minimal intra fixture trace and emit a hash success artifact. The
supported scope SHALL be limited to the `minimal-intra-8bit420-hash-v1`
fixture-trace tier over the committed IVF/DKIF-wrapped Annex B OBU payload and
SHALL cite § 5.2/§ 6.2 OBU front-door rules, minimal-tier frame/tile syntax
sections, § 8.2 symbol parsing,
§ 7.1 decode process, § 7.21.1-§ 7.21.2 output sample preparation, and
§ 5.17.12/§ 6.16.13 only for metadata-hash separation and sample-byte-order
context. The row SHALL NOT claim
full AV2 decoder conformance, Annex A level/tier conformance, Y4M/raw output,
film-grain output, metadata hash verification, full tile/CDF/intra support,
reference-refresh completeness, or AVM/dav2d integration.

#### Scenario: Minimal-tier hash JSON succeeds

- **WHEN** `splot decode --output-format hash --json` runs on the committed
  minimal-tier intra IVF fixture
- **THEN** the command exits with code 0
- **AND** stdout is a `splot.decode.hash_report` success artifact with
  `contract_version = 1`
- **AND** the report contains one or more frames sorted by `output_index`
- **AND** each hash entry names `raw_intermediate_output`,
  `splot-dfh-sha256-v1`, `av2-output-samples-v1`, and a 64-character lowercase
  hexadecimal digest
- **AND** stderr is empty

#### Scenario: Hash mode does not touch output paths

- **WHEN** hash output succeeds with no `-o` path
- **THEN** the command creates no implicit output file in the working directory
- **WHEN** hash output succeeds with `-o <path>` pointing to an existing file
- **THEN** the command leaves that file byte-for-byte unchanged
- **AND** no temporary or recovery file is left in the output directory

#### Scenario: Thread policies produce identical decoded frame hashes

- **WHEN** the same minimal-tier fixture is decoded with `--threads 1`,
  `--threads auto`, and a fixed positive `--threads N`
- **THEN** every run emits the same ordered `output_index` sequence
- **AND** every run emits the same visible dimensions, pixel format, bit depth,
  output variant, byte-stream identifier, and digest value for each frame
- **AND** any selected-thread-policy metadata difference does not change the
  decoded frame hash identity

#### Scenario: Malformed input remains diagnostic JSON

- **WHEN** malformed Annex B or IVF input is decoded with
  `--output-format hash --json`
- **THEN** the command exits nonzero
- **AND** stdout is a decoder diagnostic JSON object with
  `rule_id = "decode/malformed-source"`
- **AND** stdout is not a partial `splot.decode.hash_report`
- **AND** no output path is created or modified

#### Scenario: Outside-tier valid streams remain unsupported

- **WHEN** a valid AV2 stream, including a raw Annex B stream, is outside
  `minimal-intra-8bit420-hash-v1`
- **THEN** the command exits nonzero
- **AND** stdout or stderr reports `decode/unsupported-feature`
- **AND** the diagnostic names the blocking matrix row or feature metadata
- **AND** no hash success artifact is emitted

#### Scenario: Runtime resource limits fail before allocation or output

- **WHEN** bitstream-derived dimensions, tile payloads, decoded-frame bytes,
  output frame counts, output byte counts, or hash report sizes exceed
  `DecodeLimits` or checked arithmetic overflows
- **THEN** the command exits nonzero with `decode/resource-limit`
- **AND** the diagnostic includes the limit name, unit, and measured value when
  available
- **AND** no decoded-frame allocation, hash report construction, or output path
  publication occurs before the limit check

#### Scenario: Reference evidence remains portable metadata only

- **WHEN** local AVM or dav2d evidence is recorded for the minimal-tier fixture
- **THEN** it is committed only as portable metadata such as tool name,
  revision, sanitized command summary, fixture hash, output digest, date, and
  agreement notes
- **AND** repository code, tests, CI, xtask commands, scripts, wrappers,
  submodules, binaries, caches, and runtime execution do not locate, build,
  invoke, or require AVM or dav2d

#### Scenario: Status updates remain narrow

- **WHEN** the runtime hash path is implemented
- **THEN** `docs/DECODER-SUPPORT-MATRIX.toml`,
  `docs/DECODER-SUPPORT-STATUS.md`, `docs/IMPLEMENTATION-MATRIX.toml`, and
  generated feature/spec coverage docs mark only the proven minimal hash runtime
  scope as supported
- **AND** broad rows for full decode, tile payload decode, CDF lifecycle,
  intra/inter reconstruction, Y4M/raw output, film grain, reference update,
  layers, and decoder-model constraints remain partial or unsupported until
  their own source-backed implementation and tests land
