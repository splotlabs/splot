## Why

The local ac0ej3 decode probe now reaches the Wiener NS LR transform-record
walk and stops at `unsupported_wienerns_lr_live_transform_record_fsc_mode`.
This is the next concrete stream frontier after active MRL metadata retention:
the runtime already has loaded coefficient FSC handoff helpers, but the live
ac0ej3 LR tx-skip record path still rejects the observed `fsc_mode` subcase
before those helpers can consume the syntax.

## What Changes

- Extend the ac0ej3 selectable-transform record path for
  `DECODE-AC0EJ3-SELECTABLE-TRANSFORM-RECORDS` to carry caller-resolved
  `fsc_mode` into the nonzero luma residual handoff.
- Route the observed supported `fsc_mode`/IDTX luma transform-record residual
  subcase through the existing frame-facts `useFsc` coefficient wrapper so
  FSC coefficients are consumed into LR tx-skip metadata without decoded sample
  population.
- Preserve fail-closed diagnostics for unobserved or unsupported FSC, non-luma,
  reconstruction, filter, and output paths.
- Update the implementation matrix, decoder support matrix/status, and
  OpenSpec specs/tasks with local probe evidence for the next structured
  ac0ej3 frontier.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `ac0ej3-selectable-transform-records`: add the bounded live FSC
  transform-record residual handoff for the local ac0ej3 Wiener NS LR path.
- `decoder-support`: update support-row requirements and proof expectations for
  the ac0ej3 selectable-transform frontier after the FSC record handoff.

## Impact

- Affected code is expected to stay within `splot-decode` runtime/tile-payload
  internals, primarily
  `crates/splot-decode/src/runtime_minimal/wienerns_lr/tx_records.rs` and
  `crates/splot-decode/src/tile_payload/general_intra_residual.rs`.
- Affected tracking files are `docs/IMPLEMENTATION-MATRIX.toml`,
  `docs/DECODER-SUPPORT-MATRIX.toml`, generated decoder status docs, and this
  OpenSpec change.
- No public API, CLI option, dependency graph, encoder, or external reference
  tool invocation changes are intended.
- Non-goals: decoded `CurrFrame`/`CdefFrame` samples, inverse transforms,
  reconstruction/output, loop-restoration filtering, reference refresh,
  AVM/dav2d byte equality, broad FSC/IDTX support, and successful ac0ej3 decode.
