# Current validator state

`observed: 2026-06-09`
`repo: splotlabs/splot main`
`scope: validator/parser/inspector only`

## What is now implemented

The current repository is no longer just the original Annex B + OBU header scaffold. The validator coverage phase has advanced in several important ways:

| Area | Current state |
|---|---|
| Repository shape | Public Rust workspace with `.cargo`, `.github`, `crates`, `docs`, `fuzz`, `openspec`, `tests/fixtures`, and `xtask`. |
| README status | Pre-alpha / validator-first; Annex B envelope parser, AV2 OBU header parser, and structured header-conformance validator are present; decoder/encoder remain reserved shapes. |
| Matrix size | `docs/IMPLEMENTATION-MATRIX.toml` / generated feature status now track 114 features. |
| Descriptor foundation | `read_uvlc`, `read_le`, `read_ns`, `read_rg`, and `read_su` (§4.11.7) are mapped and tested in `splot-core/src/bitio.rs`. |
| OBU envelope/header | LEB128, OBU header, OBU type predicates, Annex B envelope, reserved OBU handling, trailing bits, byte alignment, and OBU dispatch skeleton are represented. |
| Payload dispatch | `open_bitstream_unit` dispatch is partial: temporal delimiter/trailing handling and `PayloadStatus`/inspect JSON are present, but most payload parsers remain unimplemented. |
| Sequence header | Umbrella row is partial. `AV2-5.4.1-SEQUENCE-HEADER-GENERAL` is done, and all §5.4.2–§5.4.13 child configs now parse — including `seg_info()` (§5.4.9) and the sequence `tile_params()` helper (§5.18.7.3) — so a valid sequence header parses in full and is payload-tail validated. The only residual bound is a reserved (non-conformant) `seq_level_idx` with tile info present. Local §6.4.1 and §6.17.7 tile diagnostics exist. |
| Segmentation / tiling foundation | `seg_info(numSegments)` (`splot-core/src/segment.rs`) and the reusable tile-partitioning helpers + `tile_params()` (`splot-core/src/tile.rs`) are implemented and shared by the sequence header and multi-frame header. Frame-level `segmentation_params()` / `tile_info()` live paths remain future work. |
| Stateful checks | Activated sequence limits and HLS availability are partial; sequence state exists but full §7.3.8 HLS availability is not complete. |
| Ordering | `AV2-7.3-OBU-ORDERING` and `AV2-7.3.7-TEMPORAL-UNIT-ORDER` are partial: initial temporal-unit ordering exists; frame-unit, metadata suffix, random-access, and long-term-reference ordering remain future work. |
| Conformance | Fuzz/proptest no-panic coverage exists. AVM differential harness, AVM parser traces, AVM valid/invalid stream proof, and public vector integration remain todo/pending. |

## Highest-leverage next phase

The next validator phase should **finish the sequence-header child coverage and strengthen HLS/temporal-unit state** before starting frame headers or tile groups.

Why this order:

1. §5.4 sequence header state drives many later parser branches and validator checks.
2. §6.2.2 activated sequence-header limits are only partial until sequence activation and HLS availability are modeled more completely.
3. §7.3.7 temporal-unit ordering and §7.3.8 HLS availability are currently partial and are prerequisites for meaningful frame/tile validation.
4. Frame header and tile group syntax depend on sequence-level dimensions, layer limits, tool flags, dependency maps, timing, and decoder-model fields.

## Do not start yet

Do not begin a full frame-header parser, tile-group payload parser, entropy/range coding, decoder, encoder, bitstream writer, or AVM differential harness as the primary task in this phase. Prepare hooks and fixtures, but keep the core implementation focused on sequence and HLS state.

## Immediate gaps to close

| Gap | Feature IDs |
|---|---|
| Sequence semantics | `AV2-6.4-SEQUENCE-HEADER-SEMANTICS` |
| Activated sequence state | `AV2-6.2.2-OBU-HEADER-ACTIVATED-SEQUENCE-LIMITS` |
| HLS availability | `AV2-7.3.8-HLS-AVAILABILITY` |
| Temporal-unit ordering completion | `AV2-7.3.7-TEMPORAL-UNIT-ORDER`, then children for §7.3.2 through §7.3.6 as parse dependencies allow |
| Frame segmentation/tiling live paths | `AV2-5.18.7-SEGMENTATION-TILING` (frame `segmentation_params()` / `tile_info()`), built on the now-implemented `AV2-5.4.9-SEGMENT-INFO` and `AV2-5.18.7.3-TILE-PARAMS` foundations |
| HLS OBU payload foundations | `AV2-5.5-TEMPORAL-DELIMITER`, `AV2-5.6-MSDO`, and `AV2-5.7-MULTI-FRAME-HEADER` (including MFH `seg_info()`), plus the §5.8 layer configuration records (`AV2-5.8.1-LCR-GLOBAL-INFO` through `AV2-5.8.9-LCR-XLAYER-COLOR-INFO`), atlas segments (`AV2-5.9-ATLAS-SEGMENT`), operating point sets (`AV2-5.10-OPERATING-POINT-SET`, `AV2-5.11-OPERATING-POINT-PAYLOAD`), and buffer-removal timing (`AV2-5.12-BUFFER-REMOVAL-TIMING`) are now fully parsed with stable diagnostics. Remaining HLS work is deeper semantic validation (these rows are `validate = partial`) and the deferred MFH/OPS layer-dependency-map checks (§6.10.7) |

## Current-state check commands for agents

Run these before any code edit:

```bash
git status --short
cargo xtask feature-status --format table
cargo xtask spec-coverage
cargo xtask ci
```

When the matrix and generated status disagree, `docs/IMPLEMENTATION-MATRIX.toml` is canonical and `docs/FEATURE-STATUS.md` must be regenerated.
