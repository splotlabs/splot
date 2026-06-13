## ADDED Requirements

### Requirement: Portable local-reference evidence manifest

The repository SHALL provide a versioned, portable local-reference evidence
manifest for future decoder fixtures and hash comparisons. The manifest SHALL be
tracked by Feature ID `XTASK-LOCAL-REFERENCE-EVIDENCE-MANIFEST`. Manifest
entries SHALL be non-executable metadata only: they may record reference tool
identity, upstream revisions, sanitized command summaries, committed fixture
identity, decoded-output digests, and comparison assertions, but they SHALL NOT
require AVM, dav2d, ffmpeg, a network connection, or any external decoder to be
installed or executed. Manifest metadata SHALL NOT claim current `splot decode`
runtime support, reconstruction support, deterministic hash computation, Y4M
output, AV2 decoder conformance, or AV2 bitstream conformance.

#### Scenario: Manifest validates without external tools

- **WHEN** the local-reference evidence manifest is checked
- **THEN** the checker parses and validates only committed metadata and fixture
  bytes
- **AND** it does not locate, build, spawn, or require AVM, dav2d, ffmpeg, the
  network, or `splot decode`

#### Scenario: Manifest records portable fixture identity

- **WHEN** a manifest entry references a committed fixture
- **THEN** the fixture path is repo-relative, normalized, and points to an
  existing committed regular file
- **AND** the manifest records the fixture byte length and lowercase SHA-256
  digest
- **AND** the checker verifies both values from the committed fixture bytes

#### Scenario: Manifest rejects local machine state

- **WHEN** any manifest field contains a local absolute path, `file://` URL,
  home-relative path, Windows absolute path, local environment path token,
  executable path, or shell command composition syntax
- **THEN** the checker rejects the manifest
- **AND** the rejected metadata is not treated as portable evidence

#### Scenario: Manifest cross-references repository tracking

- **WHEN** a manifest entry names a Feature ID or decoder-support row
- **THEN** the checker verifies that the Feature ID exists in
  `docs/IMPLEMENTATION-MATRIX.toml`
- **AND** every decoder-support row exists in
  `docs/DECODER-SUPPORT-MATRIX.toml`

#### Scenario: Manifest assertions are self-contained metadata

- **WHEN** a manifest entry records decoded-output digests or equality
  assertions from local reference tools
- **THEN** each digest field uses the declared algorithm and valid hex length
- **AND** each assertion references recorded digest IDs in the same evidence
  entry
- **AND** equality assertions compare the recorded metadata values only, without
  rerunning external tools
