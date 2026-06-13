# Spec coverage

Generated from `docs/IMPLEMENTATION-MATRIX.toml` by `cargo xtask spec-coverage --format markdown --output docs/SPEC-COVERAGE.md`. Do not edit by hand.

Matrix version 1. Last reviewed 2026-06-10. 147 feature(s); 107 cite a spec section.

One row per (spec section, feature) pair, in spec order; a feature citing both a syntax and a semantics section appears under both. The canonical status source is [IMPLEMENTATION-MATRIX.toml](./IMPLEMENTATION-MATRIX.toml); the full per-feature ledger is [FEATURE-STATUS.md](./FEATURE-STATUS.md).

Legend: ✅ done · 🟡 partial · ⏳ pending external proof · ⛔ blocked · 🧪 experimental · — not applicable · (blank) todo. Diagnostics counts the rule IDs recorded in the feature's proof; emitted validator IDs are registered in [VALIDATOR-DIAGNOSTICS.md](./VALIDATOR-DIAGNOSTICS.md), and emitted decoder IDs are registered in [DECODER-DIAGNOSTICS.md](./DECODER-DIAGNOSTICS.md).

## Chapter 4 — Conventions

| Section | Spec item | Feature | Mapped | Parse | Validate | Tests | Diagnostics |
|---|---|---|:-:|:-:|:-:|:-:|:-:|
| [§ 4.11.3](./spec/av2/1.0.0/04-conventions.md#s-4-11-3) | uvlc() | `AV2-4.11.3-UVLC` | ✅ | ✅ | — | ✅ | — |
| [§ 4.11.4](./spec/av2/1.0.0/04-conventions.md#s-4-11-4) | svlc() | `AV2-4.11.4-SVLC` | ✅ | ✅ | — | ✅ | — |
| [§ 4.11.5](./spec/av2/1.0.0/04-conventions.md#s-4-11-5) | le(n) | `AV2-4.11.5-LE` | ✅ | ✅ | — | ✅ | — |
| [§ 4.11.6](./spec/av2/1.0.0/04-conventions.md#s-4-11-6) | leb128() | `AV2-4.11.6-LEB128` | ✅ | ✅ | ✅ | ✅ | 1 |
| [§ 4.11.7](./spec/av2/1.0.0/04-conventions.md#s-4-11-7) | su(n) | `AV2-4.11.7-SU` | ✅ | ✅ | — | ✅ | — |
| [§ 4.11.8](./spec/av2/1.0.0/04-conventions.md#s-4-11-8) | ns(n) | `AV2-4.11.8-NS` | ✅ | ✅ | — | ✅ | — |

## Chapter 5 — Syntax structures

| Section | Spec item | Feature | Mapped | Parse | Validate | Tests | Diagnostics |
|---|---|---|:-:|:-:|:-:|:-:|:-:|
| [§ 5.2.1](./spec/av2/1.0.0/05-syntax-structures.md#s-5-2-1) | General OBU syntax | `AV2-5.2.1-OBU-DISPATCH` | ✅ | 🟡 | — | ✅ | — |
| [§ 5.2.1](./spec/av2/1.0.0/05-syntax-structures.md#s-5-2-1) | General OBU syntax | `AV2-5.2.1-OBU-TYPE` | ✅ | ✅ | ✅ | ✅ | 5 |
| [§ 5.2.2](./spec/av2/1.0.0/05-syntax-structures.md#s-5-2-2) | OBU header syntax | `AV2-5.2.2-OBU-HEADER` | ✅ | ✅ | ✅ | ✅ | 6 |
| [§ 5.2.3](./spec/av2/1.0.0/05-syntax-structures.md#s-5-2-3) | Trailing bits syntax | `AV2-5.2.3-TRAILING-BITS` | ✅ | ✅ | 🟡 | ✅ | 2 |
| [§ 5.2.4](./spec/av2/1.0.0/05-syntax-structures.md#s-5-2-4) | Byte alignment syntax | `AV2-5.2.4-BYTE-ALIGNMENT` | ✅ | ✅ | 🟡 | ✅ | 1 |
| [§ 5.3](./spec/av2/1.0.0/05-syntax-structures.md#s-5-3) | Reserved OBU syntax | `AV2-5.3-RESERVED-OBU` | ✅ | — | ✅ | ✅ | 2 |
| [§ 5.4](./spec/av2/1.0.0/05-syntax-structures.md#s-5-4) | Sequence header OBU syntax | `AV2-5.4-SEQUENCE-HEADER` | ✅ | 🟡 | 🟡 | 🟡 | 1 |
| [§ 5.4.1](./spec/av2/1.0.0/05-syntax-structures.md#s-5-4-1) | General sequence header OBU syntax | `AV2-5.4.1-SEQUENCE-HEADER-GENERAL` | ✅ | ✅ | ✅ | ✅ | 6 |
| [§ 5.4.2](./spec/av2/1.0.0/05-syntax-structures.md#s-5-4-2) | Sequence tile config syntax | `AV2-5.4.2-SEQUENCE-TILE-CONFIG` | ✅ | ✅ | ✅ | ✅ | 4 |
| [§ 5.4.3](./spec/av2/1.0.0/05-syntax-structures.md#s-5-4-3) | Sequence partition config syntax | `AV2-5.4.3-SEQUENCE-PARTITION-CONFIG` | ✅ | ✅ | 🟡 | ✅ | — |
| [§ 5.4.4](./spec/av2/1.0.0/05-syntax-structures.md#s-5-4-4) | Sequence segment config syntax | `AV2-5.4.4-SEQUENCE-SEGMENT-CONFIG` | ✅ | ✅ | 🟡 | ✅ | — |
| [§ 5.4.5](./spec/av2/1.0.0/05-syntax-structures.md#s-5-4-5) | Sequence intra config syntax | `AV2-5.4.5-SEQUENCE-INTRA-CONFIG` | ✅ | ✅ | 🟡 | ✅ | — |
| [§ 5.4.6](./spec/av2/1.0.0/05-syntax-structures.md#s-5-4-6) | Sequence inter config syntax | `AV2-5.4.6-SEQUENCE-INTER-CONFIG` | ✅ | ✅ | ✅ | ✅ | 1 |
| [§ 5.4.7](./spec/av2/1.0.0/05-syntax-structures.md#s-5-4-7) | Sequence screen content config syntax | `AV2-5.4.7-SEQUENCE-SCC-CONFIG` | ✅ | ✅ | 🟡 | ✅ | — |
| [§ 5.4.8](./spec/av2/1.0.0/05-syntax-structures.md#s-5-4-8) | Sequence transform quant entropy config syntax | `AV2-5.4.8-SEQUENCE-TQ-ENTROPY-CONFIG` | ✅ | ✅ | 🟡 | ✅ | — |
| [§ 5.4.9](./spec/av2/1.0.0/05-syntax-structures.md#s-5-4-9) | Segment information syntax | `AV2-5.4.9-SEGMENT-INFO` | ✅ | ✅ | — | ✅ | — |
| [§ 5.4.10](./spec/av2/1.0.0/05-syntax-structures.md#s-5-4-10) | Sequence filter config syntax | `AV2-5.4.10-SEQUENCE-FILTER-CONFIG` | ✅ | ✅ | 🟡 | ✅ | — |
| [§ 5.4.11](./spec/av2/1.0.0/05-syntax-structures.md#s-5-4-11) | User defined QM syntax | `AV2-5.4.11-USER-QM` | ✅ | ✅ | — | ✅ | — |
| [§ 5.4.12](./spec/av2/1.0.0/05-syntax-structures.md#s-5-4-12) | Timing info syntax | `AV2-5.4.12-TIMING-INFO` | ✅ | ✅ | ✅ | ✅ | 3 |
| [§ 5.4.13](./spec/av2/1.0.0/05-syntax-structures.md#s-5-4-13) | Sequence decoder model info syntax | `AV2-5.4.13-SEQUENCE-DECODER-MODEL-INFO` | ✅ | ✅ | 🟡 | ✅ | — |
| [§ 5.5](./spec/av2/1.0.0/05-syntax-structures.md#s-5-5) | Temporal delimiter OBU syntax | `AV2-5.5-TEMPORAL-DELIMITER` | ✅ | ✅ | 🟡 | ✅ | 2 |
| [§ 5.6](./spec/av2/1.0.0/05-syntax-structures.md#s-5-6) | Multi Stream Decoder Operation OBU syntax | `AV2-5.6-MSDO` | ✅ | ✅ | 🟡 | ✅ | 9 |
| [§ 5.7](./spec/av2/1.0.0/05-syntax-structures.md#s-5-7) | Multi frame header OBU syntax | `AV2-5.7-MULTI-FRAME-HEADER` | ✅ | ✅ | 🟡 | ✅ | 7 |
| [§ 5.8](./spec/av2/1.0.0/05-syntax-structures.md#s-5-8) | Layer config record OBU syntax | `AV2-5.8-LAYER-CONFIG-RECORD` | ✅ | ✅ | 🟡 | ✅ | 11 |
| [§ 5.8.1](./spec/av2/1.0.0/05-syntax-structures.md#s-5-8-1) | LCR global info syntax | `AV2-5.8.1-LCR-GLOBAL-INFO` | ✅ | ✅ | 🟡 | ✅ | 6 |
| [§ 5.8.2](./spec/av2/1.0.0/05-syntax-structures.md#s-5-8-2) | LCR local info syntax | `AV2-5.8.2-LCR-LOCAL-INFO` | ✅ | ✅ | ✅ | ✅ | 2 |
| [§ 5.8.3](./spec/av2/1.0.0/05-syntax-structures.md#s-5-8-3) | LCR aggregate info syntax | `AV2-5.8.3-LCR-AGGREGATE-INFO` | ✅ | ✅ | 🟡 | ✅ | — |
| [§ 5.8.4](./spec/av2/1.0.0/05-syntax-structures.md#s-5-8-4) | LCR sequence profile tier level information syntax | `AV2-5.8.4-LCR-SEQ-PTL-INFO` | ✅ | ✅ | 🟡 | ✅ | 4 |
| [§ 5.8.5](./spec/av2/1.0.0/05-syntax-structures.md#s-5-8-5) | LCR global payload syntax | `AV2-5.8.5-LCR-GLOBAL-PAYLOAD` | ✅ | ✅ | ✅ | ✅ | 1 |
| [§ 5.8.6](./spec/av2/1.0.0/05-syntax-structures.md#s-5-8-6) | LCR xlayer info syntax | `AV2-5.8.6-LCR-XLAYER-INFO` | ✅ | ✅ | ✅ | ✅ | 5 |
| [§ 5.8.7](./spec/av2/1.0.0/05-syntax-structures.md#s-5-8-7) | LCR rep info syntax | `AV2-5.8.7-LCR-REP-INFO` | ✅ | ✅ | ✅ | ✅ | 1 |
| [§ 5.8.8](./spec/av2/1.0.0/05-syntax-structures.md#s-5-8-8) | LCR embedded layer info syntax | `AV2-5.8.8-LCR-EMBEDDED-LAYER-INFO` | ✅ | ✅ | 🟡 | ✅ | 1 |
| [§ 5.8.9](./spec/av2/1.0.0/05-syntax-structures.md#s-5-8-9) | LCR xlayer color info syntax | `AV2-5.8.9-LCR-XLAYER-COLOR-INFO` | ✅ | ✅ | ✅ | ✅ | 5 |
| [§ 5.9](./spec/av2/1.0.0/05-syntax-structures.md#s-5-9) | Atlas segment info OBU syntax | `AV2-5.9-ATLAS-SEGMENT` | ✅ | ✅ | 🟡 | ✅ | 6 |
| [§ 5.9.1](./spec/av2/1.0.0/05-syntax-structures.md#s-5-9-1) | Atlas label segment info syntax | `AV2-5.9.1-ATLAS-LABEL-SEGMENT-INFO` | ✅ | ✅ | ✅ | ✅ | 5 |
| [§ 5.9.2](./spec/av2/1.0.0/05-syntax-structures.md#s-5-9-2) | Atlas enhanced atlas info syntax | `AV2-5.9.2-ATLAS-ENHANCED-INFO` | ✅ | ✅ | ✅ | ✅ | 2 |
| [§ 5.9.3](./spec/av2/1.0.0/05-syntax-structures.md#s-5-9-3) | Atlas multistream info syntax | `AV2-5.9.3-ATLAS-MULTISTREAM-INFO` | ✅ | ✅ | ✅ | ✅ | 1 |
| [§ 5.9.4](./spec/av2/1.0.0/05-syntax-structures.md#s-5-9-4) | Atlas multistream with alpha info syntax | `AV2-5.9.4-ATLAS-MULTISTREAM-ALPHA-INFO` | ✅ | ✅ | ✅ | ✅ | 1 |
| [§ 5.9.5](./spec/av2/1.0.0/05-syntax-structures.md#s-5-9-5) | Atlas basic info syntax | `AV2-5.9.5-ATLAS-BASIC-INFO` | ✅ | ✅ | ✅ | ✅ | 2 |
| [§ 5.10](./spec/av2/1.0.0/05-syntax-structures.md#s-5-10) | Operating point set OBU syntax | `AV2-5.10-OPERATING-POINT-SET` | ✅ | ✅ | 🟡 | ✅ | 6 |
| [§ 5.10](./spec/av2/1.0.0/05-syntax-structures.md#s-5-10) | Operating point set OBU syntax | `AV2-5.10-OPS-SYNTAX-ELEMENTS` | ✅ | ✅ | 🟡 | ✅ | 2 |
| [§ 5.11](./spec/av2/1.0.0/05-syntax-structures.md#s-5-11) | Operating point payload syntax | `AV2-5.10-OPERATING-POINT-SET` | ✅ | ✅ | 🟡 | ✅ | 6 |
| [§ 5.11](./spec/av2/1.0.0/05-syntax-structures.md#s-5-11) | Operating point payload syntax | `AV2-5.11-OPERATING-POINT-PAYLOAD` | ✅ | ✅ | 🟡 | ✅ | 2 |
| [§ 5.11.1](./spec/av2/1.0.0/05-syntax-structures.md#s-5-11-1) | Operating point set aggregate info syntax | `AV2-5.11.1-OPS-AGGREGATE-INFO` | ✅ | ✅ | 🟡 | ✅ | — |
| [§ 5.11.2](./spec/av2/1.0.0/05-syntax-structures.md#s-5-11-2) | Operating point set sequence profile tier level information syntax | `AV2-5.11.2-OPS-SEQ-PTL-INFO` | ✅ | ✅ | 🟡 | ✅ | 1 |
| [§ 5.11.3](./spec/av2/1.0.0/05-syntax-structures.md#s-5-11-3) | Operating point set decoder model info syntax | `AV2-5.11.3-OPS-DECODER-MODEL-INFO` | ✅ | ✅ | 🟡 | ✅ | 2 |
| [§ 5.11.4](./spec/av2/1.0.0/05-syntax-structures.md#s-5-11-4) | Operating point set color info syntax | `AV2-5.11.4-OPS-COLOR-INFO` | ✅ | ✅ | 🟡 | ✅ | — |
| [§ 5.11.5](./spec/av2/1.0.0/05-syntax-structures.md#s-5-11-5) | Operating point set mlayer info syntax | `AV2-5.11.5-OPS-MLAYER-INFO` | ✅ | ✅ | 🟡 | ✅ | 1 |
| [§ 5.12](./spec/av2/1.0.0/05-syntax-structures.md#s-5-12) | Buffer removal timing OBU syntax | `AV2-5.12-BUFFER-REMOVAL-TIMING` | ✅ | ✅ | 🟡 | ✅ | 2 |
| [§ 5.13](./spec/av2/1.0.0/05-syntax-structures.md#s-5-13) | Quantizer Matrix OBU syntax | `AV2-5.13-QUANTIZATION-MATRIX` | ✅ | ✅ | 🟡 | ✅ | 3 |
| [§ 5.14](./spec/av2/1.0.0/05-syntax-structures.md#s-5-14) | Film grain OBU syntax | `AV2-5.14-FILM-GRAIN` | ✅ | ✅ | 🟡 | ✅ | 6 |
| [§ 5.15](./spec/av2/1.0.0/05-syntax-structures.md#s-5-15) | Content interpretation OBU syntax | `AV2-5.15-CONTENT-INTERPRETATION` | ✅ | ✅ | ✅ | ✅ | 4 |
| [§ 5.16](./spec/av2/1.0.0/05-syntax-structures.md#s-5-16) | Padding OBU syntax | `AV2-5.16-PADDING` | ✅ | ✅ | ✅ | ✅ | 2 |
| [§ 5.17](./spec/av2/1.0.0/05-syntax-structures.md#s-5-17) | Metadata OBU syntax | `AV2-5.17-METADATA` | ✅ | ✅ | 🟡 | ✅ | 20 |
| [§ 5.17.1](./spec/av2/1.0.0/05-syntax-structures.md#s-5-17-1) | Metadata unit syntax | `AV2-5.17.1-METADATA-UNIT` | ✅ | ✅ | 🟡 | ✅ | 1 |
| [§ 5.17.2](./spec/av2/1.0.0/05-syntax-structures.md#s-5-17-2) | Metadata short OBU syntax | `AV2-5.17.2-METADATA-SHORT` | ✅ | ✅ | 🟡 | ✅ | 2 |
| [§ 5.17.3](./spec/av2/1.0.0/05-syntax-structures.md#s-5-17-3) | Metadata group OBU syntax | `AV2-5.17.3-METADATA-GROUP` | ✅ | ✅ | ✅ | ✅ | 7 |
| [§ 5.17.4](./spec/av2/1.0.0/05-syntax-structures.md#s-5-17-4) | Metadata ITUT T35 syntax | `AV2-5.17.4-METADATA-ITUT-T35` | ✅ | ✅ | ✅ | ✅ | 4 |
| [§ 5.17.5](./spec/av2/1.0.0/05-syntax-structures.md#s-5-17-5) | Metadata high dynamic range content light level syntax | `AV2-5.17.5-METADATA-HDR-CLL` | ✅ | ✅ | 🟡 | ✅ | 2 |
| [§ 5.17.6](./spec/av2/1.0.0/05-syntax-structures.md#s-5-17-6) | Metadata high dynamic range mastering display color volume syntax | `AV2-5.17.6-METADATA-HDR-MDCV` | ✅ | ✅ | 🟡 | ✅ | 2 |
| [§ 5.17.7](./spec/av2/1.0.0/05-syntax-structures.md#s-5-17-7) | Metadata timecode syntax | `AV2-5.17.7-METADATA-TIMECODE` | ✅ | ✅ | 🟡 | ✅ | 5 |
| [§ 5.17.8](./spec/av2/1.0.0/05-syntax-structures.md#s-5-17-8) | Metadata banding hints syntax | `AV2-5.17.8-METADATA-BANDING-HINTS` | ✅ | ✅ | ✅ | ✅ | 4 |
| [§ 5.17.9](./spec/av2/1.0.0/05-syntax-structures.md#s-5-17-9) | Metadata ICC profile syntax | `AV2-5.17.9-METADATA-ICC-PROFILE` | ✅ | ✅ | ✅ | ✅ | 4 |
| [§ 5.17.10](./spec/av2/1.0.0/05-syntax-structures.md#s-5-17-10) | Metadata scan type syntax | `AV2-5.17.10-METADATA-SCAN-TYPE` | ✅ | ✅ | ✅ | ✅ | 5 |
| [§ 5.17.11](./spec/av2/1.0.0/05-syntax-structures.md#s-5-17-11) | Metadata temporal point info syntax | `AV2-5.17.11-METADATA-TEMPORAL-POINT-INFO` | ✅ | ✅ | ✅ | ✅ | 1 |
| [§ 5.17.12](./spec/av2/1.0.0/05-syntax-structures.md#s-5-17-12) | Metadata decoded frame hash syntax | `AV2-5.17.12-METADATA-DECODED-FRAME-HASH` | ✅ | ✅ | 🟡 | ✅ | 1 |
| [§ 5.17.13](./spec/av2/1.0.0/05-syntax-structures.md#s-5-17-13) | Metadata user data unregistered syntax | `AV2-5.17.13-METADATA-USER-DATA-UNREGISTERED` | ✅ | ✅ | ✅ | ✅ | 4 |
| [§ 5.18](./spec/av2/1.0.0/05-syntax-structures.md#s-5-18) | Frame header syntax | `AV2-5.18-FRAME-HEADER` | ✅ | 🟡 | 🟡 | ✅ | 4 |
| [§ 5.18.1](./spec/av2/1.0.0/05-syntax-structures.md#s-5-18-1) | General frame header syntax | `AV2-5.18.1-FRAME-HEADER-GENERAL` | ✅ | 🟡 | 🟡 | ✅ | 2 |
| [§ 5.18.2](./spec/av2/1.0.0/05-syntax-structures.md#s-5-18-2) | Frame header info syntax | `AV2-5.18.2-FRAME-HEADER-INFO` | ✅ | 🟡 | 🟡 | ✅ | 9 |
| [§ 5.18.3](./spec/av2/1.0.0/05-syntax-structures.md#s-5-18-3) | Frame configuration structures | `AV2-5.18.3-FRAME-CONFIGURATION` | ✅ | 🟡 | — | ✅ | — |
| [§ 5.18.4](./spec/av2/1.0.0/05-syntax-structures.md#s-5-18-4) | Frame size structures | `AV2-5.18.4-FRAME-SIZE` | ✅ | 🟡 | 🟡 | ✅ | 2 |
| [§ 5.18.5](./spec/av2/1.0.0/05-syntax-structures.md#s-5-18-5) | Filtering structures | `AV2-5.18.5-FILTERING` | ✅ | 🟡 | — | 🟡 | — |
| [§ 5.18.5.2](./spec/av2/1.0.0/05-syntax-structures.md#s-5-18-5-2) | Deblocking filter params syntax | `AV2-5.18.5-FILTERING` | ✅ | 🟡 | — | 🟡 | — |
| [§ 5.18.6](./spec/av2/1.0.0/05-syntax-structures.md#s-5-18-6) | Quantization structures | `AV2-5.18.6-QUANTIZATION` | ✅ | 🟡 | 🟡 | 🟡 | 2 |
| [§ 5.18.7](./spec/av2/1.0.0/05-syntax-structures.md#s-5-18-7) | Segmentation and tiling structures | `AV2-5.18.7-SEGMENTATION-TILING` | ✅ | 🟡 | 🟡 | 🟡 | 5 |
| [§ 5.18.7.3](./spec/av2/1.0.0/05-syntax-structures.md#s-5-18-7-3) | Tile params syntax | `AV2-5.18.7.3-TILE-PARAMS` | ✅ | ✅ | ✅ | ✅ | 4 |
| [§ 5.18.7.5](./spec/av2/1.0.0/05-syntax-structures.md#s-5-18-7-5) | Uniform spacing function | `AV2-5.18.7.3-TILE-PARAMS` | ✅ | ✅ | ✅ | ✅ | 4 |
| [§ 5.18.7.7](./spec/av2/1.0.0/05-syntax-structures.md#s-5-18-7-7) | Tile size calculation function | `AV2-5.18.7.3-TILE-PARAMS` | ✅ | ✅ | ✅ | ✅ | 4 |
| [§ 5.18.7.8](./spec/av2/1.0.0/05-syntax-structures.md#s-5-18-7-8) | Quantizer index delta parameters syntax | `AV2-5.18.6-QUANTIZATION` | ✅ | 🟡 | 🟡 | 🟡 | 2 |
| [§ 5.18.8](./spec/av2/1.0.0/05-syntax-structures.md#s-5-18-8) | Transform and coding mode structures | `AV2-5.18.8-TRANSFORM-CODING-MODES` | ✅ | 🟡 | — | ✅ | — |
| [§ 5.18.9](./spec/av2/1.0.0/05-syntax-structures.md#s-5-18-9) | Global motion structures | `AV2-5.18.9-GLOBAL-MOTION` | ✅ | 🟡 | — | ✅ | — |
| [§ 5.18.10](./spec/av2/1.0.0/05-syntax-structures.md#s-5-18-10) | Film grain structures | `AV2-5.18.10-FILM-GRAIN-STRUCTURES` | ✅ | ✅ | 🟡 | ✅ | 1 |
| [§ 5.18.10.2](./spec/av2/1.0.0/05-syntax-structures.md#s-5-18-10-2) | Film grain model syntax | `AV2-5.14-FILM-GRAIN` | ✅ | ✅ | 🟡 | ✅ | 6 |
| [§ 5.19](./spec/av2/1.0.0/05-syntax-structures.md#s-5-19) | Tile group OBU syntax | `AV2-5.19-TILE-GROUP` | ✅ | 🟡 | 🟡 | ✅ | 7 |
| [§ 5.20](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20) | Tile group payload syntax | `AV2-5.19-TILE-GROUP` | ✅ | 🟡 | 🟡 | ✅ | 7 |
| [§ 5.20](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20) | Tile group payload syntax | `AV2-5.20-TILE-GROUP-PAYLOAD` | ✅ | 🟡 | 🟡 | ✅ | 3 |

## Chapter 6 — Syntax structures semantics

| Section | Spec item | Feature | Mapped | Parse | Validate | Tests | Diagnostics |
|---|---|---|:-:|:-:|:-:|:-:|:-:|
| [§ 6.2.2](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-2-2) | OBU header semantics | `AV2-5.2.1-OBU-TYPE` | ✅ | ✅ | ✅ | ✅ | 5 |
| [§ 6.2.2](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-2-2) | OBU header semantics | `AV2-5.2.2-OBU-HEADER` | ✅ | ✅ | ✅ | ✅ | 6 |
| [§ 6.2.2](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-2-2) | OBU header semantics | `AV2-6.2.2-OBU-HEADER-ACTIVATED-SEQUENCE-LIMITS` | ✅ | ✅ | 🟡 | ✅ | 3 |
| [§ 6.2.3](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-2-3) | Trailing bits semantics | `AV2-5.2.3-TRAILING-BITS` | ✅ | ✅ | 🟡 | ✅ | 2 |
| [§ 6.2.3](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-2-3) | Trailing bits semantics | `AV2-5.3-RESERVED-OBU` | ✅ | — | ✅ | ✅ | 2 |
| [§ 6.2.4](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-2-4) | Byte alignment semantics | `AV2-5.2.4-BYTE-ALIGNMENT` | ✅ | ✅ | 🟡 | ✅ | 1 |
| [§ 6.4](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-4) | Sequence header OBU semantics | `AV2-5.4-SEQUENCE-HEADER` | ✅ | 🟡 | 🟡 | 🟡 | 1 |
| [§ 6.4](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-4) | Sequence header OBU semantics | `AV2-6.2.2-OBU-HEADER-ACTIVATED-SEQUENCE-LIMITS` | ✅ | ✅ | 🟡 | ✅ | 3 |
| [§ 6.4](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-4) | Sequence header OBU semantics | `AV2-6.4-SEQUENCE-HEADER-SEMANTICS` | ✅ | ✅ | 🟡 | ✅ | 10 |
| [§ 6.4.1](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-4-1) | General sequence header OBU semantics | `AV2-5.4.1-SEQUENCE-HEADER-GENERAL` | ✅ | ✅ | ✅ | ✅ | 6 |
| [§ 6.4.2](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-4-2) | Sequence tile config semantics | `AV2-5.4.2-SEQUENCE-TILE-CONFIG` | ✅ | ✅ | ✅ | ✅ | 4 |
| [§ 6.4.3](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-4-3) | Sequence partition config semantics | `AV2-5.4.3-SEQUENCE-PARTITION-CONFIG` | ✅ | ✅ | 🟡 | ✅ | — |
| [§ 6.4.4](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-4-4) | Sequence segment config semantics | `AV2-5.4.4-SEQUENCE-SEGMENT-CONFIG` | ✅ | ✅ | 🟡 | ✅ | — |
| [§ 6.4.5](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-4-5) | Sequence intra config semantics | `AV2-5.4.5-SEQUENCE-INTRA-CONFIG` | ✅ | ✅ | 🟡 | ✅ | — |
| [§ 6.4.6](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-4-6) | Sequence inter config semantics | `AV2-5.4.6-SEQUENCE-INTER-CONFIG` | ✅ | ✅ | ✅ | ✅ | 1 |
| [§ 6.4.6](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-4-6) | Sequence inter config semantics | `AV2-6.17.2-FRAME-HEADER-INFO-SEMANTICS` | ✅ | — | 🟡 | ✅ | 15 |
| [§ 6.4.7](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-4-7) | Sequence screen content config semantics | `AV2-5.4.7-SEQUENCE-SCC-CONFIG` | ✅ | ✅ | 🟡 | ✅ | — |
| [§ 6.4.8](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-4-8) | Sequence transform quant entropy config semantics | `AV2-5.4.8-SEQUENCE-TQ-ENTROPY-CONFIG` | ✅ | ✅ | 🟡 | ✅ | — |
| [§ 6.4.9](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-4-9) | Segment information semantics | `AV2-5.4.9-SEGMENT-INFO` | ✅ | ✅ | — | ✅ | — |
| [§ 6.4.10](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-4-10) | Sequence filter config semantics | `AV2-5.4.10-SEQUENCE-FILTER-CONFIG` | ✅ | ✅ | 🟡 | ✅ | — |
| [§ 6.4.11](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-4-11) | User defined QM semantics | `AV2-5.4.11-USER-QM` | ✅ | ✅ | — | ✅ | — |
| [§ 6.4.12](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-4-12) | Timing info semantics | `AV2-5.4.12-TIMING-INFO` | ✅ | ✅ | ✅ | ✅ | 3 |
| [§ 6.4.13](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-4-13) | Sequence decoder model info semantics | `AV2-5.4.13-SEQUENCE-DECODER-MODEL-INFO` | ✅ | ✅ | 🟡 | ✅ | — |
| [§ 6.5](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-5) | Temporal delimiter OBU semantics | `AV2-5.5-TEMPORAL-DELIMITER` | ✅ | ✅ | 🟡 | ✅ | 2 |
| [§ 6.6](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-6) | Multi Stream Decoder Operation OBU semantics | `AV2-5.6-MSDO` | ✅ | ✅ | 🟡 | ✅ | 9 |
| [§ 6.7](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-7) | Multi frame header OBU semantics | `AV2-5.7-MULTI-FRAME-HEADER` | ✅ | ✅ | 🟡 | ✅ | 7 |
| [§ 6.8](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-8) | Layer config record OBU semantics | `AV2-5.8-LAYER-CONFIG-RECORD` | ✅ | ✅ | 🟡 | ✅ | 11 |
| [§ 6.8.2](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-8-2) | LCR global info semantics | `AV2-5.8.1-LCR-GLOBAL-INFO` | ✅ | ✅ | 🟡 | ✅ | 6 |
| [§ 6.8.3](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-8-3) | LCR local info semantics | `AV2-5.8.2-LCR-LOCAL-INFO` | ✅ | ✅ | ✅ | ✅ | 2 |
| [§ 6.8.4](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-8-4) | LCR aggregate info semantics | `AV2-5.8.3-LCR-AGGREGATE-INFO` | ✅ | ✅ | 🟡 | ✅ | — |
| [§ 6.8.5](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-8-5) | LCR sequence profile tier level information semantics | `AV2-5.8.4-LCR-SEQ-PTL-INFO` | ✅ | ✅ | 🟡 | ✅ | 4 |
| [§ 6.8.6](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-8-6) | LCR global payload semantics | `AV2-5.8.5-LCR-GLOBAL-PAYLOAD` | ✅ | ✅ | ✅ | ✅ | 1 |
| [§ 6.8.7](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-8-7) | LCR xlayer info semantics | `AV2-5.8.6-LCR-XLAYER-INFO` | ✅ | ✅ | ✅ | ✅ | 5 |
| [§ 6.8.8](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-8-8) | LCR rep info semantics | `AV2-5.8.7-LCR-REP-INFO` | ✅ | ✅ | ✅ | ✅ | 1 |
| [§ 6.8.9](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-8-9) | LCR embedded layer info semantics | `AV2-5.8.8-LCR-EMBEDDED-LAYER-INFO` | ✅ | ✅ | 🟡 | ✅ | 1 |
| [§ 6.8.10](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-8-10) | LCR xlayer color info semantics | `AV2-5.8.9-LCR-XLAYER-COLOR-INFO` | ✅ | ✅ | ✅ | ✅ | 5 |
| [§ 6.9](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-9) | Atlas segment info OBU semantics | `AV2-5.9-ATLAS-SEGMENT` | ✅ | ✅ | 🟡 | ✅ | 6 |
| [§ 6.9.2](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-9-2) | Atlas label segment info semantics | `AV2-5.9.1-ATLAS-LABEL-SEGMENT-INFO` | ✅ | ✅ | ✅ | ✅ | 5 |
| [§ 6.9.3](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-9-3) | Atlas enhanced atlas info semantics | `AV2-5.9.2-ATLAS-ENHANCED-INFO` | ✅ | ✅ | ✅ | ✅ | 2 |
| [§ 6.9.4](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-9-4) | Atlas multistream info semantics | `AV2-5.9.3-ATLAS-MULTISTREAM-INFO` | ✅ | ✅ | ✅ | ✅ | 1 |
| [§ 6.9.5](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-9-5) | Atlas multistream with alpha info semantics | `AV2-5.9.4-ATLAS-MULTISTREAM-ALPHA-INFO` | ✅ | ✅ | ✅ | ✅ | 1 |
| [§ 6.9.6](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-9-6) | Atlas basic info semantics | `AV2-5.9.5-ATLAS-BASIC-INFO` | ✅ | ✅ | ✅ | ✅ | 2 |
| [§ 6.10](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-10) | Operating point set OBU semantics | `AV2-5.10-OPERATING-POINT-SET` | ✅ | ✅ | 🟡 | ✅ | 6 |
| [§ 6.10](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-10) | Operating point set OBU semantics | `AV2-5.11-OPERATING-POINT-PAYLOAD` | ✅ | ✅ | 🟡 | ✅ | 2 |
| [§ 6.10.2](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-10-2) | Operating point set OBU syntax elements | `AV2-5.10-OPS-SYNTAX-ELEMENTS` | ✅ | ✅ | 🟡 | ✅ | 2 |
| [§ 6.10.3](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-10-3) | Operating point set aggregate info semantics | `AV2-5.11.1-OPS-AGGREGATE-INFO` | ✅ | ✅ | 🟡 | ✅ | — |
| [§ 6.10.4](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-10-4) | Operating point set sequence profile tier level information semantics | `AV2-5.11.2-OPS-SEQ-PTL-INFO` | ✅ | ✅ | 🟡 | ✅ | 1 |
| [§ 6.10.5](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-10-5) | Operating point set decoder model info semantics | `AV2-5.11.3-OPS-DECODER-MODEL-INFO` | ✅ | ✅ | 🟡 | ✅ | 2 |
| [§ 6.10.6](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-10-6) | Operating point set color info semantics | `AV2-5.11.4-OPS-COLOR-INFO` | ✅ | ✅ | 🟡 | ✅ | — |
| [§ 6.10.7](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-10-7) | Operating point set mlayer info semantics | `AV2-5.11.5-OPS-MLAYER-INFO` | ✅ | ✅ | 🟡 | ✅ | 1 |
| [§ 6.11](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-11) | Buffer removal timing OBU semantics | `AV2-5.12-BUFFER-REMOVAL-TIMING` | ✅ | ✅ | 🟡 | ✅ | 2 |
| [§ 6.12](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-12) | Quantizer Matrix OBU semantics | `AV2-5.13-QUANTIZATION-MATRIX` | ✅ | ✅ | 🟡 | ✅ | 3 |
| [§ 6.13](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-13) | Film grain OBU semantics | `AV2-5.14-FILM-GRAIN` | ✅ | ✅ | 🟡 | ✅ | 6 |
| [§ 6.14](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-14) | Content interpretation OBU semantics | `AV2-5.15-CONTENT-INTERPRETATION` | ✅ | ✅ | ✅ | ✅ | 4 |
| [§ 6.15](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-15) | Padding OBU semantics | `AV2-5.16-PADDING` | ✅ | ✅ | ✅ | ✅ | 2 |
| [§ 6.16](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-16) | Metadata OBU semantics | `AV2-5.17-METADATA` | ✅ | ✅ | 🟡 | ✅ | 20 |
| [§ 6.16.1](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-16-1) | Metadata unit semantics | `AV2-5.17.1-METADATA-UNIT` | ✅ | ✅ | 🟡 | ✅ | 1 |
| [§ 6.16.2](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-16-2) | Metadata short OBU semantics | `AV2-5.17.2-METADATA-SHORT` | ✅ | ✅ | 🟡 | ✅ | 2 |
| [§ 6.16.3](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-16-3) | Metadata group OBU semantics | `AV2-5.17.3-METADATA-GROUP` | ✅ | ✅ | ✅ | ✅ | 7 |
| [§ 6.16.4](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-16-4) | Metadata ITUT T35 semantics | `AV2-5.17.4-METADATA-ITUT-T35` | ✅ | ✅ | ✅ | ✅ | 4 |
| [§ 6.16.5](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-16-5) | Metadata high dynamic range content light level semantics | `AV2-5.17.5-METADATA-HDR-CLL` | ✅ | ✅ | 🟡 | ✅ | 2 |
| [§ 6.16.6](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-16-6) | Metadata high dynamic range mastering display color volume semantics | `AV2-5.17.6-METADATA-HDR-MDCV` | ✅ | ✅ | 🟡 | ✅ | 2 |
| [§ 6.16.7](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-16-7) | Metadata timecode semantics | `AV2-5.17.7-METADATA-TIMECODE` | ✅ | ✅ | 🟡 | ✅ | 5 |
| [§ 6.16.8](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-16-8) | Metadata banding hints semantics | `AV2-5.17.8-METADATA-BANDING-HINTS` | ✅ | ✅ | ✅ | ✅ | 4 |
| [§ 6.16.9](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-16-9) | Metadata ICC profile semantics | `AV2-5.17.9-METADATA-ICC-PROFILE` | ✅ | ✅ | ✅ | ✅ | 4 |
| [§ 6.16.10](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-16-10) | Metadata scan type semantics | `AV2-5.17.10-METADATA-SCAN-TYPE` | ✅ | ✅ | ✅ | ✅ | 5 |
| [§ 6.16.11](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-16-11) | Metadata temporal point info semantics | `AV2-5.17.11-METADATA-TEMPORAL-POINT-INFO` | ✅ | ✅ | ✅ | ✅ | 1 |
| [§ 6.16.12](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-16-12) | Metadata user data unregistered semantics | `AV2-5.17.13-METADATA-USER-DATA-UNREGISTERED` | ✅ | ✅ | ✅ | ✅ | 4 |
| [§ 6.16.13](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-16-13) | Metadata decoded frame hash semantics | `AV2-5.17.12-METADATA-DECODED-FRAME-HASH` | ✅ | ✅ | 🟡 | ✅ | 1 |
| [§ 6.17](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-17) | Frame header OBU semantics | `AV2-5.18-FRAME-HEADER` | ✅ | 🟡 | 🟡 | ✅ | 4 |
| [§ 6.17](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-17) | Frame header OBU semantics | `AV2-5.18.1-FRAME-HEADER-GENERAL` | ✅ | 🟡 | 🟡 | ✅ | 2 |
| [§ 6.17](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-17) | Frame header OBU semantics | `AV2-5.18.8-TRANSFORM-CODING-MODES` | ✅ | 🟡 | — | ✅ | — |
| [§ 6.17](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-17) | Frame header OBU semantics | `AV2-5.18.9-GLOBAL-MOTION` | ✅ | 🟡 | — | ✅ | — |
| [§ 6.17.2](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-17-2) | Frame header info semantics | `AV2-5.18.2-FRAME-HEADER-INFO` | ✅ | 🟡 | 🟡 | ✅ | 9 |
| [§ 6.17.2](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-17-2) | Frame header info semantics | `AV2-6.17.2-FRAME-HEADER-INFO-SEMANTICS` | ✅ | — | 🟡 | ✅ | 15 |
| [§ 6.17.2](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-17-2) | Frame header info semantics | `AV2-7.23-REFERENCE-FRAME-UPDATE` | ✅ | — | 🟡 | ✅ | 1 |
| [§ 6.17.2](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-17-2) | Frame header info semantics | `AV2-7.3.9-LONG-TERM-REFERENCE-AVAILABILITY` | ✅ | — | 🟡 | 🟡 | 1 |
| [§ 6.17.4.1](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-17-4-1) | Frame size semantics | `AV2-5.18.4-FRAME-SIZE` | ✅ | 🟡 | 🟡 | ✅ | 2 |
| [§ 6.17.4.1](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-17-4-1) | Frame size semantics | `AV2-6.17.2-FRAME-HEADER-INFO-SEMANTICS` | ✅ | — | 🟡 | ✅ | 15 |
| [§ 6.17.5.2](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-17-5-2) | Deblocking filter params semantics | `AV2-5.18.5-FILTERING` | ✅ | 🟡 | — | 🟡 | — |
| [§ 6.17.6](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-17-6) | Quantization structures | `AV2-5.18.6-QUANTIZATION` | ✅ | 🟡 | 🟡 | 🟡 | 2 |
| [§ 6.17.7](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-17-7) | Segmentation and tiling structures | `AV2-5.18.7-SEGMENTATION-TILING` | ✅ | 🟡 | 🟡 | 🟡 | 5 |
| [§ 6.17.7](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-17-7) | Segmentation and tiling structures | `AV2-5.18.7.3-TILE-PARAMS` | ✅ | ✅ | ✅ | ✅ | 4 |
| [§ 6.17.10.1](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-17-10-1) | Film grain config semantics | `AV2-5.18.10-FILM-GRAIN-STRUCTURES` | ✅ | ✅ | 🟡 | ✅ | 1 |
| [§ 6.17.10.2](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-17-10-2) | Film grain model semantics | `AV2-5.14-FILM-GRAIN` | ✅ | ✅ | 🟡 | ✅ | 6 |
| [§ 6.18](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-18) | Tile group OBU semantics | `AV2-5.19-TILE-GROUP` | ✅ | 🟡 | 🟡 | ✅ | 7 |
| [§ 6.19](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-19) | Tile group payload semantics | `AV2-5.19-TILE-GROUP` | ✅ | 🟡 | 🟡 | ✅ | 7 |
| [§ 6.19](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-19) | Tile group payload semantics | `AV2-5.20-TILE-GROUP-PAYLOAD` | ✅ | 🟡 | 🟡 | ✅ | 3 |

## Chapter 7 — Decoding process

| Section | Spec item | Feature | Mapped | Parse | Validate | Tests | Diagnostics |
|---|---|---|:-:|:-:|:-:|:-:|:-:|
| [§ 7.1](./spec/av2/1.0.0/07-decoding-process.md#s-7-1) | General decoding process | `CLI-DECODE` | ✅ | — | — | ✅ | 1 |
| [§ 7.1](./spec/av2/1.0.0/07-decoding-process.md#s-7-1) | General decoding process | `CLI-DECODE-HASH-OUTPUT` | ✅ | — | — | ✅ | 1 |
| [§ 7.3](./spec/av2/1.0.0/07-decoding-process.md#s-7-3) | Ordering of OBUs | `AV2-7.3-OBU-ORDERING` | ✅ | — | 🟡 | ✅ | 4 |
| [§ 7.3.2](./spec/av2/1.0.0/07-decoding-process.md#s-7-3-2) | Coded multistream video sequence boundaries | `AV2-7.3.2-CMVS-BOUNDARIES` | ✅ | — | 🟡 | 🟡 | 1 |
| [§ 7.3.3](./spec/av2/1.0.0/07-decoding-process.md#s-7-3-3) | Coded output frame unit | `AV2-7.3.3-CODED-OUTPUT-FRAME-UNIT` | ✅ | — | 🟡 | ✅ | 7 |
| [§ 7.3.4](./spec/av2/1.0.0/07-decoding-process.md#s-7-3-4) | Coded non-output frame unit | `AV2-7.3.4-CODED-NONOUTPUT-FRAME-UNIT` | ✅ | — | 🟡 | ✅ | 1 |
| [§ 7.3.5](./spec/av2/1.0.0/07-decoding-process.md#s-7-3-5) | Coded frame unit | `AV2-7.3.5-CODED-FRAME-UNIT` | ✅ | — | 🟡 | ✅ | — |
| [§ 7.3.6](./spec/av2/1.0.0/07-decoding-process.md#s-7-3-6) | Coded extended layer unit | `AV2-7.3.6-CODED-EXTENDED-LAYER-UNIT` | ✅ | — | 🟡 | ✅ | 13 |
| [§ 7.3.7](./spec/av2/1.0.0/07-decoding-process.md#s-7-3-7) | Temporal unit | `AV2-7.3.7-TEMPORAL-UNIT-ORDER` | ✅ | ✅ | 🟡 | ✅ | 7 |
| [§ 7.3.8](./spec/av2/1.0.0/07-decoding-process.md#s-7-3-8) | Availability of high level syntax OBUs | `AV2-6.2.2-OBU-HEADER-ACTIVATED-SEQUENCE-LIMITS` | ✅ | ✅ | 🟡 | ✅ | 3 |
| [§ 7.3.8](./spec/av2/1.0.0/07-decoding-process.md#s-7-3-8) | Availability of high level syntax OBUs | `AV2-7.3.8-HLS-AVAILABILITY` | ✅ | ✅ | 🟡 | ✅ | 14 |
| [§ 7.3.8.10](./spec/av2/1.0.0/07-decoding-process.md#s-7-3-8-10) | Content interpretation OBU availability | `AV2-7.3.3-CODED-OUTPUT-FRAME-UNIT` | ✅ | — | 🟡 | ✅ | 7 |
| [§ 7.3.9](./spec/av2/1.0.0/07-decoding-process.md#s-7-3-9) | Availability of long-term reference frames | `AV2-7.3.9-LONG-TERM-REFERENCE-AVAILABILITY` | ✅ | — | 🟡 | 🟡 | 1 |
| [§ 7.3.9](./spec/av2/1.0.0/07-decoding-process.md#s-7-3-9) | Availability of long-term reference frames | `AV2-7.4-RANDOM-ACCESS` | ✅ | — | 🟡 | 🟡 | 2 |
| [§ 7.4.2](./spec/av2/1.0.0/07-decoding-process.md#s-7-4-2) | Random access and use of long-term reference frames | `AV2-7.4-RANDOM-ACCESS` | ✅ | — | 🟡 | 🟡 | 2 |
| [§ 7.4.4](./spec/av2/1.0.0/07-decoding-process.md#s-7-4-4) | Open Random Access | `AV2-7.4-RANDOM-ACCESS` | ✅ | — | 🟡 | 🟡 | 2 |
| [§ 7.4.5](./spec/av2/1.0.0/07-decoding-process.md#s-7-4-5) | Random Access Switch | `AV2-7.3.9-LONG-TERM-REFERENCE-AVAILABILITY` | ✅ | — | 🟡 | 🟡 | 1 |
| [§ 7.4.5](./spec/av2/1.0.0/07-decoding-process.md#s-7-4-5) | Random Access Switch | `AV2-7.4-RANDOM-ACCESS` | ✅ | — | 🟡 | 🟡 | 2 |
| [§ 7.4.6](./spec/av2/1.0.0/07-decoding-process.md#s-7-4-6) | Multistream Random Access | `AV2-7.3.7-TEMPORAL-UNIT-ORDER` | ✅ | ✅ | 🟡 | ✅ | 7 |
| [§ 7.21](./spec/av2/1.0.0/07-decoding-process.md#s-7-21) | Output processes | `CLI-DECODE-HASH-OUTPUT` | ✅ | — | — | ✅ | 1 |
| [§ 7.23](./spec/av2/1.0.0/07-decoding-process.md#s-7-23) | Reference frame update process | `AV2-7.23-REFERENCE-FRAME-UPDATE` | ✅ | — | 🟡 | ✅ | 1 |

## Chapter 9 — Additional tables

| Section | Spec item | Feature | Mapped | Parse | Validate | Tests | Diagnostics |
|---|---|---|:-:|:-:|:-:|:-:|:-:|
| [§ 9](./spec/av2/1.0.0/09-additional-tables/09-00-overview.md#s-9) | Additional tables | `AV2-9-ADDITIONAL-TABLES` | ✅ | 🟡 | — | 🟡 | — |

## Annexes

| Section | Spec item | Feature | Mapped | Parse | Validate | Tests | Diagnostics |
|---|---|---|:-:|:-:|:-:|:-:|:-:|
| [Annex A](./spec/av2/1.0.0/annex-a-profiles-levels-and-tiers.md#s-annex-a) | Profiles, levels, and tiers | `AV2-A-LEVELS-TIERS` | ✅ | — | 🟡 | ✅ | 5 |
| [Annex A](./spec/av2/1.0.0/annex-a-profiles-levels-and-tiers.md#s-annex-a) | Profiles, levels, and tiers | `AV2-A-PROFILES` | ✅ | — | 🟡 | ✅ | 7 |
| [Annex B](./spec/av2/1.0.0/annex-b-length-delimited-bitstream-format.md#s-annex-b) | Length delimited bitstream format | `AV2-B-ANNEXB-OBU-ENVELOPE` | ✅ | ✅ | ✅ | ✅ | 1 |
| [Annex E](./spec/av2/1.0.0/annex-e-decoder-model.md#s-annex-e) | Decoder model | `AV2-E-DECODER-MODEL` | ✅ | — |  |  | — |

## Features without a spec section

40 feature(s) track conformance, encoder, CLI, automation, or documentation work with no single spec section; see [FEATURE-STATUS.md](./FEATURE-STATUS.md):

- `AV2-IVF-CONTAINER` — IVF container support
- `CLI-INSPECT` — splot inspect command
- `CLI-VALIDATE` — splot validate command
- `CONF-AVM-DIFF-HARNESS` — AVM differential testing harness
- `CONF-AVM-INVALID-STREAMS` — AVM invalid stream conformance
- `CONF-AVM-PARSER-TRACES` — AVM parser trace comparison
- `CONF-AVM-VALID-STREAMS` — AVM valid stream conformance
- `CONF-FUZZ-NO-PANIC` — Parser no-panic fuzzing
- `CONF-INSPECT-SNAPSHOTS` — Inspector snapshot tests
- `CONF-PUBLIC-VECTOR-LICENSE-REVIEW` — Public vector license review
- `CONF-PUBLIC-VECTORS` — Public AV2 vector corpus integration
- `DOC-AUDIT-PROTOCOLS` — Agent audit protocol skills
- `DOC-AV2-SPEC-MIRROR` — AV2 specification mirror
- `DOC-DECODE-LIMITS-CONTRACT` — Decode limits contract documentation
- `DOC-DECODED-FRAME-PLANE-MODEL-CONTRACT` — Decoded frame and plane model contract documentation
- `DOC-DECODER-DIAGNOSTICS` — Decoder diagnostics registry documentation
- `DOC-DECODER-ROADMAP` — Decoder roadmap documentation
- `DOC-DECODER-SUPPORT-MATRIX` — Decoder support matrix documentation
- `DOC-DETERMINISTIC-FRAME-HASH-CONTRACT` — Deterministic decoded-frame hash contract documentation
- `DOC-ENCODER-REFERENCE-GATE` — Encoder reference gate documentation
- `DOC-FEATURE-TRACKING` — Feature tracking documentation
- `DOC-MINIMAL-DECODE-TIER-CONTRACT` — Minimal decode tier contract documentation
- `DOC-VALIDATOR-ROADMAP` — Validator coverage roadmap documentation
- `ENC-BITSTREAM-WRITER` — Bitstream writer foundation
- `ENC-INTRA-TOY-V0` — Minimal toy intra encoder path
- `ENC-RATE-CONTROL-V0` — Initial rate control strategy
- `ENC-SPEED-PRESETS` — Encoder speed preset framework
- `ENC-Y4M-INPUT` — Y4M input reader integration
- `VALIDATOR-CONTEXT-SPLIT` — Validator context module split
- `XTASK-AUDIT-SCOPE` — Changed-file AV2 audit scope
- `XTASK-CI-QUALITY-GATES` — CI quality gates (docs build + coverage threshold)
- `XTASK-CONVENTIONAL-COMMITS` — Conventional commit enforcement
- `XTASK-DECODER-DIAGNOSTIC-REGISTRY` — Decoder diagnostic registry enforcement
- `XTASK-DECODER-SUPPORT-STATUS` — Decoder support status reporting and checks
- `XTASK-DIAGNOSTIC-REGISTRY` — Validator diagnostic registry enforcement
- `XTASK-FEATURE-STATUS` — xtask feature status reporting and checks
- `XTASK-GEN-TABLES` — AV2 § 9 tables code generator
- `XTASK-LOCAL-REFERENCE-EVIDENCE-MANIFEST` — Portable local-reference evidence manifest
- `XTASK-SOURCE-LINES` — Rust source-file line budget
- `XTASK-VALIDATOR-MODULE-SPLIT` — Validator module split
