# Design: frame-header-writer-compose

## Context

`parse_frame_header_core` (info.rs) parses the § 5.18.2 frame header. On the intra path it
reaches `FrameHeaderParseStatus::IntraHeaderComplete` after the activation prefix, a
frame-type-dependent control region (output flags, `order_hint`, `refresh_frame_flags`, the
long-term-id reads), `frame_size()` / `screen_content_params()` / `intrabc_params()`,
`disable_cdf_update`, the structure cluster (`tile_info` → `quantization_params` →
`segmentation_params` → `setup_qm_params` → `delta_q_params` → lossless), the loop-filter
cluster (deblocking / GDF / CDEF / LR / CCSO), and the § 5.18.2 tail. The sub-writers for every
one of those sub-structures already exist (#4a–#4h); the control region between the prefix and
`frame_size()`, plus `disable_cdf_update` and `refresh_frame_flags`, are the "glue" with no
existing writer.

## Decisions

- **Compose, don't re-derive.** The capstone writer writes the glue bits directly and delegates
  each sub-structure to its existing sub-writer with the exact gating inputs the parser used
  (reconstructed from `FrameHeaderCore` + the `CoreSeqView` / `MfhFrameView`). The
  `coded_lossless` that gates the filter cluster and `read_tx_mode()` comes from
  `core.lossless_info`.
- **Scratch-writer reject-before-write.** The composition writes many sub-structures in
  sequence; a sub-writer that rejects mid-compose would otherwise leave the real `writer` with a
  partial buffer. To guarantee `bit_len() == 0` on any reject without duplicating every
  sub-writer's validation, the whole header is written to a local scratch `BitWriter`; only on
  full success are its bits appended to the caller's `writer`. A top-level check still rejects
  the obvious non-reproducible models (wrong status, missing `Option`s, `lr_params_partial`)
  up front.
- **`IntraHeaderComplete`-only.** The writer accepts only a model that parsed to
  `IntraHeaderComplete`: `status`, `frame_is_intra == Some(true)`, `show_existing_frame !=
  Some(true)`, every intra-path `Option` present, and `lr_params_partial == None` (a partial LR
  parse means `StoppedBeforeWienerNsFilter`, not a complete header). Inter / switch / TIP /
  bridge / SEF models reach other statuses and are rejected.
- **`refresh_frame_flags` is the trickiest glue.** It inverts `read_refresh_frame_flags`: a KEY
  closed-loop-no-mlayer frame infers all-1s (no bit); `enable_short_refresh_frame_flags` codes a
  single-bit `frame_to_refresh` f(ceil_log2(NumRefFrames)) (so a `refresh_frame_flags` that is
  not a single set bit, or whose bit index is out of range, is rejected); otherwise the full
  `refresh_frame_flags` f(NumRefFrames). The chosen arm follows the model's `frame_type` + the
  seq flags, exactly as the parser selects it.
- **No panic on constructed models.** Every derived width (`ceil_log2(num_ref_frames)`,
  `order_hint_bits`, `long_term_frame_id_bits`, the frame-size widths) and every shift
  (`1 << frame_to_refresh`, guarded by `frame_to_refresh < num_ref_frames`) is range-checked
  before use; the sub-writers are already panic-safe.

## Testing

The strongest proof is an end-to-end **parse → write → parse** round-trip: the existing
`IntraHeaderComplete` frame-header inputs/fixtures are parsed to a `FrameHeaderCore`, written
back with `write_frame_header_core`, and asserted byte-exact (and reparsed equal). Coverage spans
single-picture Key, CLK, OLK, IntraOnly, lossless vs non-lossless, grain present/absent,
multi-tile, and `cur_mfh_id` 0 / >0. One reject test per path (each non-`IntraHeaderComplete`
status, a `None` required field, `lr_params_partial` set, a SEF/show-existing model), each
asserting `bit_len() == 0`, plus a round-trip proptest where feasible.
