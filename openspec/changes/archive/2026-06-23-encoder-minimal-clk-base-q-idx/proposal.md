## Why

The decodable-tile arc, brick 4. Bricks 2-3 produced a general-intra DC skip `tile_data`
coded at coefficient CDF q-context `0` — which the decoder derives only from a frame whose
`base_q_idx <= 90`. The existing minimal `OBU_CLOSED_LOOP_KEY` container assembler hardcodes
`base_q_idx == 255` (q-context 3), so muxing the skip `tile_data` into it would make the
decoder read the `txb_skip` symbols at the wrong q-context. This brick parameterizes the
minimal CLK container by `base_q_idx` so a later brick can mux the skip `tile_data` into a
decodable frame.

## What Changes

- Add `ENC-MINIMAL-CLK-BASE-Q-IDX` as an encoder writer-bridge feature (splot-core).
- Thread `base_q_idx` through the minimal CLK assembly chain (`minimal_intra_clk_body_bytes`,
  `build_minimal_intra_clk_core`, the tile-group / Annex B / IVF assemblers) via private
  impls; the frozen no-arg public functions delegate at the frozen `base_q_idx == 255`.
- Add one public entry point, `encode_minimal_intra_clk_ivf_with_base_q_idx(base_q_idx,
  tile_data)`, and reject `base_q_idx == 0` (it would make `CodedLossless == 1` and change the
  fixed § 5.18.2 body layout) with a typed `MinimalIntraCoreError::LosslessBaseQIdx`.

## Capabilities

### New Capabilities

- None.

### Modified Capabilities

- `encoder-tools`: add a requirement for the `base_q_idx`-parameterized minimal CLK container.

## Impact

- Affected code: `crates/splot-core/src/headers/frame/encoder_input.rs` (the chain + the new
  public function + the error variant + tests), `crates/splot-core/src/headers/frame/mod.rs`
  (re-export).
- Affected docs/tracking: `docs/IMPLEMENTATION-MATRIX.toml`, generated feature status/spec
  coverage, `openspec/specs/encoder-tools/spec.md`.
- Public API impact: one added function; the existing frozen-tier functions are unchanged in
  signature and behavior. No dependency-graph change.
- Validator/CLI impact: none. The cross-crate decode oracle is a later splot-cli brick.
