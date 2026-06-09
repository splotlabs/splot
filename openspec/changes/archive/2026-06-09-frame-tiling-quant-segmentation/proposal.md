# Change proposal: frame header tiling, quantization, and segmentation

## Why

The frame-header core foundation (`frame-header-core-foundation`, archived) parses the
intra-frame path of `frame_header_info()` through `disable_cdf_update` and stops with
`FrameHeaderParseStatus::StoppedBeforeFilteringQuantSegmentation`. The next contiguous
structures in exact AV2 v1.0.0 § 5.18.2 call order are `tile_info()` (§ 5.18.7.2),
`quantization_params()` (§ 5.18.6.1), `segmentation_params()` (§ 5.18.7.1),
`setup_qm_params()` (§ 5.18.6.2), `delta_q_params()` (§ 5.18.7.8), and the derived
lossless/QM-level reads (`qm_index`, `allow_tcq`, `allow_parity_hiding`). Parsing them
is the validator-roadmap Phase 8 continuation: it advances
`AV2-5.18.6-QUANTIZATION` (currently all-`todo`) and
`AV2-5.18.7-SEGMENTATION-TILING` (currently `partial`), builds directly on the
already-`done` § 5.18.7.3 tile-params helper (`AV2-5.18.7.3-TILE-PARAMS`,
`crates/splot-core/src/tile.rs`), and unblocks per-frame tile layout state needed by
later tile-group payload validation (Phase 9).

## What Changes

- Extend the intra-path frame-header parser (`crates/splot-core/src/headers/frame/`)
  past `disable_cdf_update`, in spec order, with new submodules for:
  - `tile_info()` — AV2 v1.0.0 § 5.18.7.2, reusing the § 5.18.7.3 `tile_params()`
    helper already in `crates/splot-core/src/tile.rs`;
  - `quantization_params()` and `read_delta_q()` — § 5.18.6.1 / § 5.18.6.3;
  - `segmentation_params()` — § 5.18.7.1;
  - `setup_qm_params()` — § 5.18.6.2;
  - `delta_q_params()` (quantizer index delta parameters) — § 5.18.7.8;
  - the per-segment lossless/QM derivation loop (`LosslessArray`, `CodedLossless`,
    `qm_index`), `allow_tcq`, and `allow_parity_hiding` reads from § 5.18.2.
- Move the intra-path stop point forward: replace the
  `StoppedBeforeFilteringQuantSegmentation` terminal status with a new explicit status
  (stopped before `deblocking_filter_params()`, § 5.18.5.2). Deeper structures
  (filtering, CDEF/LR/CCSO/GDF, transform modes, global motion, film grain) stay out.
- Add structured validator diagnostics for the locally-decidable § 6.17.6 /
  § 6.17.7.1 / § 6.17.7.2 / § 6.17.7.4 semantics constraints (with stable `rule_id`,
  `severity`, `spec_section`, byte offsets), registered in
  `docs/VALIDATOR-DIAGNOSTICS.md` (enforced by `cargo xtask check-diagnostic-registry`).
- Surface the new parsed fields in the `splot inspect` JSON frame-header summary.
- Update `docs/IMPLEMENTATION-MATRIX.toml` rows honestly (`AV2-5.18.6-QUANTIZATION`,
  `AV2-5.18.7-SEGMENTATION-TILING`, umbrella `AV2-5.18-FRAME-HEADER` /
  `AV2-5.18.1-FRAME-HEADER-GENERAL` / `AV2-5.18.2-FRAME-HEADER-INFO` stay `partial`)
  and regenerate `docs/FEATURE-STATUS.md`.

Feature IDs: `AV2-5.18.6-QUANTIZATION`, `AV2-5.18.7-SEGMENTATION-TILING`
(supporting: `AV2-5.18.7.3-TILE-PARAMS`, `AV2-5.18.2-FRAME-HEADER-INFO`,
`AV2-5.18-FRAME-HEADER`).

## Capabilities

### New Capabilities

(none — this extends existing capabilities)

### Modified Capabilities

- `bitstream`: parse the § 5.18.6 quantization structures and § 5.18.7.1/§ 5.18.7.2/
  § 5.18.7.8 segmentation-and-tiling structures on the intra frame-header path,
  with typed EOF/malformed errors and no panics.
- `validator`: emit locally-decidable § 6.17.6 / § 6.17.7 diagnostics for
  quantization, segmentation, and tile-info fields, and report the new frame-header
  parse stop point honestly.

## Impact

- `crates/splot-core/src/headers/frame/` (new `tiling.rs` / `quant.rs` /
  `segmentation.rs` submodules; `info.rs` tail and `FrameHeaderParseStatus` change),
  `crates/splot-core/src/tile.rs` (reuse, possible small API additions).
- `crates/splot-validate/src/` new diagnostics + context wiring for sequence-derived
  inputs (superblock size, `pic_qm_num_minus_1`, segmentation/quantization sequence
  flags).
- `crates/splot-cli` inspect JSON output (snapshot updates).
- `fuzz/fuzz_targets/parse_obu.rs` reaches the new code automatically.
- Docs: `docs/IMPLEMENTATION-MATRIX.toml`, `docs/FEATURE-STATUS.md`,
  `docs/SPEC-MAPPING.md`, `docs/VALIDATOR-DIAGNOSTICS.md`, `docs/VALIDATOR-ROADMAP.md`
  Phase 8 status note.
- No new dependencies; no crate-graph changes; no encoder/writer changes.

## Non-goals

- Filtering structures (§ 5.18.5), GDF/CDEF/LR/CCSO params (§ 5.18.7.9–§ 5.18.7.12).
- Transform/coding modes (§ 5.18.8), global motion (§ 5.18.9), film grain (§ 5.18.10).
- Inter/switch/TIP/RAS/bridge frame paths beyond their existing stop points.
- `frame_header_copy()` bit identity, tile-group payload (§ 5.20), entropy decoding.
- Decoder-state-dependent semantics (e.g. checks needing reconstructed frames).
- Encoder/writer work and AVM differential harness changes.
