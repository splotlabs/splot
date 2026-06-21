# Design: decode-inter-header-shared-tail

## Context

The § 5.18.2 shared structure cluster (mirror :5183-5341) is read identically by
the intra and inter paths: `tile_info()` → `quantization_params()` →
`segmentation_params()` → `setup_qm_params()` → `delta_q_params()` → the per-segment
lossless / `allow_tcq` / `allow_parity_hiding` derivation → the loop-filter cluster
(`deblocking_filter_params()` / `gdf_params()` / `cdef_params()` / `lr_params()` /
`ccso_params()`) → the § 5.18.2 tail (`read_tx_mode()` / `frame_reference_mode()` /
`skip_mode_params()` / `allow_bawp` / `allow_warpmv_mode` / `reduced_tx_set` /
`global_motion_params()` / `film_grain_config()`). The intra path already parses all
of this (`info.rs::parse_intra_structures` / `parse_filter_cluster` /
`parse_intra_tail_structures`) via reusable `pub` sub-parsers.

## Decision: reuse the intra sub-parsers, gate the inter-specific arms

A separate `inter_shared_tail.rs` module orchestrates the same `pub` sub-parsers
with the inter inputs, rather than duplicating them or growing `info.rs` (already at
its hard-cap allowance). It keeps `info.rs` the owner of `FrameHeaderCore` and the
intra orchestration; the inter orchestration lives in the new module.

The sub-parsers are `FrameIsIntra`-arm-independent EXCEPT:

| structure | inter-specific arm | gate |
| --- | --- | --- |
| `deblocking_filter_params()` | `allow_df_sub_pu` f(1) (mirror :5935) | reads it (new `read_allow_df_sub_pu` param) |
| `segmentation_params()` | `segmentation_update_map` / `temporal_update` (mirror :6337) | read `segmentation_enabled` only; stop if 1 |
| `lr_params()` | temporal-prediction arm (mirror :7377, `numRefFrames > 0`) | admission gate: stop if `enable_restoration && NumTotalRefs > 0` |
| `ccso_params()` | `reuse_ccso` / `sb_reuse_ccso` / `ccso_ref_idx` (mirror :7491) | admission gate: stop if `enable_ccso` |
| `frame_reference_mode()` | `reference_select` f(1) (mirror :7747) | reads it inline |
| `skip_mode_params()` | `skip_mode_present` f(1) (mirror :7717) | reads it inline (skipModeAllowed for non-switch inter) |
| `global_motion_params()` | `use_global_motion` + warp models (mirror :7798) | reused `parse_global_motion_params` inter arm + its honest stops |
| `allow_bawp` / `allow_warpmv_mode` | f(1) (mirror :5313 / :5327) | gated reads |

`tile_info` / `quantization_params` / `setup_qm_params` / `delta_q_params` /
`parse_lossless_info` / `gdf_params` / `cdef_params` / `read_tx_mode` /
`film_grain_config` have no inter-specific arm and are reused as-is.

## Decision: admission gate BEFORE any shared-tail bit

`lr_params()` / `ccso_params()` are mid-cluster, but their inter arms are unmodeled.
Rather than reuse the intra parser unsoundly when those arms could fire, the parser
checks the admission condition (`enable_restoration && NumTotalRefs > 0` ||
`enable_ccso`) at the TOP of `parse_inter_shared_tail` and stops honestly with
`UnsupportedUntilFeature` before reading any shared-tail bit. This guarantees the
parser never exposes a possibly-mis-positioned `setup_qm` / `using_qmatrix` etc. to
downstream validator checks — the "tighten admission to the verified subset"
discipline (reject → never confident-wrong). The verified `syn-2frame-inter-64x64`
fixture has both off and completes; the richer `syn-key-inter-64x64` inter frames
(CCSO on) stop at the gate, preserving that fixture's `clean` verdict.

## Decision: frame size lifted before the tail

The shared tail's `tile_info()` needs the reference-grounded `FrameWidth`/`Height`
the control region resolved on `control.frame_size`. `parse_inter_path` lifts it
onto `core.frame_size` BEFORE invoking the shared tail (the refresh-flags /
`disable_cdf_update` lift stays in `finish_inter_control_with_tail`). When the size
is genuinely unknown (a hit on an unmodeled ref slot) it stays `None` and the shared
tail stops at its own frame-size guard.

## Honesty boundary

- Verification is the bit-level parse unit test on the real fixture (the parse
  consumes EXACTLY the shared-tail field sequence and reaches `InterHeaderComplete`;
  the 56-bit minimal inter payload = 27-bit control region + 21-bit shared tail +
  8-bit §5.19 tile-group tail, hand-decoded against the spec and confirmed by the
  fixture's bit-exact avmdec/dav2d decode).
- NO decode-output change: the runtime still rejects the inter frame at §5.20
  mode_info. This brick is the frame-header parse only.
- Every honest stop is a coverage stop (`UnsupportedUntilFeature`), never a
  truncation; an EOF inside the modeled tail is `StoppedInsideInterControl`.
