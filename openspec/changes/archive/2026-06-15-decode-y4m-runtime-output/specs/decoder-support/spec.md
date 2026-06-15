## ADDED Requirements

### Requirement: Minimal-tier runtime Y4M output
For Feature ID `DECODE-Y4M-RUNTIME-OUTPUT`, the decoder support model SHALL provide a narrow `splot decode` Y4M success path for the existing `minimal-intra-8bit420-hash-v1` tier, using the same byte-consuming validation, tile trace, output sample values, visible geometry, bit depth, and pixel format already required for `DECODE-MINIMAL-TIER-RUNTIME-SUCCESS`.

#### Scenario: Explicit Y4M output succeeds for the minimal fixture
- **WHEN** `splot decode --output-format y4m <minimal-ivf-fixture> -o <output.y4m>` is run for the committed minimal 64x64 IVF fixture
- **THEN** the command exits successfully
- **AND** stdout and stderr are empty
- **AND** `<output.y4m>` contains a complete Y4M stream for one 64x64 8-bit 4:2:0 raw-intermediate-output frame
- **AND** the frame payload contains the same flat sample values used by the minimal hash runtime

#### Scenario: Implicit Y4M output remains the compatibility form
- **WHEN** `splot decode <minimal-ivf-fixture> -o <output.y4m>` is run without `--output-format`
- **THEN** the command selects Y4M output
- **AND** it writes the same bytes as explicit `--output-format y4m`

#### Scenario: Out-of-tier Y4M inputs fail closed
- **WHEN** `splot decode --output-format y4m <input> -o <output.y4m>` is run for a malformed, resource-limited, or out-of-tier source
- **THEN** the command emits the existing structured `decode/malformed-source`, `decode/resource-limit`, or `decode/unsupported-feature` diagnostic as appropriate
- **AND** it does not create, truncate, or replace `<output.y4m>`

### Requirement: Atomic runtime Y4M publication
The CLI SHALL publish runtime Y4M output atomically: all Y4M bytes MUST be serialized before opening output paths, then written to a same-directory temporary file, and the requested output path MUST be replaced only after successful decode, serialization, temp-file write, flush, and file sync.

#### Scenario: Existing output is replaced only after success
- **WHEN** the requested Y4M output path already contains bytes
- **AND** the minimal runtime Y4M decode succeeds
- **THEN** the requested path is replaced by the complete Y4M stream
- **AND** no temporary output file remains in the output directory

#### Scenario: Failure preserves existing output
- **WHEN** the requested Y4M output path already contains bytes
- **AND** decode, serialization, flush, sync, rename, or cleanup fails before publication
- **THEN** the requested path remains byte-for-byte unchanged
- **AND** no partial Y4M stream is visible at the requested path

#### Scenario: Source diagnostics win before output publication
- **WHEN** `splot decode --output-format y4m <input> -o <output.y4m>` is run for a malformed or out-of-tier source whose output parent cannot be created
- **THEN** the command emits the source diagnostic rather than `decode/output-error`
- **AND** it does not create the missing output parent or requested output path

#### Scenario: Hash output remains no-touch
- **WHEN** `splot decode --output-format hash <input> -o <path>` is run
- **THEN** hash mode does not create, truncate, or replace `<path>`
- **AND** this remains true for both hash success and hash diagnostic paths

### Requirement: Decode output error diagnostics
The decoder support model SHALL expose `decode/output-error` for Y4M serialization and CLI publication failures that are not malformed-source, resource-limit, or unsupported-feature conditions.

#### Scenario: Output path cannot be published
- **WHEN** runtime Y4M decode reaches output publication but the output path cannot be created, written, flushed, synced, renamed, or cleaned up
- **THEN** `splot decode` emits a structured `decode/output-error` diagnostic
- **AND** the diagnostic includes a stable operation identifier
- **AND** it does not include nondeterministic temporary filename suffixes

#### Scenario: Output error is separate from AV2 conformance
- **WHEN** the failure is a filesystem or writer publication failure
- **THEN** the diagnostic is not reported as AV2 malformed source or unsupported feature
- **AND** any spec section field is omitted unless the failure is tied to AV2 output-sample semantics rather than filesystem publication

### Requirement: Runtime Y4M byte accounting
The runtime Y4M output path SHALL check `DecodeLimitName::MaxOutputBytes` against the complete Y4M stream length, including the Y4M stream header, per-frame header, and visible sample payload bytes, before publishing the file.

#### Scenario: Output byte limit rejects before publication
- **WHEN** the configured `max_output_bytes` is smaller than the complete minimal Y4M stream length
- **THEN** runtime Y4M output fails with `decode/resource-limit`
- **AND** the requested output path is not created, truncated, or replaced

#### Scenario: Output is deterministic across thread policies
- **WHEN** the same minimal fixture is decoded to Y4M with `--threads 1`, `--threads auto`, and a fixed positive thread count
- **THEN** each successful command writes byte-identical Y4M output
