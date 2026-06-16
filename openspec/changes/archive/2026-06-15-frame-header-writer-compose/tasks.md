# Tasks

## Composing writer (additive)
- [x] `write/frame_header_core.rs`: `write_frame_header_core(writer, core, seq, mfh)` — the
      exact inverse of `parse_frame_header_core` on the `IntraHeaderComplete` path, writing the
      control-region glue bits directly and delegating each sub-structure to #4a–#4h.
- [x] Reject-before-write for the whole composition (a scratch `BitWriter` so a sub-structure
      reject mid-compose leaves the real writer untouched, `bit_len() == 0`); accept only
      `status == IntraHeaderComplete` + `frame_is_intra` + the required `Option`s + no
      `lr_params_partial`.
- [x] Invert `read_refresh_frame_flags` exactly (the KEY all-1s / short / full arms), and the
      frame-type / long-term-id / output-flag glue. Guard every width/shift (`ceil_log2`,
      `1 << frame_to_refresh`). Expose `CoreSeqView` / `MfhFrameView` for the signature. Register
      + re-export in `write/mod.rs`. No model field / `WriteError` variant added.

## Tests and proof
- [x] End-to-end parse → write → parse round-trips over the existing `IntraHeaderComplete`
      frame-header inputs/fixtures (single-picture Key, CLK, OLK, IntraOnly, lossless vs
      non-lossless, grain present/absent, multi-tile, cur_mfh_id 0 / >0), each byte-exact. One
      reject test per path (each non-`IntraHeaderComplete` status, a `None` required field,
      `lr_params_partial` set, a SEF/show-existing model), asserting `bit_len() == 0`. A
      round-trip proptest if feasible.

## Matrix and docs
- [x] Advanced `AV2-5.18.2-FRAME-HEADER-INFO` `write` `todo` → `partial` (modeled intra path
      done; the inter frame-header paths and the §5.18.7.11 frame-level Wiener bank remain
      unwritten, matching the sibling §5.18.3/§5.18.4/§5.18.7 writer rows' `partial`);
      `AV2-5.18-FRAME-HEADER` umbrella `write` stays `partial` (inter / SEF TODO). Regenerated
      `docs/FEATURE-STATUS.md`.

## Checks
- [x] `cargo xtask ci` and `openspec validate frame-header-writer-compose --strict`
