## Why

> **Historical note.** This change predates `decode-minimal-fixture-avm-skip-polarity`, which retired the frozen `base_q_idx == 255` committed minimal fixture (`syn-flat-intra-64x64-minimal.ivf`) and replaced it with an AVM/dav2d-conformant `base_q_idx` 210 luma-skip stream that routes through the general intra path. References below to the committed minimal fixture as the frozen `base_q_idx == 255` anchor — and to keeping that committed fixture's hash byte-identical — are historical; the routing rule (a `base_q_idx == 255` frame falls through to the frozen gate) still holds.

`splot decode` only ever reconstructs the committed frozen minimal-tier hash
fixture (`base_q_idx == 255`, an all-zero residual, a fixed six-symbol trace).
Every other real intra stream is rejected at the frame-tools gate, so the large
coefficient-decode and reconstruction machinery is never exercised on a real
bitstream and progress cannot be measured against a reference decoder.

A tiny AVM-generated minimal-tool intra `OBU_CLOSED_LOOP_KEY` stream gives the
first measurable on-ramp: `avmdec` and `dav2d` decode it to byte-identical raw
output. The first brick stands up a general intra decode path next to the frozen
tier and proves splot's real AV2 § 5.20.3.1 partition traversal runs on that
stream, scoping the remaining gap precisely.

## What Changes

- Add Feature ID `DECODE-GENERAL-INTRA-FRAME-FRONTIER`.
- Add a crate-private general intra decode frontier in `splot-decode` that
  routes a single-tile 64x64 8-bit 4:2:0 intra key frame with any `base_q_idx`
  other than the frozen fixture value 255 (and with segmentation, quant
  matrices, delta-Q, in-loop filters, CCSO, GDF, and film grain disabled) off
  the frozen hash tier, runs the real AV2 § 5.20.3.1 root partition traversal to
  the single-block frontier, then returns a structured
  `decode/unsupported-feature` diagnostic for the not-yet-wired block-symbol,
  coefficient, and reconstruction decode.
- Keep the frozen `base_q_idx == 255` minimal hash contract byte-identical.
- Commit `syn-flat-intra-64x64-q80.ivf` (a real nonzero-DC-residual fixture
  whose avmdec and dav2d raw outputs agree) and record the agreement in the
  local-reference evidence manifest.
- Add focused CLI tests for the general path reaching the partition frontier and
  for the frozen hash regression guard.
- Update decoder tracking, roadmap, generated status docs, and OpenSpec tasks.

## Capabilities

### New Capabilities
- `decode-general-intra-frame-frontier`: Crate-private general intra decode
  frontier that accepts a minimal-tool intra key frame off the frozen hash tier
  and runs the real partition traversal to the single-block frontier.

### Modified Capabilities
- `decoder-support`: Track the new partial decoder-support row for the general
  intra frame frontier.

## Impact

- Affects `crates/splot-decode/src/runtime_minimal.rs`,
  `crates/splot-decode/src/tile_payload.rs`, and
  `crates/splot-cli/tests/decode_cli.rs`.
- Adds `tests/conformance/vectors/valid/syn-flat-intra-64x64-q80.ivf` plus
  `tests/conformance/manifest.toml` and `docs/LOCAL-REFERENCE-EVIDENCE.toml`
  entries.
- Updates `docs/IMPLEMENTATION-MATRIX.toml`,
  `docs/DECODER-SUPPORT-MATRIX.toml`,
  `xtask/src/decoder_conformance_coverage.rs`, `docs/DECODER-ROADMAP.md`, and
  generated status/coverage docs.
- No public API, dependency graph, encoder, validator, dequantization, inverse
  transform, residual add, reconstruction, output, reference-refresh, or
  in-repo AVM/dav2d integration changes are in scope.
