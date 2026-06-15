## ADDED Requirements

### Requirement: Runtime Live Cursor Frontier Bridge
The tile partition traversal boundary SHALL provide a crate-private bridge that
can return both the existing traversal plan and the live §8.2 symbol decoder
cursor after the first `decode_block()` frontier. The bridge SHALL be usable by
the minimal runtime without adding a public `splot-core` checkpoint-resume API
or expanding the frontier beyond §5.20.3.1 partition traversal.

#### Scenario: Live cursor matches frontier checkpoint
- **WHEN** the runtime bridge reaches the root `decode_block()` frontier for the
  committed minimal tile payload
- **THEN** the returned traversal plan records the same symbol count and
  consumed-bit position as the live symbol decoder cursor
- **AND** the live cursor can continue decoding the existing traced flat-block
  symbols without replaying the root partition symbol manually

#### Scenario: Bridge remains narrower than decode block
- **WHEN** the root partition frontier is planned for the minimal runtime
- **THEN** the bridge asserts the frontier before §5.20.4.1 `decode_block()`
- **AND** it does not mutate `MiSizes`, parse block syntax, reconstruct pixels,
  update references, emit output, or perform CDF copyback/averaging
