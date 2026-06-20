# Writer coverage

Generated from `docs/IMPLEMENTATION-MATRIX.toml` by `cargo xtask writer-coverage --format markdown --output docs/spec-coverage-writer.md`. Do not edit by hand.

The AV2 bitstream **writer** (`splot-core::write`) surface: one row per writable `splot-core` syntax feature, plus every other `splot-core` feature with a landed writer, with its `write` maturity (writers in other crates, e.g. the `splot-recon` Y4M output, are out of scope). The writer is the inverse of the parser — `parse(write(parse(x))) == parse(x)` — byte-exact on the canonical subset and semantic (round-trip on the parsed model) for the canonicalizing writers (e.g. film grain and quantizer matrix, whose model is lossy versus the wire). The canonical status source is [IMPLEMENTATION-MATRIX.toml](./IMPLEMENTATION-MATRIX.toml); each row's per-writer round-trip details live in its matrix notes, and the full per-feature ledger is [FEATURE-STATUS.md](./FEATURE-STATUS.md).

Matrix version 1. Last reviewed 2026-06-15. 93 writable feature(s).

`write` legend: `done` written and round-trip-proven, `partial` in progress, `todo` not written yet, `pending` waiting on external proof, `blocked` blocked, `exp` experimental.

| Write status | Features |
|---|---:|
| `done` | 80 |
| `partial` | 13 |

| Section | Feature | Name | Write | Module |
|---|---|---|:-:|---|
| 4.11.3 | `AV2-4.11.3-UVLC` | Unsigned variable-length code descriptor | done | `crates/splot-core/src/bitio.rs` |
| 4.11.4 | `AV2-4.11.4-SVLC` | Signed variable-length code descriptor | done | `crates/splot-core/src/bitio.rs` |
| 4.11.5 | `AV2-4.11.5-LE` | Little-endian fixed-width descriptor | done | `crates/splot-core/src/bitio.rs` |
| 4.11.6 | `AV2-4.11.6-LEB128` | LEB128 descriptor | done | `crates/splot-core/src/leb128.rs` |
| 4.11.7 | `AV2-4.11.7-SU` | Signed integer descriptor | done | `crates/splot-core/src/bitio.rs` |
| 4.11.8 | `AV2-4.11.8-NS` | Non-symmetric integer descriptor | done | `crates/splot-core/src/bitio.rs` |
| 4.11.10 | `AV2-4.11.10-RG` | Rice-Golomb descriptor | done | `crates/splot-core/src/bitio.rs` |
| 5.2.1 | `AV2-5.2.1-OBU-DISPATCH` | open_bitstream_unit payload dispatch | partial | `crates/splot-core/src/obu.rs` |
| 5.2.2, 6.2.2 | `AV2-5.2.2-OBU-HEADER` | OBU header syntax | done | `crates/splot-core/src/obu.rs` |
| 5.2.3, 6.2.3 | `AV2-5.2.3-TRAILING-BITS` | Trailing bits syntax and semantics | done | `crates/splot-core/src/obu.rs` |
| 5.2.4, 6.2.4 | `AV2-5.2.4-BYTE-ALIGNMENT` | Byte alignment syntax and semantics | done | `crates/splot-core/src/bitio.rs` |
| 5.4, 6.4 | `AV2-5.4-SEQUENCE-HEADER` | Sequence header OBU syntax | done | `crates/splot-core/src/headers.rs` |
| 5.4.1, 6.4.1 | `AV2-5.4.1-SEQUENCE-HEADER-GENERAL` | General sequence header syntax | done | `crates/splot-core/src/headers/sequence.rs` |
| 5.4.1 | `ENC-WRITER-INPUT-SEQ-VIEW` | Encoder writer-input minimal-intra CoreSeqView constructor | done | `crates/splot-core/src/headers/frame/encoder_input.rs` |
| 5.4.2, 6.4.2 | `AV2-5.4.2-SEQUENCE-TILE-CONFIG` | Sequence tile configuration syntax | done | `crates/splot-core/src/headers/sequence.rs` |
| 5.4.3, 6.4.3 | `AV2-5.4.3-SEQUENCE-PARTITION-CONFIG` | Sequence partition configuration syntax | done | `crates/splot-core/src/headers/sequence.rs` |
| 5.4.4, 6.4.4 | `AV2-5.4.4-SEQUENCE-SEGMENT-CONFIG` | Sequence segment configuration syntax | done | `crates/splot-core/src/headers/sequence.rs` |
| 5.4.5, 6.4.5 | `AV2-5.4.5-SEQUENCE-INTRA-CONFIG` | Sequence intra configuration syntax | done | `crates/splot-core/src/headers/sequence.rs` |
| 5.4.6, 6.4.6 | `AV2-5.4.6-SEQUENCE-INTER-CONFIG` | Sequence inter configuration syntax | done | `crates/splot-core/src/headers/sequence.rs` |
| 5.4.6 | `ENC-WRITER-INPUT-INTER-VIEW` | Encoder writer-input minimal-intra inter view constructor | done | `crates/splot-core/src/headers/frame/encoder_input.rs` |
| 5.4.7, 6.4.7 | `AV2-5.4.7-SEQUENCE-SCC-CONFIG` | Sequence screen-content-coding configuration syntax | done | `crates/splot-core/src/headers/sequence.rs` |
| 5.4.8, 6.4.8 | `AV2-5.4.8-SEQUENCE-TQ-ENTROPY-CONFIG` | Sequence transform, quantization, and entropy configuration syntax | done | `crates/splot-core/src/headers/sequence.rs` |
| 5.4.9, 6.4.9 | `AV2-5.4.9-SEGMENT-INFO` | Segment info syntax | done | `crates/splot-core/src/segment.rs` |
| 5.4.10, 6.4.10 | `AV2-5.4.10-SEQUENCE-FILTER-CONFIG` | Sequence filter configuration syntax | done | `crates/splot-core/src/headers/sequence.rs` |
| 5.4.11, 6.4.11 | `AV2-5.4.11-USER-QM` | Sequence user quantization matrix syntax | done | `crates/splot-core/src/headers/quantizer_matrix.rs` |
| 5.4.12, 6.4.12 | `AV2-5.4.12-TIMING-INFO` | Sequence timing info syntax | done | `crates/splot-core/src/headers/sequence.rs` |
| 5.4.13, 6.4.13 | `AV2-5.4.13-SEQUENCE-DECODER-MODEL-INFO` | Sequence decoder model info syntax | done | `crates/splot-core/src/headers/sequence.rs` |
| 5.5, 6.5 | `AV2-5.5-TEMPORAL-DELIMITER` | Temporal delimiter OBU syntax | done | `crates/splot-core/src/obu.rs` |
| 5.6, 6.6 | `AV2-5.6-MSDO` | Multistream decoder operation OBU syntax | done | `crates/splot-core/src/hls.rs` |
| 5.7, 6.7 | `AV2-5.7-MULTI-FRAME-HEADER` | Multi-frame header OBU syntax | done | `crates/splot-core/src/hls.rs` |
| 5.8, 6.8 | `AV2-5.8-LAYER-CONFIG-RECORD` | Layer configuration record OBU syntax | done | `crates/splot-core/src/headers/layer_config_record.rs` |
| 5.8.1, 6.8.2 | `AV2-5.8.1-LCR-GLOBAL-INFO` | LCR global info syntax | done | `crates/splot-core/src/headers/layer_config_record.rs` |
| 5.8.2, 6.8.3 | `AV2-5.8.2-LCR-LOCAL-INFO` | LCR local info syntax | done | `crates/splot-core/src/headers/layer_config_record.rs` |
| 5.8.3, 6.8.4 | `AV2-5.8.3-LCR-AGGREGATE-INFO` | LCR aggregate info syntax | done | `crates/splot-core/src/headers/layer_config_record.rs` |
| 5.8.4, 6.8.5 | `AV2-5.8.4-LCR-SEQ-PTL-INFO` | LCR sequence profile tier level information syntax | done | `crates/splot-core/src/headers/layer_config_record.rs` |
| 5.8.5, 6.8.6 | `AV2-5.8.5-LCR-GLOBAL-PAYLOAD` | LCR global payload syntax | done | `crates/splot-core/src/headers/layer_config_record.rs` |
| 5.8.6, 6.8.7 | `AV2-5.8.6-LCR-XLAYER-INFO` | LCR xlayer info syntax | done | `crates/splot-core/src/headers/layer_config_record.rs` |
| 5.8.7, 6.8.8 | `AV2-5.8.7-LCR-REP-INFO` | LCR representation info syntax | done | `crates/splot-core/src/headers/layer_config_record.rs` |
| 5.8.8, 6.8.9 | `AV2-5.8.8-LCR-EMBEDDED-LAYER-INFO` | LCR embedded layer info syntax | done | `crates/splot-core/src/headers/layer_config_record.rs` |
| 5.8.9, 6.8.10 | `AV2-5.8.9-LCR-XLAYER-COLOR-INFO` | LCR xlayer color info syntax | done | `crates/splot-core/src/headers/layer_config_record.rs` |
| 5.9, 6.9 | `AV2-5.9-ATLAS-SEGMENT` | Atlas segment info OBU syntax | done | `crates/splot-core/src/headers/atlas_segment.rs` |
| 5.9.1, 6.9.2 | `AV2-5.9.1-ATLAS-LABEL-SEGMENT-INFO` | Atlas label segment info syntax | done | `crates/splot-core/src/headers/atlas_segment.rs` |
| 5.9.2, 6.9.3 | `AV2-5.9.2-ATLAS-ENHANCED-INFO` | Atlas enhanced atlas info syntax | done | `crates/splot-core/src/headers/atlas_segment.rs` |
| 5.9.3, 6.9.4 | `AV2-5.9.3-ATLAS-MULTISTREAM-INFO` | Atlas multistream info syntax | done | `crates/splot-core/src/headers/atlas_segment.rs` |
| 5.9.4, 6.9.5 | `AV2-5.9.4-ATLAS-MULTISTREAM-ALPHA-INFO` | Atlas multistream with alpha info syntax | done | `crates/splot-core/src/headers/atlas_segment.rs` |
| 5.9.5, 6.9.6 | `AV2-5.9.5-ATLAS-BASIC-INFO` | Atlas basic info syntax | done | `crates/splot-core/src/headers/atlas_segment.rs` |
| 5.10, 5.11, 6.10 | `AV2-5.10-OPERATING-POINT-SET` | Operating point set OBU syntax | done | `crates/splot-core/src/headers/operating_point_set.rs` |
| 5.10, 6.10.2 | `AV2-5.10-OPS-SYNTAX-ELEMENTS` | Operating point set syntax elements | done | `crates/splot-core/src/headers/operating_point_set.rs` |
| 5.11, 6.10 | `AV2-5.11-OPERATING-POINT-PAYLOAD` | Operating point payload syntax | done | `crates/splot-core/src/headers/operating_point_set.rs` |
| 5.11.1, 6.10.3 | `AV2-5.11.1-OPS-AGGREGATE-INFO` | Operating point aggregate info syntax | done | `crates/splot-core/src/headers/operating_point_set.rs` |
| 5.11.2, 6.10.4 | `AV2-5.11.2-OPS-SEQ-PTL-INFO` | Operating point sequence profile tier level information syntax | done | `crates/splot-core/src/headers/operating_point_set.rs` |
| 5.11.3, 6.10.5 | `AV2-5.11.3-OPS-DECODER-MODEL-INFO` | Operating point decoder model info syntax | done | `crates/splot-core/src/headers/operating_point_set.rs` |
| 5.11.4, 6.10.6 | `AV2-5.11.4-OPS-COLOR-INFO` | Operating point color info syntax | done | `crates/splot-core/src/headers/operating_point_set.rs` |
| 5.11.5, 6.10.7 | `AV2-5.11.5-OPS-MLAYER-INFO` | Operating point mlayer info syntax | done | `crates/splot-core/src/headers/operating_point_set.rs` |
| 5.12, 6.11 | `AV2-5.12-BUFFER-REMOVAL-TIMING` | Buffer removal timing OBU syntax | done | `crates/splot-core/src/headers/buffer_removal_timing.rs` |
| 5.13, 6.12 | `AV2-5.13-QUANTIZATION-MATRIX` | Quantization matrix OBU syntax | done | `crates/splot-core/src/headers/quantizer_matrix.rs` |
| 5.14, 5.18.10.2, 6.13, 6.17.10.2 | `AV2-5.14-FILM-GRAIN` | Film grain OBU syntax | done | `crates/splot-core/src/headers/film_grain.rs` |
| 5.15, 6.14 | `AV2-5.15-CONTENT-INTERPRETATION` | Content interpretation OBU syntax | done | `crates/splot-core/src/headers/content_interpretation.rs` |
| 5.16, 6.15 | `AV2-5.16-PADDING` | Padding OBU syntax | done | `crates/splot-core/src/headers/padding.rs` |
| 5.17, 6.16 | `AV2-5.17-METADATA` | Metadata OBU syntax | done | `crates/splot-core/src/headers/metadata.rs` |
| 5.17.1, 6.16.1 | `AV2-5.17.1-METADATA-UNIT` | Metadata unit syntax | done | `crates/splot-core/src/headers/metadata.rs` |
| 5.17.2, 6.16.2 | `AV2-5.17.2-METADATA-SHORT` | Short metadata OBU syntax | done | `crates/splot-core/src/headers/metadata.rs` |
| 5.17.3, 6.16.3 | `AV2-5.17.3-METADATA-GROUP` | Metadata group OBU syntax | done | `crates/splot-core/src/headers/metadata.rs` |
| 5.17.4, 6.16.4 | `AV2-5.17.4-METADATA-ITUT-T35` | ITU-T T.35 metadata syntax | done | `crates/splot-core/src/headers/metadata.rs` |
| 5.17.5, 6.16.5 | `AV2-5.17.5-METADATA-HDR-CLL` | HDR CLL metadata syntax | done | `crates/splot-core/src/headers/metadata.rs` |
| 5.17.6, 6.16.6 | `AV2-5.17.6-METADATA-HDR-MDCV` | HDR MDCV metadata syntax | done | `crates/splot-core/src/headers/metadata.rs` |
| 5.17.7, 6.16.7 | `AV2-5.17.7-METADATA-TIMECODE` | Timecode metadata syntax | done | `crates/splot-core/src/headers/metadata.rs` |
| 5.17.8, 6.16.8 | `AV2-5.17.8-METADATA-BANDING-HINTS` | Banding hints metadata syntax | done | `crates/splot-core/src/headers/metadata.rs` |
| 5.17.9, 6.16.9 | `AV2-5.17.9-METADATA-ICC-PROFILE` | ICC profile metadata syntax | done | `crates/splot-core/src/headers/metadata.rs` |
| 5.17.10, 6.16.10 | `AV2-5.17.10-METADATA-SCAN-TYPE` | Scan type metadata syntax | done | `crates/splot-core/src/headers/metadata.rs` |
| 5.17.11, 6.16.11 | `AV2-5.17.11-METADATA-TEMPORAL-POINT-INFO` | Temporal point info metadata syntax | done | `crates/splot-core/src/headers/metadata.rs` |
| 5.17.12, 6.16.13 | `AV2-5.17.12-METADATA-DECODED-FRAME-HASH` | Decoded frame hash metadata syntax | done | `crates/splot-core/src/headers/metadata.rs` |
| 5.17.13, 6.16.12 | `AV2-5.17.13-METADATA-USER-DATA-UNREGISTERED` | User data unregistered metadata syntax | done | `crates/splot-core/src/headers/metadata.rs` |
| 5.18, 6.17 | `AV2-5.18-FRAME-HEADER` | Frame header syntax | partial | `crates/splot-core/src/headers/frame/mod.rs` |
| 5.18.1, 6.17 | `AV2-5.18.1-FRAME-HEADER-GENERAL` | Frame header general syntax | partial | `crates/splot-core/src/headers/frame/mod.rs` |
| 5.18.2, 6.17.2 | `AV2-5.18.2-FRAME-HEADER-INFO` | Frame header info syntax | partial | `crates/splot-core/src/headers/frame/info.rs` |
| 5.18.2, 5.4.1, 5.4.6, 5.4.7, 5.4.8 | `ENC-FRAME-HEADER-CORE-ASSEMBLER` | Encoder writer-input minimal-intra FrameHeaderCore parse-backed assembler | done | `crates/splot-core/src/headers/frame/encoder_input.rs` |
| 5.18.3 | `AV2-5.18.3-FRAME-CONFIGURATION` | Frame configuration syntax | partial | `crates/splot-core/src/headers/frame/config.rs` |
| 5.18.4, 6.17.4.1 | `AV2-5.18.4-FRAME-SIZE` | Frame size syntax | partial | `crates/splot-core/src/headers/frame/size.rs` |
| 5.18.5, 5.18.5.2, 6.17.5.2 | `AV2-5.18.5-FILTERING` | Frame filtering syntax | partial | `crates/splot-core/src/headers/frame/filtering.rs` |
| 5.18.6, 5.18.7.8, 6.17.6 | `AV2-5.18.6-QUANTIZATION` | Frame quantization syntax | done | `crates/splot-core/src/headers/frame/quant.rs` |
| 5.18.7, 6.17.7 | `AV2-5.18.7-SEGMENTATION-TILING` | Frame segmentation and tiling syntax | partial | `crates/splot-core/src/headers/frame/tiling.rs` |
| 5.18.7.3, 5.18.7.5, 5.18.7.7, 6.17.7 | `AV2-5.18.7.3-TILE-PARAMS` | Tile params helper and tile-partitioning tables | done | `crates/splot-core/src/tile.rs` |
| 5.18.8, 6.17 | `AV2-5.18.8-TRANSFORM-CODING-MODES` | Frame transform and coding mode syntax | partial | `crates/splot-core/src/headers/frame/tail.rs` |
| 5.18.9, 6.17 | `AV2-5.18.9-GLOBAL-MOTION` | Frame global motion syntax | partial | `crates/splot-core/src/headers/frame/global_motion.rs` |
| 5.18.10, 6.17.10.1 | `AV2-5.18.10-FILM-GRAIN-STRUCTURES` | Frame film grain structures syntax | done | `crates/splot-core/src/headers/frame/tail.rs` |
| 5.19, 5.20, 6.18, 6.19 | `AV2-5.19-TILE-GROUP` | Tile group OBU syntax | partial | `crates/splot-core/src/headers/tile_group.rs` |
| 5.19 | `ENC-WRITER-INPUT-STRUCTURE` | Encoder writer-input single-tile structure constructor | done | `crates/splot-core/src/headers/tile_group.rs` |
| 5.20, 6.19 | `AV2-5.20-TILE-GROUP-PAYLOAD` | Tile group payload syntax | partial | `crates/splot-core/src/headers/tile_group.rs` |
| 5.20, 6.19 | `ENC-WRITER-INPUT-FRAMING` | Encoder writer-input single-tile framing constructor | done | `crates/splot-core/src/headers/tile_group.rs` |
| 8.2.2, 8.2.3, 8.2.4, 8.2.5, 8.2.6, 9.2 | `ENC-BITSTREAM-WRITER` | Bitstream writer foundation | partial | `crates/splot-core/src/write/mod.rs` |
| — | `AV2-IVF-CONTAINER` | IVF container support | done | `crates/splot-core/src/ivf.rs` |
| Annex B | `AV2-B-ANNEXB-OBU-ENVELOPE` | Annex B length-delimited OBU envelope | done | `crates/splot-core/src/annexb.rs` |
