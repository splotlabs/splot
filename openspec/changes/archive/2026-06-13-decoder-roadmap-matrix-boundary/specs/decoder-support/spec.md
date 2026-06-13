## ADDED Requirements

### Requirement: Decoder roadmap
The repository SHALL document the decoder scope in `docs/DECODER-ROADMAP.md`.
The roadmap SHALL state that decoder work exists to support future encoder
roundtrips and reconstruction correctness, not production playback. The roadmap
SHALL define the staged `splot decode` path, the first supported tier before it
is implemented, deterministic frame-hash expectations, unsupported-feature
handling, and the local-only AVM/dav2d evidence boundary.

#### Scenario: Reader checks decoder scope
- **WHEN** a reader opens `docs/DECODER-ROADMAP.md`
- **THEN** the document says whether `splot decode` currently reconstructs pixels,
  what the first supported tier is, and which broad AV2 tools remain unsupported

#### Scenario: Local reference boundary is visible
- **WHEN** a reader checks how AVM or dav2d may be used during decoder work
- **THEN** the roadmap states that they are local development evidence only and
  SHALL NOT be invoked by repo code, build scripts, tests, `xtask`, or CI

### Requirement: Decoder support matrix
The repository SHALL provide `docs/DECODER-SUPPORT-MATRIX.toml` as the canonical
decoder/reconstruction support status file. Each row SHALL include a stable row
id, a linked Feature ID where available, spec sections, parser source,
decode/reconstruction module, supported tier, status, self-contained tests,
diagnostics, local reference evidence, and notes. Row status SHALL be one of
`todo`, `partial`, `supported`, `unsupported-intentional`, or `blocked`.

#### Scenario: Matrix row records unsupported behavior
- **WHEN** a decoder area is intentionally unsupported
- **THEN** its matrix row records `status = "unsupported-intentional"` or
  `status = "todo"` with the relevant spec section and the diagnostic or
  planned diagnostic that will explain the unsupported feature

#### Scenario: Supported row has proof
- **WHEN** a matrix row has `status = "supported"`
- **THEN** the row records at least one self-contained test or fixture that does
  not require AVM or dav2d at test time

### Requirement: Generated decoder support status
The repository SHALL generate a committed decoder support status document from
`docs/DECODER-SUPPORT-MATRIX.toml`. The generated document SHALL summarize row
counts by status and tier, list each row with its spec sections and tests, and
name any local reference evidence as portable metadata only.

#### Scenario: Matrix is rendered
- **WHEN** `cargo xtask decoder-support --format markdown --output docs/DECODER-SUPPORT-STATUS.md` runs
- **THEN** the command writes a deterministic Markdown render of
  `docs/DECODER-SUPPORT-MATRIX.toml`

#### Scenario: Generated document drifts
- **WHEN** `docs/DECODER-SUPPORT-MATRIX.toml` changes without regenerating
  `docs/DECODER-SUPPORT-STATUS.md`
- **THEN** `cargo xtask check-decoder-support` fails and names the regeneration
  command

### Requirement: Structured decode unsupported diagnostics
Unsupported decoder features SHALL be represented in docs and matrix rows as
structured diagnostics with a stable code, severity, optional spec section,
matrix row id, human-readable message, and remediation. Until `splot decode`
is backed by a library decoder, the roadmap SHALL describe this as the required
contract for later implementation rather than claiming it already exists.

#### Scenario: Unsupported feature is documented
- **WHEN** a matrix row identifies an unsupported AV2 tool
- **THEN** the row links the unsupported behavior to a stable diagnostic code or
  planned diagnostic code and a spec section where applicable

### Requirement: Local reference evidence remains non-executable
The repository SHALL treat local AVM/dav2d evidence as non-executable metadata
only. Evidence may be recorded as commit hashes, command summaries, decoded
hashes, and comparison notes in documentation, PR descriptions, agent-log files,
or portable fixture manifests. The repository SHALL NOT add code paths, scripts,
wrappers, build probes, dependencies, tests, CI jobs, or `xtask` commands that
locate, build, invoke, or require AVM or dav2d.

#### Scenario: CI runs decoder support checks
- **WHEN** `cargo xtask ci` runs on a machine without AVM or dav2d installed
- **THEN** decoder support status checks pass or fail solely from committed
  repository files

#### Scenario: Local evidence is recorded
- **WHEN** a future decoder fixture records AVM or dav2d evidence
- **THEN** the committed evidence is portable metadata and does not contain local
  absolute paths or require the reference tools to be installed
