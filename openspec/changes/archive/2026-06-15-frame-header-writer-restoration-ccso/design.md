# Design: frame-header-writer-restoration-ccso

## Context

`lr_params()` (§ 5.18.7.11) selects a per-plane restoration tool (`tool_index ns(n)` over an
enabled-tools table), optionally signals `frame_filters_on`, and signals the per-plane
`LoopRestorationSize` via a `SbSize`-dependent flag cascade. `ccso_params()` (§ 5.18.7.12) signals
a frame flag, then per plane a `ccso_planes` flag and (when set) the bo_only / scale / quant / ext
/ edge_clf / max_band cascade and a `ccso_offset_idx tu(7)` loop. The LR Wiener-bank decode
(`read_wienerns_filter()`) is unmodeled — the parser stops before it; the CCSO offset values were
surfaced by PR1 (`ccso-offset-index-model`).

## Decisions

- **LR is additive with a hard residual; CCSO is additive on PR1's surface.** The LR Wiener bank
  is genuinely unmodeled (the parser *stops*, never producing a complete `LrParams` with
  `frame_filters_on == true`), so the writer rejects that case — it is not parser-reachable. CCSO
  is fully modeled after PR1, so its writer is byte-exact with no further model change.
- **Reverse the `tool_index` via a shared table.** The `indexToTool` construction (RESTORE_NONE,
  the enabled switchable tools, RESTORE_SWITCHABLE) is extracted into a `pub(crate)` helper that
  both the parser and the writer call, so the table — and the `ns(n)` width `n = toolsCount +
  allowSwitchable` — never drift. The writer maps the plane's restoration type to its tool id and
  finds its index; a type not in the table (a disabled tool) is rejected.
- **Reverse the size-shift uniquely.** `LoopRestorationSize = base >> shift` is an exact
  power-of-two division, so the writer recovers `shift` (validating `base == size << shift`) and
  emits the `*_use_half/max/quarter_size` flags by `SbSize`, mirroring `read_lr_size_shift`. A
  shift unreachable for the frame's `SbSize` (e.g. shift 2 with `Block256x256`) is rejected — the
  encoding is unique, so the round-trip is byte-exact.
- **CCSO offset loop re-derives its own length.** The writer recomputes `maxEdgeInterval` (from
  `ccso_bo_only` / `ccso_edge_clf`) and `maxBand` (`1 << ccso_max_band_log2`) exactly as the
  parser does, validates `ccso_offset_idx.len()` matches, and emits each value via the new
  `write_tu`. The quant-step gate (`CCSO_Quant_Sz[scale][quant] != 0` → `ccso_edge_clf` coded)
  uses the same constant as the parser.
- **No panic on constructed models.** The `CCSO_INPUT_INTERVAL - edge_clf` subtraction cannot
  underflow (`edge_clf` is 0/1, the constant is 3); `1 << ccso_max_band_log2` is guarded by the
  `f(2 + bo_only)` domain (`<= 7`); `CCSO_Quant_Sz` is indexed with `.get()`; the size-shift
  division guards `size != 0` and the exact-power-of-two check before `trailing_zeros`. Every
  `f(n)` value is domain-checked before the write, and `check_*_encodable` runs fully before the
  first write.

## Testing

Round-trip via the public parsers across every branch (LR: disabled / all-NONE / a real tool +
size signaling exercising shifts 0–3 across the three `SbSize` arms / luma-only / chroma-only /
both / num_planes 1 vs 3; CCSO: disabled / frame-flag-false / single-picture / all-off / bo_only /
full-arm / quant-step-0 / multi-offset). One reject test per `NonCanonicalFrameHeader` path,
including the LR `frame_filters_on` hard residual, the unreachable-shift rejects, and the CCSO
length / domain rejects (each asserting `bit_len() == 0`). A round-trip property test per parser
(LR round-trips only the `Parsed` outcomes) plus a never-panics-on-constructed-models proptest per
writer, and a `write_tu` round-trip against the reader.
