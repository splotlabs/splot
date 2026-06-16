# Change: frame-header-writer-compose

## Feature IDs

- `ENC-BITSTREAM-WRITER` (advances the writer surface; umbrella stays `partial`)
- `AV2-5.18.2-FRAME-HEADER-INFO` (the composing `write_frame_header_core`; `write` → `partial`
  — the modeled intra path is complete, but the row also covers the inter paths, which stay
  unwritten, so `write` stays `partial` in lockstep with `parse`, like the sibling
  § 5.18.3/§ 5.18.4/§ 5.18.7 writer rows)
- `AV2-5.18-FRAME-HEADER` (umbrella; `write` stays `partial` — the inter / SEF paths stay TODO)

## Why

Ninth and final slice (#4i) of the frame-header writer (intra path). It composes the
`write_frame_header_core` capstone over all of #4a–#4h: the exact inverse of
`parse_frame_header_core` on the path that reaches
`FrameHeaderParseStatus::IntraHeaderComplete`. With it, splot can write a complete intra
frame header and round-trip it (`parse(write(x)) == x`).

## What changes

- **Composing writer** (`crates/splot-core/src/write/frame_header_core.rs`):
  `write_frame_header_core(writer, core, seq, mfh)` — emits the whole intra frame header in
  § 5.18.2 order by writing the control-region "glue" bits directly and delegating each
  sub-structure to the existing #4a–#4h sub-writers.
- **Control-region glue** (no existing sub-writer): the activation prefix (reusing
  `write_frame_header_prefix`), the frame-type arm (`restricted_prediction_switch` /
  `frame_is_inter`), the long-term-id reads, the output-control flags, `frame_size_override_flag`,
  `order_hint`, `refresh_frame_flags` (the exact inverse of `read_refresh_frame_flags`, including
  the KEY all-1s / short / full arms), and `disable_cdf_update`.
- **Reject-before-write for the whole composition.** The writer accepts only a model whose
  `status == IntraHeaderComplete` (and `frame_is_intra`, the required `Option`s present, no
  `lr_params_partial`); a scratch-`BitWriter` is used so a sub-structure reject mid-compose
  leaves the real `writer` untouched (`bit_len() == 0`), never a partial buffer.
- **Seq-view exposure**: the `CoreSeqView` / `MfhFrameView` parser inputs (and their
  constructors) are exposed for the writer's public signature.
- **No model field and no new `WriteError` variant** (reuses `NonCanonicalFrameHeader`).

## Validator impact

None. No new diagnostics.

## Non-goals

- No inter / switch / TIP / bridge / SEF frame-header writers (those paths reach a different
  terminal status; this writer rejects them).
- No tile-group / metadata payload writers (later mission backlog).

## Impact

- Crate: `crates/splot-core` (additive `write` module + the seq-view exposure).
- Docs: `docs/IMPLEMENTATION-MATRIX.toml` (+ regenerated `docs/FEATURE-STATUS.md`).
