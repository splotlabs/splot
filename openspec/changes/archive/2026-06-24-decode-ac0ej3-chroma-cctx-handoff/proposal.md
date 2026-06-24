## Why

The live `ac0ej3.ivf` decode has advanced past active intra IST metadata handoff and now reaches chroma residual syntax in the active Wiener NS LR transform-record path. The current DCT-only residual guard rejects before reading CCTX type syntax or chroma eob-1 non-DCT transform-set coefficient syntax, so the stream cannot advance to the next authoritative runtime frontier.

Feature ID: `DECODE-AC0EJ3-CHROMA-CCTX-HANDOFF`.

## What Changes

- Add the `TileCctxTypeCdf` row to the tile CDF subset and expose it through the existing block-symbol trace read boundary.
- In the Wiener NS LR tx-skip record handoff only, read U-plane `cctx_type` when §5.20.7.27 requires it and retain the decoded value as syntax metadata, including nonzero values.
- In the same handoff-only policy, admit intra chroma non-DCT transform-set coefficient syntax after recording the required CCTX type, so the existing ordinary coefficient path can derive the real chroma transform type without claiming reconstruction support.
- Keep reconstruction, output, CCTX transforms, chroma output, and successful ac0ej3 decode unsupported.

## Capabilities

### New Capabilities
- `ac0ej3-chroma-cctx-handoff`: Covers the syntax-only CCTX metadata and chroma transform-type handoff needed by the live ac0ej3 Wiener NS LR tx-skip record path.

### Modified Capabilities
- `decoder-support`: Adds the decoder support row and live ac0ej3 frontier evidence for this handoff.

## Impact

- Affected code: `crates/splot-decode/src/tile_payload/cdf*`, `crates/splot-decode/src/tile_payload/general_intra_residual.rs`, and Wiener NS LR tx-skip record tests.
- Affected docs/tracking: `docs/IMPLEMENTATION-MATRIX.toml`, `docs/DECODER-SUPPORT-MATRIX.toml`, generated status/coverage docs, and OpenSpec decoder-support specs.
- No new dependencies, public APIs, encoder behavior, or license changes.
