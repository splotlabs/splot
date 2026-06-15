# Design: frame-header-writer-quantization

## Context

The quant cluster reads `base_q_idx`, the gated `read_delta_q()` deltas, the QM matrix-set
cascade, `delta_q_params()`, and the per-segment lossless / `qm_index` / `allow_tcq` /
`allow_parity_hiding` tail. The shared `read_delta_q()` and `get_qindex_ignore_delta_q()`
helpers already exist in the parser.

## Decisions

- **Additive — canonicalization, not a model extension.** The principled split from #4b / #4c:
  those surfaced bits that affect *layout / downstream parsing* (and were not recoverable). The
  quant read-but-not-stored points are **redundant encodings of a value the model fully
  retains**, so the writer picks the canonical encoding and round-trips semantically — exactly
  the sequence writer's leb128 / `num_ref_frames == 8` contract. No `QuantizationParams` /
  `SetupQmParams` / `LosslessInfo` field is added.
- **`qm_index` reverse-lookup over the full coded domain.** `parse_lossless_info` reads
  `qm_index` `f(CeilLog2(qmNum))` and indexes `levels[qm_index]` for *any* value in the field's
  `0 ..= 2^CeilLog2(qmNum) - 1` range (entries `>= qmNum` are the zeroed defaults). The writer
  searches that full coded domain — not just `0..qmNum` — so it faithfully inverts a stream
  that coded an index `>= qmNum`. The smallest matching index is the canonical choice; no match
  is a typed reject.
- **Re-derive the lossless state for reject-before-write.** `write_lossless_info` re-derives
  `LosslessArray` / `CodedLossless` / `HasLosslessSegment` via `get_qindex_ignore_delta_q`
  (exposed `pub(crate)`) and rejects a stored model that disagrees, rather than trusting the
  stored arrays. It also validates `qm.pic_qm_num_minus_1 < MAX_PIC_QM_NUM` up front (it drives
  the `qm_index` field width), so a non-canonical `qm` is rejected before any bit even when the
  writer is called without a prior `write_setup_qm_params`.
- **No panic on constructed models.** Every subtraction is in `i64`, every array index is
  `min`-bounded against `MAX_SEGMENTS` / `MAX_PIC_QM_NUM`, the `qm_index` reverse-lookup returns
  a typed reject on no-match, and all `su(7)` / `f(n)` domains (`su(7)` is two's-complement, so
  `[-64, 63]`) are validated before the write call.

## Testing

Round-trip via the public parsers across every branch (the QM cascade, the
`diff_uv_delta` / `equal_ac_dc_q` combinations, delta-q present/absent, lossless
coded/has-segment, `using_qmatrix` on/off, all four canonicalization edges). One reject test
per `WriteError` path (asserting `bit_len() == 0`), including the constructed-model edges
(`qm_index` no-match, out-of-range `su` / `f(4)`, the `pic_qm_num_minus_1` over-wide bound, a
hostile `max_segments`). A round-trip property test per parser.
