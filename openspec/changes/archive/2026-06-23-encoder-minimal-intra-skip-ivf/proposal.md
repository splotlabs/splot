## Why

The decodable-tile arc, brick 5 — the milestone. Bricks 1-4 built every piece of a decodable
minimal intra skip frame: the `do_split` flag, the mode prefix, the per-plane `txb_skip`
symbols at the general transform contexts, their § 8.2.4 finalization into `tile_data`, and a
`base_q_idx <= 90` container. This brick assembles them into `splot-encode`'s first public
"emit a decodable stream" function and proves, cross-crate, that `splot decode` reconstructs
the stream to a flat frame.

## What Changes

- Add `ENC-MINIMAL-INTRA-SKIP-IVF` as an encoder feature (splot-encode + splot-cli oracle).
- Add `splot_encode::emit_minimal_intra_skip_ivf() -> Result<Vec<u8>>`: it composes the
  general-intra DC skip `tile_data` and muxes it through the `base_q_idx`-80 minimal CLK IVF
  container, returning a complete decodable AV2 IVF. Add a `MinimalIntraSkipIvf` error
  variant for the container-assembly failure path.
- Add the cross-crate oracle `crates/splot-cli/tests/encode_decode_roundtrip.rs`: it emits the
  IVF and runs `splot decode --output-format raw`, asserting a 6144-byte frame that is flat at
  `128` (the § 7.13.2 DC prediction of a no-neighbour skipped block).

## Capabilities

### New Capabilities

- None.

### Modified Capabilities

- `encoder-tools`: add a requirement for the first end-to-end decodable minimal intra skip
  stream and its cross-crate decode oracle.

## Impact

- Affected code: `crates/splot-encode/src/general_intra_trace.rs` (the public function),
  `crates/splot-encode/src/error.rs` (the error variant), `crates/splot-encode/src/lib.rs`
  (the re-export), `crates/splot-cli/tests/encode_decode_roundtrip.rs` (new oracle).
- Affected docs/tracking: `docs/IMPLEMENTATION-MATRIX.toml`, generated feature status/spec
  coverage, `openspec/specs/encoder-tools/spec.md`.
- Public API impact: one added `splot-encode` function and one error variant. No
  dependency-graph change (splot-cli already depends on splot-encode and splot-decode).
- Validator/CLI impact: none (a new test only; no CLI surface change).
