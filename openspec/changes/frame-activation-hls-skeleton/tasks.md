# Tasks: frame activation HLS skeleton

## 1. OpenSpec and matrix setup

- [x] Add this OpenSpec change and validate it with `openspec validate frame-activation-hls-skeleton --strict`.
- [x] Update `docs/IMPLEMENTATION-MATRIX.toml` before code for the affected Feature IDs.
- [x] Add `frame-header/` and `tile-group/` to the xtask diagnostic allowlist and the diagnostics registry.

## 2. Core prefix parsers (`splot-core`)

- [x] Add `FrameHeaderPrefix` and `FrameHeaderPrefixStatus` types in `headers/frame.rs`.
- [x] Add a prefix parser for `frame_header_info()` activation fields.
- [x] Extract `cur_mfh_id` (`uvlc`, inferred 0 for bridge frames).
- [x] Extract `seq_header_id_in_frame_header` when `cur_mfh_id == 0`.
- [x] Add `MfhId` newtype and a `MultiFrameHeaderRecord` availability record.
- [x] Add `TileGroupHeaderPrefix` for §5.19 `is_first_tile_group` / `frame_header_present_flag`.
- [x] Expose consumed bits and a prefix-only status so the parse cannot be mistaken for a full §5.18/§5.19 parse.

## 3. HLS availability store (`splot-validate`)

- [x] Extend `HlsAvailabilityStore` with in-band MFH records keyed by `mfh_id`.
- [x] Record MFH availability after a valid `multi_frame_header_obu()` parse.
- [x] Preserve the existing `mfh/sequence-header-unavailable` check.
- [x] Add frame-header `cur_mfh_id` availability + range diagnostics.
- [x] Add frame-header `seq_header_id_in_frame_header` availability + range diagnostics.
- [x] Preserve default-disabled external HLS behavior.

## 4. Sequence activation and CVS scoping (`splot-validate`)

- [x] Use parsed frame-header references to activate the sequence header for parsed CLK/OLK paths.
- [x] Store the latest well-formed sequence header per id so a reconfiguration is used for layer limits.
- [x] Reset CVS-scoped fingerprints / content-interpretation records at the global temporal delimiter so an in-CVS non-identical repeat after the activating CLK is caught.
- [x] Document the remaining cross-temporal-unit-within-CVS false-negative bound.

## 5. Tests

- [x] Parser unit test: frame header prefix with `cur_mfh_id == 0` and `seq_header_id_in_frame_header`.
- [x] Parser unit test: frame header prefix with `cur_mfh_id > 0`.
- [x] Parser unit test: first tile group reaches the frame-header prefix.
- [x] Parser unit test: EOF before `cur_mfh_id` is a structured error.
- [x] Validator test: missing frame-header sequence header emits `hls/unavailable-sequence-header`.
- [x] Validator test: available in-band / external sequence header is accepted.
- [x] Validator test: missing MFH emits `hls/unavailable-multi-frame-header`; available MFH is accepted.
- [x] Validator test: CLK-driven activation updates active layer limits.
- [x] Validator test: in-CVS non-identical repeated sequence header remains flagged.

## 6. Docs and proof

- [x] Update `docs/FEATURE-STATUS.md` via `cargo xtask feature-status --format markdown`.
- [x] Update `STATUS.md` with implemented work, deferred full-frame work, diagnostics, and command output.
- [x] Add a note to `docs/VALIDATOR-ROADMAP.md` that frame activation skeleton precedes full frame header / tile payload.
- [x] Run `cargo fmt --all -- --check`.
- [x] Run `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`.
- [x] Run `cargo test --workspace --all-targets --locked`.
- [x] Run `cargo xtask check-feature-status`.
- [x] Run `cargo xtask spec-coverage`.
- [x] Run `cargo xtask ci`.
- [x] Run `openspec validate frame-activation-hls-skeleton --strict`.
