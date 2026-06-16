# Tasks: fix the single-picture bridge frame-header parse path

## 1. Bookkeeping

- [x] 1.1 Confirm the matrix row id (`AV2-5.18.2-FRAME-HEADER-INFO`) and set
  `openspec_change`. Re-read § 5.18.2 verbatim (mirror `#s-5-18-2`, :4117-5093) for
  the single-picture + `IsBridge` interaction, plus § 5.18.4.1 `frame_size()`,
  § 5.18.3.3 `screen_content_params()`, and § 5.18.3.4 `intrabc_params()`.
- [x] 1.2 Triangulate the exact bit-read order against the spec mirror (normative),
  AVM (`av2/decoder/decodeframe.c`), and dav2d (`src/obu.c`); record the two
  spec-vs-AVM divergences (overwrite-gated refresh; `bridge_frame_max_width/height`
  frame-size fields) and the dav2d non-implementation.

## 2. Parsing

- [x] 2.1 Route a single-picture `IsBridge` frame in `parse_core_body` to a new
  `parse_single_picture_bridge_tail` (not `parse_intra_tail`), keyed on
  `core.is_bridge`.
- [x] 2.2 Read the spec-mirror prefix: `bridge_frame_overwrite_flag` f(1) (:4423);
  `read_refresh_frame_flags(..., FrameType::Key)` (:4445); non-override
  `parse_frame_size` (:4567, default dims, no bits); `parse_screen_content_params_full`
  (:4569); `parse_intrabc_params_full(frame_is_intra = true)` (:4571).
- [x] 2.3 Record `num_total_refs = 0` (:4573), `tip_frame_mode = TIP_FRAME_DISABLED`
  (:4575), and `primary_ref_frame = PRIMARY_REF_NONE` (:4345) on `core.inter`, then
  stop with `InterStop::BruInactiveOrBridgeReturn` (:4971) via `finish_inter_control`
  (status `UnsupportedUntilFeature`; EOF → `StoppedInsideInterControl`). Do NOT read
  `disable_cdf_update` or the structure cluster.

## 3. Tests and proof

- [x] 3.1 Replace `frame_header_core_single_picture_bridge_takes_intra_key_path` with
  `frame_header_core_single_picture_bridge_reads_prefix_then_bridge_return`
  (asserts the stop, the skipped `disable_cdf_update`/cluster, and the recorded
  prefix facts).
- [x] 3.2 Add `frame_header_core_single_picture_bridge_reads_scc_and_intrabc_conditionals`
  (data-dependent reads + `overwrite == 1` still reads `refresh_frame_flags`
  unconditionally) and `frame_header_core_single_picture_bridge_eof_in_prefix_is_truncation`
  (codex-F2 preservation → `StoppedInsideInterControl`).
- [x] 3.3 Confirm no downstream (`splot-validate` / `splot-cli` / `splot-decode`)
  regression from the `IntraHeaderComplete` → `UnsupportedUntilFeature` change.

## 4. Matrix and docs

- [x] 4.1 Update the `AV2-5.18.2-FRAME-HEADER-INFO` notes (single-picture-bridge fix
  + the spec-vs-AVM/dav2d divergence) and add the three new tests to `feature.proof`.
  Bumped the `info.rs` source-line allowance (4624 -> 4889) with a dated rationale.
- [ ] 4.2 Re-record the audit ledger (`cargo xtask audit-scope --all --write-ledger`)
  as a POST-MERGE step — the changed files should read as legitimate drift until merged.

## 5. Checks

- [x] 5.1 `openspec validate frame-header-single-picture-bridge-fix --strict` (valid).
- [x] 5.2 `cargo xtask ci` green (toolchain `1.96.0-aarch64-apple-darwin`).
