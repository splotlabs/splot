## ADDED Requirements

### Requirement: IVF validation input

`splot-validate` SHALL validate both raw Annex B inputs and IVF-wrapped Annex B
inputs through the default byte-validation API.

#### Scenario: Valid IVF input validates like its payload

- **WHEN** `Validator::validate_bytes` receives an IVF file whose frames contain
  conformant Annex B OBUs
- **THEN** validation SHALL report no errors caused by the container
- **AND** SHALL run the existing OBU checks over the frame payload OBUs.

#### Scenario: Malformed IVF input is a report

- **WHEN** `Validator::validate_bytes` receives a malformed IVF file
- **THEN** validation SHALL emit a stable `ivf/*` diagnostic
- **AND** SHALL return a `ValidationReport` rather than panicking or returning a
  CLI-only error.

### Requirement: IVF diagnostic namespace

IVF diagnostics SHALL use the `ivf/` namespace, include severity, byte offset when
known, and a human-readable message.

#### Scenario: Truncated frame payload diagnostic

- **WHEN** an IVF frame declares more payload bytes than remain in the input
- **THEN** validation SHALL emit `ivf/truncated-frame-payload`
- **AND** the diagnostic SHALL point at the first missing byte offset.
