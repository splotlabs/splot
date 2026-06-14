## MODIFIED Requirements

### Requirement: Structured decode unsupported diagnostics
Unsupported decoder features SHALL be represented in docs and matrix rows as
structured diagnostics with a stable rule id, severity, optional spec section,
matrix row id, human-readable message, and remediation. `splot-decode` SHALL
own the `decode/unsupported-feature` descriptor with severity `Error`. When
`splot decode` reaches the runtime decode/output boundary after successful byte
planning, the descriptor SHALL cite AV2 §7.1 with matrix row
`cli-decode-entrypoint` and Feature ID `CLI-DECODE`. When the byte or stream
planner rejects a parsed but unsupported structure, the diagnostic SHALL reuse
`decode/unsupported-feature` with the planner-owned matrix row, Feature ID,
spec section, reason, OBU type, and byte offset. The `splot decode` CLI entry
point SHALL render library-owned diagnostic reports without changing their base
text or JSON field values.

#### Scenario: Unsupported feature is documented
- **WHEN** a matrix row identifies an unsupported AV2 tool
- **THEN** the row links the unsupported behavior to a stable diagnostic code or
  planned diagnostic code and a spec section where applicable

#### Scenario: Decode crate owns the runtime unsupported diagnostic descriptor
- **WHEN** `splot-decode` is tested
- **THEN** it exposes the runtime `decode/unsupported-feature` descriptor with
  severity `Error`, spec section `7.1`, matrix row `cli-decode-entrypoint`, and
  Feature ID `CLI-DECODE`

#### Scenario: Decode command emits runtime unsupported text diagnostic
- **WHEN** `splot decode <input> -o <output>` is run on bytes that can be
  planned but runtime decode support is not implemented
- **THEN** it reads the input bytes, plans them through `DecodeContext::plan_bytes`,
  and exits with code `1`
- **AND** stderr contains diagnostic rule id `decode/unsupported-feature`,
  severity `Error`, spec section `7.1`, matrix row `cli-decode-entrypoint`,
  Feature ID `CLI-DECODE`, and a plan summary
- **AND** no AVM, dav2d, ffmpeg, or external decoder is located or invoked
- **AND** the requested output path is not created, truncated, or written

#### Scenario: Decode command emits runtime unsupported JSON diagnostic
- **WHEN** `splot decode --json <input> -o <output>` is run on bytes that can be
  planned but runtime decode support is not implemented
- **THEN** it exits with code `1`
- **AND** stdout is a machine-readable diagnostic object containing
  `rule_id = "decode/unsupported-feature"`, `severity = "Error"`,
  `spec_section = "7.1"`, `matrix_row = "cli-decode-entrypoint"`, and
  `feature_id = "CLI-DECODE"`
- **AND** stdout includes a runtime-unsupported detail block with input length,
  detected bitstream format, OBU count, frame-candidate count, and selected
  output format
- **AND** stderr remains empty unless an operational error occurs

#### Scenario: Decode command emits planner unsupported diagnostic
- **WHEN** `splot decode <input> -o <output>` reads bytes whose source parses but
  contains a structure outside the initial planner tier
- **THEN** it exits with code `1`
- **AND** it emits `decode/unsupported-feature` using the planner matrix row,
  Feature ID, AV2 spec section, unsupported reason, OBU type, and byte offset
- **AND** the requested output path is not created, truncated, or written

#### Scenario: Decode command reports missing input as operational error
- **WHEN** `splot decode <missing-input> -o <output>` is run
- **THEN** it exits with code `2`
- **AND** it does not emit a `decode/*` diagnostic
- **AND** it does not create the missing input path or output path

### Requirement: Decode resource-limit diagnostic contract
The repository SHALL document and emit `decode/resource-limit` when a
byte-consuming `splot decode` path rejects an input because a measured
spec-derived or repository-owned decode-planner value exceeds a configured
`DecodeLimits` threshold. The diagnostic SHALL include the stable decoder
diagnostic fields `rule_id`, `severity`, `spec_section`, `matrix_row`,
`feature_id`, `message`, and `remediation`, plus resource fields `limit_name`,
`limit`, `actual`, `unit`, `byte_offset`, and `bit_offset`. Resource limits are
`splot` policy over measured values and SHALL NOT be described as AV2
conformance failures.

#### Scenario: Limit violation reports measured value
- **WHEN** `splot decode` rejects an input because byte planning exceeds a
  `DecodeLimits` threshold
- **THEN** it emits `decode/resource-limit` with severity `Error`, matrix row
  `decode-limits-budget`, Feature ID `DOC-DECODE-LIMITS-CONTRACT`, the relevant
  AV2 or policy section, the limit name, configured limit, measured actual
  value, unit, and nullable byte/bit offsets
- **AND** the requested output path is not created, truncated, or written

#### Scenario: Resource-limit diagnostic is in emitted registry
- **WHEN** `cargo xtask check-diagnostic-registry` runs after source emits
  `decode/resource-limit`
- **THEN** `decode/resource-limit` appears inside the emitted decoder diagnostic
  registry marker region
- **AND** the decoder support matrix links the emitted diagnostic to a support row

### Requirement: Decode byte-planner CLI handoff
The `splot decode` CLI SHALL read input bytes, construct a `DecodeContext` from
the requested thread policy, and call `DecodeContext::plan_bytes` with finite
default `DecodeOptions` before reporting decoder-stage diagnostics. The CLI
SHALL NOT duplicate byte parsing or stream planning logic, call
`WorkerPool::install` directly, spawn threads, use global pools, or add
concurrency dependencies.

#### Scenario: Malformed source emits structured diagnostic
- **WHEN** `splot decode` reads malformed raw Annex B bytes, a malformed IVF
  container, or malformed Annex B inside IVF
- **THEN** it exits with code `1`
- **AND** it emits `decode/malformed-source` with severity `Error`, matrix row
  `decode-byte-stream-planner`, Feature ID `DECODE-BYTE-STREAM-PLANNER`,
  source issue kind, parser rule ID when known, byte offset when known, IVF
  frame index when known, and parser message
- **AND** the requested output path is not created, truncated, or written

#### Scenario: Thread policy uses decode context
- **WHEN** `splot decode --threads auto`, `--threads 1`, or another fixed
  positive thread count is run on the same diagnostic-producing input
- **THEN** each invocation reaches the same `DecodeContext::plan_bytes`
  diagnostic result
- **AND** the CLI code does not introduce direct Rayon, crossbeam, global-pool,
  queue, or ad-hoc thread usage outside `splot_parallel`

#### Scenario: Prior byte planner review feedback stays protected
- **WHEN** tests exercise byte planning after the CLI handoff
- **THEN** unsupported structures keep precedence over later traversal limits,
  fatal IVF first-frame header errors remain retry-stable, `decode_plan_bytes`
  fuzz seeds include prefixed valid fixture paths, and `DecodeContext` docs
  accurately describe raw-byte planning
