## ADDED Requirements

### Requirement: Coefficient state context handoff support status

The decoder support model SHALL track
`DECODE-COEFF-STATE-CONTEXT-HANDOFF` as a distinct crate-private partial
decoder boundary named `coeff-state-context-handoff`. The row SHALL mark only
loaded-but-unwired ordinary non-FSC nonzero coefficient pass composition that
reads sign-source DC contexts from tile coefficient state before committing the
final context lines through that same state object, and SHALL keep runtime
nonzero coefficient decode, dequantization, reconstruction, output, reference
refresh, public APIs, and AVM/dav2d evidence unsupported until separately
implemented.

#### Scenario: Matrix records the state context handoff boundary

- **WHEN** `cargo xtask check-decoder-support` validates the decoder support
  matrix
- **THEN** `coeff-state-context-handoff` appears with Feature ID
  `DECODE-COEFF-STATE-CONTEXT-HANDOFF`
- **AND** it cites AV2 §5.20.7.27 for sign-source context use and the
  end-of-`coeffs()` level/DC context update
- **AND** it names focused tests for successful read-before-write behavior,
  ordinary-pass failure preserving context state, and invalid context-update
  facts preserving context state
- **AND** broad runtime `coeffs()`, reconstruction, output, and reference
  support remain partial or unsupported in their existing rows
