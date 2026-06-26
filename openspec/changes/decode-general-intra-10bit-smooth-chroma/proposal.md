# Change: decode-general-intra-10bit-smooth-chroma

## Feature IDs

- `DECODE-GENERAL-INTRA-10BIT-SMOOTH-CHROMA`

## Why

The merged `DECODE-GENERAL-INTRA-10BIT` 10-bit (`bit_depth_idc == 0`, AV2
§ 6.4.1 Table 6.3) general-intra reconstruction admits only DC chroma: a 10-bit
block whose chroma resolves to a non-DC mode is rejected up front with
`unsupported_10bit_non_dc_intra`. The 8-bit general-intra path already
reconstructs § 7.13.2.13 SMOOTH chroma over the § 7.13.2.1 no-neighbour fallback
edges at the top-left block, and the reconstruction math is bit-depth-generic, so
the smallest verifiable next 10-bit increment is admitting SMOOTH chroma at that
same no-neighbour top-left shape. This is pinned by a committed fixture whose
avmdec and dav2d raw outputs agree byte-for-byte.

## What Changes

- Add Feature ID `DECODE-GENERAL-INTRA-10BIT-SMOOTH-CHROMA`.
- Relax the 10-bit chroma admission gate in `general_intra.rs` to also admit
  `SupportedChromaMode::Smooth` when the block is the no-neighbour top-left leaf
  (`frontier.r == 0 && frontier.c == 0`), in addition to the already-admitted DC
  chroma. The DC_PRED-luma single-64x64 square-leaf shape is unchanged; a 10-bit
  non-DC luma block, a non-DC / non-(top-left SMOOTH) chroma block, or a
  neighbour-having SMOOTH chroma block (frame-MI `c != 0`, which would read real
  reconstructed 10-bit edges no fixture pins) still rejects with
  `unsupported_10bit_non_dc_intra` before any coefficient read or sample write.
- Add the project-owned `syn-smchroma-intra-64x64-10bit-q160.ivf` fixture and a
  decode test proving it reconstructs bit-exactly to the avmdec / dav2d oracle.
- Update decoder tracking (matrix, decoder-support, LOCAL-REFERENCE-EVIDENCE,
  conformance manifest) and the generated status docs.

## Capabilities

### New Capabilities
- `decode-general-intra-10bit-smooth-chroma`: A 10-bit (`bit_depth_idc == 0`)
  general intra reconstruction that admits DC_PRED luma plus § 7.13.2.13 SMOOTH
  chroma at the no-neighbour top-left block (single 64x64).

### Modified Capabilities
- `decoder-support`: Track the new partial decoder-support row for the 10-bit
  general intra DC-luma + SMOOTH-chroma reconstruction.

## Impact

- Adds `tests/conformance/vectors/valid/syn-smchroma-intra-64x64-10bit-q160.ivf`
  plus a decode test in
  `crates/splot-decode/src/runtime_minimal/general_intra_tests.rs`.
- Modifies `crates/splot-decode/src/runtime_minimal/general_intra.rs` (the 10-bit
  chroma admission gate only).
- Updates `docs/IMPLEMENTATION-MATRIX.toml`,
  `docs/DECODER-SUPPORT-MATRIX.toml`, `docs/LOCAL-REFERENCE-EVIDENCE.toml`,
  `tests/conformance/manifest.toml`, and the generated status/coverage docs.
- No public API, dependency graph, encoder, or validator changes. Neighbour-having
  SMOOTH chroma, 10-bit non-DC luma, non-64x64 partition-leaf reconstruction,
  10-bit inter prediction / reference retention, in-loop filters, and live in-CI
  AVM/dav2d remain out of scope.

## Non-goals

- No neighbour-having (`frame-MI c != 0`) SMOOTH chroma reconstruction.
- No 10-bit non-DC luma prediction.
- No 10-bit non-64x64 partition-leaf reconstruction (rectangular or split square
  sub-block).
- No 10-bit inter prediction or 10-bit reference-frame retention.
- No change to the successful 8-bit fixture subset or the existing 10-bit
  DC-chroma subset.

## Acceptance criteria

- [ ] `splot decode syn-smchroma-intra-64x64-10bit-q160.ivf` with
      `--output-format raw` produces output byte-identical to avmdec/dav2d
      (raw md5 `a09a6344f3ec7a1efbb695d4f527d7c8`).
- [ ] `--output-format hash` succeeds and emits the stable `splot-dfh-sha256-v1`
      digest `4fe932e5e5dea4a1830eae4853b198c738e8d1919049736d2f4a234c491d5397`.
- [ ] Every existing 8-bit and 10-bit conformance fixture stays byte-identical
      / fails closed as before.
- [ ] A 10-bit non-DC luma, a non-DC / non-(top-left SMOOTH) chroma, or a
      neighbour-having SMOOTH chroma block still rejects with
      `unsupported_10bit_non_dc_intra`.
- [ ] Feature tracking, OpenSpec, and generated docs are updated.
