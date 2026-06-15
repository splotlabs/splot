# encoder-tools delta: frame-header-writer-quantization

## ADDED Requirements

### Requirement: frame-header quantization writers

`splot-core` SHALL provide writers that are the exact inverse of the § 5.18.6 quantization
parsers (`read_delta_q`, `quantization_params`, `setup_qm_params`), the § 5.18.7.8
`delta_q_params`, and the § 5.18.2 lossless / QM-index tail. For every model the writer
accepts, reparsing the written bits with the corresponding parser SHALL yield the original
(`parse(write(x)) == x`). The writers SHALL be additive (no model or parser-error change; only
`pub(crate)` visibility on parser helpers) and SHALL never panic: a model the parser could not
have produced SHALL be rejected with a typed writer error before any bit is written.

Where a value has more than one parser-reachable encoding (a zero `read_delta_q`, an
all-equal QM `qm_uv_same_as_y`, the `equal_ac_dc_q` chroma-DC, or a `qm_index` selecting a
repeated level), the writer MAY emit the canonical (shortest / smallest-index) encoding; the
round-trip is then semantic universally and byte-exact on the canonical subset.

#### Scenario: each quant structure round-trips across every branch

- **WHEN** a parsed quantization / QM-setup / delta-q / lossless structure is written with the
  same gating inputs and reparsed
- **THEN** the reparsed structure SHALL equal the original, across every conditional branch
  (the QM cascade, the `diff_uv_delta` / `equal_ac_dc_q` combinations, delta-q present/absent,
  lossless coded/has-segment, and `using_qmatrix` on/off).

#### Scenario: a non-reproducible quant model is rejected before any bit

- **WHEN** a model carries a value outside its descriptor domain (`base_q_idx`, `delta_q`
  `su(7)`, a `qm_*` `f(4)`, `pic_qm_num_minus_1` `f(2)`), an inferred field that disagrees with
  its gate, a lossless array that disagrees with the `get_qindex` re-derivation, or a
  `seg_qm_level` that no QM level reproduces
- **THEN** the writer SHALL return a typed `WriteError` and write no bit.
