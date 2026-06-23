## Why

The decodable-tile arc, brick 3. Brick 2 composed the general-intra DC skip-block symbol
trace and proved it round-trips through one § 8.2 coder. This brick turns that trace into its
§ 8.2.4-finalized **`tile_data` bytes** — the entropy-coded payload a single-tile general
intra frame carries directly. A single last tile has no `tile_size_minus_1` prefix, so the
decoder consumes these bytes from byte 0 via § 8.2.2 `init_symbol`; the encoder's finalized
output is therefore usable as `tile_data` with no wrapper. This is the byte boundary between
the encoder's symbol trace and the container assembly that later bricks add.

## What Changes

- Add `ENC-GENERAL-INTRA-SKIP-TILE-DATA` as an encoder block-symbol-trace feature
  (splot-encode).
- Add `general_intra_trace::encode_general_intra_dc_skip_tile_data() -> Result<Vec<u8>>`: it
  composes the brick-2 skip trace and finalizes it through the existing
  `encode_block_symbol_trace` § 8.2 coder, returning the `tile_data` bytes.
- Document the muxing contract a later brick must honor: the frame header must set
  `base_q_idx <= 90` (coefficient CDF q-context `0`) and `disable_cdf_update == 0` (so the
  tile reader's adaptive CDFs match the `CdfUpdateMode::Enabled` this trace is coded under).

## Capabilities

### New Capabilities

- None.

### Modified Capabilities

- `encoder-tools`: add a requirement for the encoder general-intra DC skip `tile_data` bytes.

## Impact

- Affected code: `crates/splot-encode/src/general_intra_trace.rs` (the new function +
  tests).
- Affected docs/tracking: `docs/IMPLEMENTATION-MATRIX.toml`, generated feature status/spec
  coverage, `openspec/specs/encoder-tools/spec.md`.
- Public API impact: none (all crate-private). No dependency-graph change.
- Validator/CLI impact: none. The cross-crate decode oracle (splot-cli) and the
  `base_q_idx <= 90` frame-header variant (splot-core) are separate later bricks.
