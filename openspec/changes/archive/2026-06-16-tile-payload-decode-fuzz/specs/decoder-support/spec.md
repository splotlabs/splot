## ADDED Requirements

### Requirement: Tile payload fuzz support evidence

The decoder support matrix SHALL record `CONF-TILE-PAYLOAD-DECODE-FUZZ` as
self-contained fuzz evidence for the current minimal tile-payload runtime byte
frontier. The evidence SHALL reference the cargo-fuzz target, the
`splot-decode` `fuzzing` harness used by the target, focused
tile-payload/runtime tests, and the required
fuzz/check commands.

#### Scenario: Decoder support records the fuzz target

- **WHEN** `cargo xtask check-decoder-support` validates the decoder support
  matrix
- **THEN** a support row for `tile-payload-decode-fuzz` links Feature ID
  `CONF-TILE-PAYLOAD-DECODE-FUZZ` to `fuzz/fuzz_targets/tile_payload_decode_bytes.rs`
  and the feature-gated fuzzing harness

#### Scenario: Broad tile decode remains partial

- **WHEN** decoder support status is regenerated after adding the fuzz target
- **THEN** `tile-payload-decode` remains `partial` and its notes continue to
  exclude full `decode_tile()`, broad §8.3 CDF selection, recursive
  partition/block syntax, reconstruction expansion, reference refresh, and
  external decoder integration
