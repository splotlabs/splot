# Change: frame-header-writer-quantization

## Feature IDs

- `ENC-BITSTREAM-WRITER` (advances the writer surface; umbrella stays `partial`)
- `AV2-5.18.6-QUANTIZATION` (advances its `write` stage `todo -> done`, the modeled
  intra surface)

## Why

Fourth slice (#4d) of the frame-header writer (intra path). It inverts the five quant
parsers — `read_delta_q` (§ 5.18.6.3), `quantization_params` (§ 5.18.6.1),
`setup_qm_params` (§ 5.18.6.2), `delta_q_params` (§ 5.18.7.8), and the § 5.18.2
lossless / `allow_tcq` / `allow_parity_hiding` tail.

Unlike #4b / #4c, this slice is **additive — no model change**. The few read-but-not-stored
points are **redundant encodings of a fully-preserved value** (not layout-affecting discards),
so they are handled by **canonicalization** — the same approach the sequence writer takes for
leb128-minimal and the `num_ref_frames == 8` alias.

## What changes

- **Writers** (`crates/splot-core/src/write/frame_quant.rs`): `write_read_delta_q`,
  `write_quantization_params`, `write_setup_qm_params`, `write_delta_q_params`,
  `write_lossless_info` — each validating the whole model up front (reject-before-write).
- **Four documented canonical forms** (semantic round-trip universal; byte-exact on the
  canonical subset):
  - `read_delta_q`: `delta_q == 0` → `delta_coded = 0` (not the `delta_coded = 1, su(7) = 0`
    form).
  - `setup_qm` `qm_uv_same_as_y`: when `qm_u == qm_y && qm_v == qm_y` → `qm_uv_same_as_y = 1`.
  - `quantization_params` `equal_ac_dc_q`: the parser reads then overwrites the chroma DC with
    the AC value; the writer re-emits the retained (AC) value.
  - lossless `qm_index`: recovered by reverse-lookup over the full `f(CeilLog2(qmNum))` coded
    domain (indices `>= qmNum` reference the parser's zeroed default levels).
- **Visibility only** outside `write/`: `get_qindex_ignore_delta_q` (re-derive the § 5.18.2
  lossless state) and `ceil_log2` (the `qm_index` field width) are made `pub(crate)`. No model
  field and no `WriteError` variant added.

## Validator impact

None. No new diagnostics; the validator is unchanged.

## Non-goals

- No `segmentation_params()` (§ 5.18.7.1), filter, restoration, or tail writers — later
  slices.
- No composing `write_frame_header`.

## Impact

- Crate: `crates/splot-core` (additive `write` module + `pub(crate)` visibility on two parser
  helpers).
- Docs: `docs/IMPLEMENTATION-MATRIX.toml` (+ regenerated `docs/FEATURE-STATUS.md`).
