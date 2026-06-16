## ADDED Requirements

### Requirement: Minimal-tier runtime raw output
The decoder support model SHALL provide Feature ID `DECODE-MINIMAL-RAW-RUNTIME-OUTPUT`
as a narrow `splot decode` raw output success path for the existing
`minimal-intra-8bit420-hash-v1` tier. The raw path MUST use the same
byte-consuming validation, tile trace, output sample values, visible geometry,
bit depth, and pixel format already required for
`DECODE-MINIMAL-TIER-RUNTIME-SUCCESS`.

#### Scenario: Explicit raw output succeeds for the minimal fixture
- **WHEN** `splot decode --output-format raw <minimal-ivf-fixture> -o <output.raw>` is run for the committed minimal 64x64 IVF fixture
- **THEN** the command exits successfully
- **AND** stdout and stderr are empty
- **AND** `<output.raw>` contains exactly one headerless `raw_intermediate_output` event encoded as `av2-output-samples-v1`
- **AND** the bytes are visible Y samples, then visible U samples, then visible V samples, with the same flat sample values used by the minimal hash runtime

#### Scenario: Raw output does not require an IVF timebase
- **WHEN** `splot decode --output-format raw <minimal-ivf-fixture> -o <output.raw>` is run for the committed minimal fixture shape with a zero IVF timebase numerator or denominator
- **THEN** the command exits successfully
- **AND** `<output.raw>` contains the same raw sample bytes as the nonzero-timebase fixture

#### Scenario: Out-of-tier raw inputs fail closed
- **WHEN** `splot decode --output-format raw <input> -o <output.raw>` is run for a malformed, resource-limited, or out-of-tier source
- **THEN** the command emits the existing structured `decode/malformed-source`, `decode/resource-limit`, or `decode/unsupported-feature` diagnostic as appropriate
- **AND** it does not create, truncate, or replace `<output.raw>`

### Requirement: Atomic runtime raw publication
The CLI SHALL publish runtime raw output atomically: all raw sample bytes MUST
be decoded and serialized before opening output paths, then written to a
same-directory temporary file, and the requested output path MUST be replaced
only after successful decode, serialization, temp-file write, flush, and file
sync.

#### Scenario: Existing raw output is replaced only after success
- **WHEN** the requested raw output path already contains bytes
- **AND** the minimal runtime raw decode succeeds
- **THEN** the requested path is replaced by the complete raw sample byte stream
- **AND** no temporary output file remains in the output directory

#### Scenario: Raw failure preserves existing output
- **WHEN** the requested raw output path already contains bytes
- **AND** decode, serialization, write, flush, sync, rename, or cleanup fails before publication
- **THEN** the requested path remains byte-for-byte unchanged
- **AND** no partial raw stream is visible at the requested path

#### Scenario: Raw source diagnostics win before output publication
- **WHEN** `splot decode --output-format raw <input> -o <output.raw>` is run for a malformed or out-of-tier source whose output parent cannot be created
- **THEN** the command emits the source diagnostic rather than `decode/output-error`
- **AND** it does not create the missing output parent or requested output path

### Requirement: Runtime raw byte accounting
The runtime raw output path SHALL check `DecodeLimitName::MaxOutputBytes`
against the complete raw visible sample byte stream length before publishing the
file.

#### Scenario: Raw output byte limit rejects before publication
- **WHEN** the configured `max_output_bytes` is smaller than the complete minimal raw sample byte stream length
- **THEN** runtime raw output fails with `decode/resource-limit`
- **AND** the requested output path is not created, truncated, or replaced

#### Scenario: Raw output is deterministic across thread policies
- **WHEN** the same minimal fixture is decoded to raw with `--threads 1`, `--threads auto`, and a fixed positive thread count
- **THEN** each successful command writes byte-identical raw output

### Requirement: Decode runtime raw fuzz entry point
For Feature ID `CONF-DECODE-RUNTIME-RAW-FUZZ`, the fuzz corpus SHALL include a
self-contained `decode_runtime_raw_bytes` target that feeds arbitrary bytes and
bounded mutations of the committed minimal IVF fixture through the runtime raw
byte API without filesystem I/O or external decoder invocation.

#### Scenario: Raw runtime fuzz target is registered
- **WHEN** `cargo xtask check-fuzz-targets` runs
- **THEN** `fuzz/fuzz_targets/decode_runtime_raw_bytes.rs` has a matching `[[bin]]` entry in `fuzz/Cargo.toml`

#### Scenario: Raw runtime fuzz accepts typed outcomes
- **WHEN** `decode_runtime_raw_bytes` runs on arbitrary input
- **THEN** successful cases satisfy only the stable minimal raw output shape
- **AND** malformed, unsupported, resource-limit, and writer-failure paths return typed `DecodeError` values rather than panicking
