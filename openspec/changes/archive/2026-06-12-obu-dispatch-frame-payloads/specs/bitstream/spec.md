# bitstream delta: obu-dispatch-frame-payloads

Closes the `AV2-5.2.1-OBU-DISPATCH` frame-carrying catch-all residual.

## ADDED Requirements

### Requirement: frame-carrying payload dispatch

`dispatch_obu_payload` SHALL parse the state-free prefix of every
frame-carrying OBU type and report an honest state-dependent status for
the remainder instead of a blanket unimplemented result; the stateless
dispatch surface and the stateful inspector surface SHALL be consistent
and documented.

#### Scenario: frame OBU dispatches its prefix

- **WHEN** a frame-carrying OBU is dispatched without cross-OBU state
- **THEN** its state-free prefix is parsed and the status names the
  state-dependent remainder

#### Scenario: truncation in the prefix surfaces

- **WHEN** the payload ends inside the state-free prefix
- **THEN** the parser reports the truncation without panicking
