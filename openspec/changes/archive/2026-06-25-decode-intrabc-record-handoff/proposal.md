## Why

The local decoder mission probe now reaches the selectable transform-record walk and
stops when §5.20.5.3 signals `use_intrabc = 1`. This is the next concrete
stream frontier after the FSC transform-record handoff: the runtime must consume
the IntrABC mode-info syntax in spec order before the transform-record and
residual handoff can continue.

## What Changes

- Extend the local decoder mission selectable-transform record path for
  `DECODE-SELECTABLE-TRANSFORM-RECORDS` to recognize the observed
  `use_intrabc = 1` branch in the luma/shared intra mode-info prelude.
- Consume the bounded §5.20.5.4 `read_intrabc_info()` symbols needed to preserve
  arithmetic stream alignment and retain IntrABC metadata for the transform
  handoff.
- Keep the runtime fail-closed before decoded sample population, motion
  compensation from the current frame, reconstruction, loop restoration, output,
  or byte-equality claims.
- Update the implementation matrix, decoder support matrix/status, and
  OpenSpec specs/tasks with local probe evidence for the next structured
  local decoder mission frontier.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `selectable-transform-records`: add a bounded IntrABC mode-info record
  handoff for the local decoder mission Wiener NS LR selectable transform-record path.
- `decoder-support`: update support-row requirements and proof expectations for
  the local decoder mission selectable-transform frontier after IntrABC syntax consumption.

## Impact

- Affected code is expected to stay within `splot-decode` runtime/tile-payload
  internals, primarily
  `crates/splot-decode/src/runtime_minimal/wienerns_lr/tx_records.rs` and any
  narrow helper module needed for IntrABC mode-info metadata.
- Affected tracking files are `docs/IMPLEMENTATION-MATRIX.toml`,
  `docs/DECODER-SUPPORT-MATRIX.toml`, generated decoder status docs, and this
  OpenSpec change.
- No public API, CLI option, dependency graph, encoder, or external reference
  tool invocation changes are intended.
- Non-goals: broad IntrABC reconstruction support, current-frame block-copy
  prediction, decoded `CurrFrame`/`CdefFrame` samples, inverse transforms,
  loop-restoration filtering, reference refresh, AVM/dav2d byte equality, and
  successful local decoder mission decode.
