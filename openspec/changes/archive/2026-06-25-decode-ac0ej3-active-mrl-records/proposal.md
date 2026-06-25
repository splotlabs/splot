## Why

The ac0ej3 mission stream now reaches the active MRL mode-info frontier after selectable narrow transform-record handoff. The current runtime consumes the nonzero `mrl_index` / `mrl_sec_index` symbols and immediately rejects, which prevents later Wiener NS LR `LrTxSkip` record derivation from observing the stream's next syntax frontier.

## What Changes

- Track `DECODE-AC0EJ3-ACTIVE-INTRA-TOOL-FRONTIER` as still partial, but widen the LR transform-record handoff to retain active MRL metadata for luma/shared leaves.
- Add tile-local `UsesMrls` state derived from decoded `mrl_index` / `mrl_sec_index`, and use neighbouring `UsesMrls` for AV2 §8.3.2 `mrl_index` / `mrl_sec_index` CDF contexts.
- Allow active MRL syntax to continue through selectable transform-record and skipped residual derivation when no decoded sample prediction is claimed.
- Keep full active MRL prediction, DIP, IBP, intra-edge filtering, decoded samples, loop-restoration output, reference refresh, AVM/dav2d equality, and successful ac0ej3 decode out of scope.

## Capabilities

### New Capabilities
(none)

### Modified Capabilities
- `ac0ej3-active-intra-tool-frontier`: active MRL syntax is retained as metadata for LR tx-skip record derivation instead of always producing the active-MRL unsupported diagnostic.
- `decoder-support`: the ac0ej3 support rows and probe frontier move from active MRL mode-info to the next unsupported runtime frontier reached after active MRL metadata retention.

## Impact

- Affected code: `crates/splot-decode/src/tile_payload/general_intra_block.rs`, `crates/splot-decode/src/tile_payload/intra_joint_modes.rs`, `crates/splot-decode/src/tile_payload/partition_traversal.rs`, `crates/splot-decode/src/tile_payload/runtime_frontier.rs`, and `crates/splot-decode/src/runtime_minimal/wienerns_lr/`.
- Affected docs/status: `docs/IMPLEMENTATION-MATRIX.toml`, `docs/DECODER-SUPPORT-MATRIX.toml`, generated decoder support/status docs, and OpenSpec active capability specs.
- Diagnostics remain structured `decode/unsupported-feature`; this change replaces the current `unsupported_wienerns_lr_live_transform_record_mrl_mode` ac0ej3 stop with `unsupported_wienerns_lr_live_transform_record_fsc_mode`.
- No new dependency, crate graph, license, unsafe, encoder, or broad reconstruction change.
