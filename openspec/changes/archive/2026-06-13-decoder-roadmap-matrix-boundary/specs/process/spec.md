## ADDED Requirements

### Requirement: generated decoder support document
The repository SHALL provide a generated document
`docs/DECODER-SUPPORT-STATUS.md`, rendered from
`docs/DECODER-SUPPORT-MATRIX.toml` by
`cargo xtask decoder-support --format markdown --output docs/DECODER-SUPPORT-STATUS.md`.
`cargo xtask check-decoder-support` SHALL fail when the committed document does
not match its render. `cargo xtask ci` SHALL run this check without invoking
AVM, dav2d, or any external decoder. Tracked by
`XTASK-DECODER-SUPPORT-STATUS`.

#### Scenario: looking up decoder support
- **WHEN** a reader opens `docs/DECODER-SUPPORT-STATUS.md`
- **THEN** the document shows decoder/reconstruction row status counts and the
  row-level support status rendered from `docs/DECODER-SUPPORT-MATRIX.toml`

#### Scenario: committed decoder status drifts
- **WHEN** `docs/DECODER-SUPPORT-MATRIX.toml` changes without regenerating
  `docs/DECODER-SUPPORT-STATUS.md`
- **THEN** `cargo xtask check-decoder-support` fails and names the regenerate
  command

#### Scenario: reference tools are absent
- **WHEN** `cargo xtask ci` runs on a machine without AVM or dav2d
- **THEN** the decoder support document check still runs from committed files
  only and does not locate, build, or execute either reference tool
