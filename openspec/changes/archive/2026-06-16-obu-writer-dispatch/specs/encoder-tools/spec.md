# encoder-tools delta: obu-writer-dispatch

## ADDED Requirements

### Requirement: complete-OBU writer dispatch

`splot-core` SHALL provide a writer that turns a parsed OBU (`ObuHeader` + `ParsedObu`) back into
bytes — the inverse of `dispatch_obu_payload` / `finish_obu_payload`. It SHALL write the typed payload
body via the existing per-structure writers and then the OBU tail (for a non-empty payload of an
extensible OBU type, `obu_extension_flag = 0` then `trailing_bits()`; nothing for an empty payload),
and SHALL prepend the OBU header in the complete-OBU form. For every parsed OBU of a *written* type
(temporal delimiter, sequence header, padding, metadata short, metadata group), reparsing the written
bytes SHALL yield the original `ParsedObu`. The length-summarized / opaque payloads (padding, the
metadata blobs) SHALL be supplied via a passthrough input.

For an OBU type that has no body writer yet, the dispatch SHALL return a typed
`WriteError::Unimplemented` (an honest stub) rather than panic or emit wrong bytes. The writer SHALL
be additive (no model or parser-error change beyond the new `Unimplemented` variant), SHALL be
reject-before-write (a delegated sub-writer reject or a passthrough mismatch leaves the writer
untouched), and SHALL never panic.

#### Scenario: a parsed OBU of a written type round-trips

- **WHEN** a parsed OBU of a written type (with its passthrough bytes) is written via the dispatch and
  the bytes are reparsed
- **THEN** the reparsed `ParsedObu` SHALL equal the original, and the bytes SHALL be byte-exact on the
  canonical subset.

#### Scenario: an unwritten OBU type yields a typed Unimplemented

- **WHEN** the dispatch is asked to write a `ParsedObu` variant that has no body writer yet
- **THEN** it SHALL return `WriteError::Unimplemented` and write no bit.
