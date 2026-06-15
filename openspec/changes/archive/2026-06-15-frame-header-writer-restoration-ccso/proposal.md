# Change: frame-header-writer-restoration-ccso

## Feature IDs

- `ENC-BITSTREAM-WRITER` (advances the writer surface; umbrella stays `partial`)
- `AV2-5.18.7-SEGMENTATION-TILING` (advances the `lr_params()` / `ccso_params()` portion of its
  `write` stage; the row stays `partial` — the inter paths and the frame-level Wiener bank remain)

## Why

Seventh slice (#4g, PR2) of the frame-header writer (intra path). It inverts the two
loop-restoration / CCSO parsers: `lr_params()` (§ 5.18.7.11) and `ccso_params()` (§ 5.18.7.12).
This is the writer half; PR1 (`ccso-offset-index-model`) already extended the model to surface
the `ccso_offset_idx` values the CCSO writer needs.

## What changes

- **`write_tu` primitive** (`crates/splot-core/src/write/bit_writer.rs`): the inverse of the
  `tu(mx)` truncated-unary reader (§ 4.11.9) — `value` `1`-bits then a `0` terminator when
  `value < mx`. The `ccso_offset_idx` writer needs it.
- **Writers** (`crates/splot-core/src/write/frame_restoration.rs`): `write_lr_params` and
  `write_ccso_params`, each validating the whole model up front (`check_*_encodable`,
  reject-before-write; `bit_len() == 0` on every reject).
- **LR hard residual.** A *complete* `LrParams` can never carry `frame_filters_on == true` — the
  parser returns `LrParseOutcome::StoppedBeforeWienerNsFilter` (a stop, not a complete parse) the
  moment a plane signals it, because `read_wienerns_filter()` (the frame-level Wiener bank) is
  unmodeled. So `write_lr_params` **rejects** any plane with `frame_filters_on == true` and ships
  the rich `frame_filters_on == false` surface (the `tool_index ns(n)` reverse-lookup over the
  enabled-tools table and the `LoopRestorationSize` size-shift reversal). Additive — no model
  change.
- **CCSO byte-exact.** `write_ccso_params` reproduces every plane, including the per-plane
  `ccso_offset_idx tu(7)` loop, using the values surfaced by PR1. Additive — no model change.
- **Drift-proof extractions** (`crates/splot-core/src/headers/frame/restoration.rs`): the
  `indexToTool` per-plane tool table is pulled into a `pub(crate)` helper that both
  `parse_lr_params` and the writer call (behavior-preserving); `RESTORATION_TILESIZE_MAX`,
  `default_restoration_size`, and the CCSO quant-step constants are exposed `pub(crate)`.
- **No new `WriteError` variant** (reuses `NonCanonicalFrameHeader`; `write_tu` reuses
  `ValueOutOfRange`).

## Validator impact

None. No new diagnostics.

## Non-goals

- No § 5.18.7.11 frame-level Wiener-bank (`read_wienerns_filter()`) writer — genuinely unmodeled
  (the parser stops); `frame_filters_on == true` is rejected.
- No inter-path lr/ccso reuse arms (dead on the intra path).
- No composing `write_frame_header`.

## Impact

- Crate: `crates/splot-core` (additive `write` module + a `write_tu` primitive + behavior-preserving
  `pub(crate)` extractions in the restoration parser).
- Docs: `docs/IMPLEMENTATION-MATRIX.toml`.
