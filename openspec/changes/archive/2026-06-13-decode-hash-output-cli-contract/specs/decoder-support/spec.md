## ADDED Requirements

### Requirement: Decode hash output CLI contract

The `splot decode` CLI SHALL provide a hash-output selection contract before any
runtime decode path is marked supported. The contract SHALL be tracked by
Feature ID `CLI-DECODE-HASH-OUTPUT`. The CLI SHALL preserve
`splot decode <input> -o <output>` as the compatibility form for future Y4M
output, SHALL allow explicit `--output-format y4m`, and SHALL allow
`--output-format hash` without a Y4M output path. Until runtime decode support
lands, every valid `splot decode` invocation SHALL continue to emit the existing
`decode/unsupported-feature` diagnostic, exit with code `1`, avoid input reads,
avoid output writes, and avoid external decoder invocation.

#### Scenario: Compatibility Y4M form remains valid but unsupported

- **WHEN** `splot decode <input> -o <output>` is run before runtime decode
  support is implemented
- **THEN** it remains a valid CLI invocation
- **AND** it exits with code `1` and emits `decode/unsupported-feature`
- **AND** it does not read `<input>` or modify `<output>`

#### Scenario: Explicit hash format is accepted without Y4M output

- **WHEN** `splot decode <input> --output-format hash` is run before runtime
  decode support is implemented
- **THEN** it is a valid CLI invocation
- **AND** it exits with code `1` and emits `decode/unsupported-feature`
- **AND** it does not read `<input>` or create any output file

#### Scenario: Explicit Y4M format still requires an output path

- **WHEN** `splot decode <input> --output-format y4m` is run without
  `-o/--output`
- **THEN** clap rejects the invocation as a usage error
- **AND** no `decode/unsupported-feature` runtime diagnostic is emitted

#### Scenario: Missing output selection remains a usage error

- **WHEN** `splot decode <input>` is run without `-o/--output` and without
  `--output-format`
- **THEN** clap rejects the invocation as a usage error
- **AND** no `decode/unsupported-feature` runtime diagnostic is emitted

#### Scenario: JSON mode remains diagnostic-only while unsupported

- **WHEN** `splot decode --json <input> --output-format hash` is run before
  runtime decode support is implemented
- **THEN** stdout contains the existing machine-readable
  `decode/unsupported-feature` diagnostic object
- **AND** stderr remains empty unless an operational error occurs
- **AND** no hash report schema or decoded-frame hash support is claimed

#### Scenario: Hash format refers to repository-owned decoded output hashes

- **WHEN** a reader checks the hash-output CLI contract
- **THEN** it identifies future hash output as `splot-dfh-sha256-v1` over
  decoded AV2 output samples in repository-owned emission-index order
- **AND** it does not describe AV2 `METADATA_TYPE_DECODED_FRAME_HASH`,
  `hash_type = 0` MD5, reserved AV2 hash types, OBU bytes, metadata payloads,
  parser facts, Y4M output, or AVM/dav2d execution as current `splot decode`
  output support
