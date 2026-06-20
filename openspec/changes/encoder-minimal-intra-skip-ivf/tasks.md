## 1. Public emit function

- [x] 1.1 Add `splot_encode::emit_minimal_intra_skip_ivf()`: compose the skip `tile_data` and mux it through `encode_minimal_intra_clk_ivf_with_base_q_idx(80, ..)`.
- [x] 1.2 Add the `Error::MinimalIntraSkipIvf` variant for the container-assembly failure path, and re-export the function from the crate root.

## 2. Cross-crate decode oracle

- [x] 2.1 Add `crates/splot-cli/tests/encode_decode_roundtrip.rs`: emit the IVF, run `splot decode --output-format raw`, and assert a 6144-byte frame flat at `128`.
- [x] 2.2 A splot-encode test asserts the emitted bytes parse as a single-frame AV02 64x64 IVF, and that emission is deterministic.

## 3. Tracking and verification

- [x] 3.1 Add `ENC-MINIMAL-INTRA-SKIP-IVF` to the implementation matrix and refresh generated status/coverage docs.
- [x] 3.2 Keep tracking honest: the first decodable minimal intra skip stream, decode-verified against `splot-decode`; it is not yet AVM/dav2d-validated for this exact stream, nor a general encoder, a `receive_packet` packet, nor Baseline Encoder Profile v1.
- [x] 3.3 Run OpenSpec validation, focused encode/cli tests, feature-status checks, and `cargo xtask ci`.
