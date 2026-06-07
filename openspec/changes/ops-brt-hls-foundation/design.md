# Design: Operating point set + buffer removal timing HLS foundation

## Parser design

Two focused modules in `splot-core`:

- `headers/operating_point_set.rs` reads § 5.10 `operating_point_set_obu()` and its
  § 5.11 `operating_point_payload()` children, dispatching to `ops_aggregate_info()`
  (§ 5.11.1), `ops_seq_profile_tier_level_info()` (§ 5.11.2), `ops_decoder_model_info()`
  (§ 5.11.3), `ops_color_info()` (§ 5.11.4), and `ops_mlayer_info()` (§ 5.11.5).
- `headers/buffer_removal_timing.rs` reads § 5.12 `buffer_removal_timing_obu()` in both
  the extended-layer (`br_time`) and OPS-dependent (`br_ops_id` / `br_ops_cnt` /
  per-op) forms.

The OPS header presence flags (`ops_intent_present_flag`, `ops_ptl_present_flag`,
`ops_color_info_present_flag`, and the global `ops_mlayer_info_idc`) are read once and
threaded into each `operating_point_payload()`. Each payload records `ops_data_size`
immediately, captures the bit position, parses the conditional fields and the per-layer
`OpsxLayerId` entries, byte-aligns, and computes `opsBytes` as the byte delta — keeping
both the declared and computed sizes so the validator can diagnose a mismatch.
Reserved-nonzero values are retained rather than rejected; only truncated or malformed
input produces a typed `Error`.

`OBU_OPERATING_POINT_SET` is extensible, so its dispatch finishes with the shared
`finish_obu_payload()` extensible tail (`obu_extension_flag` then `trailing_bits()`).
`OBU_BUFFER_REMOVAL_TIMING` is not extensible (`ObuType::is_extensible_obu` is `false`),
so it finishes with `trailing_bits()` only.

## Validator design

A dedicated `OpsAvailabilityStore`, separate from the monotonic `HlsAvailabilityStore`,
holds active OPS records keyed by `(obu_xlayer_id, ops_id)`. It is **not** monotonic:
§ 6.10.1 defines explicit reset/update behavior, so records are removed on reset.

| `ops_reset_flag` | `ops_cnt` | behavior |
|---|---|---|
| 1 | 0 | reset all OPS for the layer (all layers if global) |
| 1 | >0 | reset, then define this `(xlayer, ops_id)` |
| 0 | 0 | reset only this `(xlayer, ops_id)` |
| 0 | >0 | define/update only this `(xlayer, ops_id)` |

OPS local-semantic checks run against the *prior* store state (before the OBU is
applied) so cross-OPS inheritance references resolve correctly. A buffer-removal-timing
OBU resolves `(obu_xlayer_id, br_ops_id)`: an unavailable OPS under external-HLS-
disabled mode is `brt/unavailable-operating-point-set`, and a `br_ops_cnt` differing
from the active `ops_cnt` is `brt/ops-count-mismatch`. When external HLS is provided,
the hard missing-OPS error is suppressed so streams that rely on external OPS delivery
are not false-flagged.

## Ordering

§ 7.3.7 lists the global temporal-unit prefix OBUs exhaustively (MSDO, global LCR,
global OPS, global atlas, global metadata); `OBU_BUFFER_REMOVAL_TIMING` is not among
them. § 7.3.3 / § 7.3.4 place a BRT inside a coded output / non-output frame unit at
the frame's own `obu_xlayer_id`. So a **local** BRT is classified as a coded extended
layer OBU (it starts the coded-layer phase), and a **global** BRT is left unclassified
rather than flagged — a sound-over-complete choice that avoids false positives. The
hard `brt/global-ordering-position` diagnostic is deferred until decoder-model /
random-access state is modeled.

## Diagnostics

Stable rule IDs (documented in `docs/OPS-BRT-DIAGNOSTICS.md`):

- `ops/local-reserved-bits-nonzero` (§ 6.10.2)
- `ops/mlayer-info-idc-reserved` (§ 6.10.2)
- `ops/ptl-reserved-bits-nonzero` (§ 6.10.4)
- `ops/payload-size-mismatch` (§ 6.10.2)
- `ops/inherited-op-index-out-of-range` (§ 6.10.2)
- `brt/unavailable-operating-point-set` (§ 7.3.8.5)
- `brt/ops-count-mismatch` (§ 6.11)

## Boundaries

OPS/BRT parse coverage is complete, but semantic validation stays `partial`: Annex A/E
level and schedule conformance and the § 6.10.7 dependency-map agreement are not
implemented, so the matrix does not mark `validate = "done"`.
