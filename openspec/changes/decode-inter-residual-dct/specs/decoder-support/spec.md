## ADDED Requirements

### Requirement: Inter block residual decode support row
The decoder support model SHALL track `DECODE-INTER-RESIDUAL-DCT` as a distinct
partial `splot-decode` row named `inter-residual-dct`. The row SHALL cite AV2
§ 5.20.7, § 5.20.7.27, § 5.20.8.2, § 5.20.8.3, § 7.14.3, § 7.14.4, and § 7.15.4,
SHALL record the inter-residual decode tests plus the conformance manifest test,
and SHALL carry the reciprocal LOCAL-REFERENCE-EVIDENCE pointer for the two-frame
inter-residual fixture. The row SHALL document that the verified subset is the
single-reference zero-MV-or-sub-pel `skip ∈ {0, 1}` 64x64 block whose `skip == 0`
residual is the DCT-only (no inter-IST / inter-DDT / CCTX / FSC / IDTX-intra)
TX_64X64 luma + TX_32X32 chroma case, and SHALL keep multi-transform-block
splits, non-DCT inter transform sets, compound / multi-reference prediction,
motion modes, non-64x64 / multi-block inter, in-loop filters, and live AVM/dav2d
invocation in CI out of scope as deferred work.

#### Scenario: Matrix records narrow inter-residual support
- **WHEN** `cargo xtask check-decoder-support` validates the decoder support
  matrix
- **THEN** row `inter-residual-dct` appears with Feature ID
  `DECODE-INTER-RESIDUAL-DCT`
- **AND** it is marked partial rather than supported for inter decode
- **AND** it does not claim multi-transform-block residuals, non-DCT inter
  transform sets, compound / multi-reference prediction, motion modes, or
  multi-block inter
