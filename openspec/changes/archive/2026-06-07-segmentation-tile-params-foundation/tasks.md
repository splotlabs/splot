# Tasks: segmentation and tile-params foundation

## 1. OpenSpec and matrix setup

- [x] Add this OpenSpec change and validate it with `openspec validate segmentation-tile-params-foundation --strict`.
- [x] Update `docs/IMPLEMENTATION-MATRIX.toml` before code for the affected Feature IDs.
- [x] Add `AV2-4.11.7-SU` and `AV2-5.18.7.3-TILE-PARAMS` rows.
- [x] Add `tile-params/` to the xtask diagnostic allowlist and the diagnostics registry.

## 2. `su(n)` descriptor (`splot-core`)

- [x] Add `BitReader::read_su(width) -> Result<i32>` per §4.11.7.
- [x] Reject `width == 0` and `width > 32` with a structured error (no panic).
- [x] Unit tests: `su(1)` 0 and -1, positive/negative multi-bit, EOF, invalid width.

## 3. `seg_info(numSegments)` (`splot-core`)

- [x] Add `segment.rs` with `MAX_SEGMENTS`, `SEG_LVL_MAX`, `SegmentFeature`, `SegmentInfo`.
- [x] Add the exact `Segmentation_Feature_Bits` / `_Signed` / `_Max` tables (§5.4.9).
- [x] Implement `parse_seg_info(reader, num_segments)` (signed `su`, clipping, zero defaults).
- [x] Unit tests: all-disabled 8- and 16-segment, a signed quantizer feature path, EOF, no panic.

## 4. Wire `seg_info()` into sequence and MFH

- [x] `SequenceSegmentConfig` carries `SegmentInfo` when present; remove the bounded hole.
- [x] `MultiFrameHeader` carries `SegmentInfo` when present; remove the `unimplemented_at` for seg_info.
- [x] Update sequence/MFH/dispatch tests that expected bounded segment info.

## 5. Tile params foundation (`splot-core`)

- [x] Add `tile.rs` with `tile_log2`, `uniform_spacing`, and the block-size / scaling tables.
- [x] Implement `parse_tile_params(reader, input)` (uniform and non-uniform paths).
- [x] Wire `SequenceTileConfig { seq_tile_info_present_flag, allow_tile_info_change, params }`.
- [x] Pass profile/tier/level + frame dims + seqSbSize from the parsed header.
- [x] Unit tests: `tile_log2`, `uniform_spacing`, absent, uniform present, non-uniform present, no panic.

## 6. Validator and diagnostics

- [x] Promote fully parsed sequence headers / MFHs to payload-tail validation (existing gates).
- [x] Emit `tile-params/tile-cols-out-of-range` / `tile-params/tile-rows-out-of-range`.
- [x] Emit `tile-params/nonuniform-cols-do-not-cover-frame` / `...rows...`.
- [x] Validator tests: malformed tail after segment info diagnosed; non-uniform >64 tiles flagged; coverage check.
- [x] Keep existing frame-header activation / HLS tests passing.

## 7. CLI and fixtures

- [x] Repurpose the sequence-tile fixture to a fully parsed tile config and update the CLI test + README.

## 8. Docs and proof

- [x] Update `docs/VALIDATOR-DIAGNOSTICS.md` with the `tile-params/` diagnostics.
- [x] Update `docs/CURRENT-VALIDATOR-STATE.md`.
- [x] Regenerate `docs/FEATURE-STATUS.md` via `cargo xtask feature-status --format markdown --output docs/FEATURE-STATUS.md`.
- [x] Update any roadmap note that still calls `seg_info()` / sequence `tile_params()` future.
- [x] Run `cargo fmt --all -- --check`.
- [x] Run `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`.
- [x] Run `cargo test --workspace --all-targets --locked`.
- [x] Run `cargo xtask check-feature-status`.
- [x] Run `cargo xtask spec-coverage`.
- [x] Run `cargo xtask ci`.
- [x] Run `openspec validate segmentation-tile-params-foundation --strict`.
