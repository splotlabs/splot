## Why

The decodable-tile arc, brick 2. Brick 1 added the root `do_split` partition flag — the
first symbol the AVM-validated general intra decode path reads. To emit a complete block the
encoder must compose the full ordered symbol stream that path reads for one undivided 64x64
superblock: the `do_split` flag, the § 5.20.5.3 mode-info prefix, then the per-plane
§ 5.20.7.27 `all_zero` (`txb_skip`) symbols. The existing
`compose_minimal_intra_dc_complete_all_zero_block_trace` codes the `txb_skip` symbols at the
minimal `TX_4X4` context and never leads with `do_split`; the general path reads them at the
64x64-leaf transform contexts (`TX_64X64` luma, `TX_32X32` chroma) after `do_split`.

## What Changes

- Add `ENC-GENERAL-INTRA-SKIP-BLOCK-TRACE` as an encoder block-symbol-trace feature
  (splot-encode).
- Add `general_intra_64x64_luma_all_zero_token` and
  `general_intra_32x32_chroma_u_all_zero_token` coefficient-token constructors (the general
  path's `TX_64X64` / `TX_32X32` `txSzCtx`), with the `txSzCtx` values (`4`, `3`) confirmed
  empirically against the decoder's general-intra `txb_skip` selector.
- Add a `general_intra_trace` module with `compose_general_intra_dc_skip_block_trace()`: the
  ordered trace `[do_split, y_mode_set, y_mode_index, uv_mode, luma all_zero, U all_zero,
  V all_zero]` = `[0, 0, 0, 0, 1, 1, 1]`, at coefficient CDF q-context `0`
  (`base_q_idx <= 90`).
- Route the two general-context `txb_skip` rows through `BlockSymbolTraceCdfRows` and
  `row_mut` so the trace composes through the existing `roundtrip_block_symbol_trace`.

## Capabilities

### New Capabilities

- None.

### Modified Capabilities

- `encoder-tools`: add a requirement for the encoder general-intra DC skip-block trace.

## Impact

- Affected code: `crates/splot-encode/src/general_intra_trace.rs` (new),
  `crates/splot-encode/src/coefficient_tokenization.rs` (two general token constructors),
  `crates/splot-encode/src/block_symbol_trace.rs` (the two general `txb_skip` CDF rows +
  routing), `crates/splot-encode/src/coefficient_tokenization_tests.rs` (constructor tests),
  `crates/splot-encode/src/lib.rs` (module).
- Affected docs/tracking: `docs/IMPLEMENTATION-MATRIX.toml`, generated feature status/spec
  coverage, `openspec/specs/encoder-tools/spec.md`.
- Public API impact: none (all crate-private). No dependency-graph change.
- Validator/CLI impact: none.
