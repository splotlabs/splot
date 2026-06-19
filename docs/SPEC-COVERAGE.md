# Spec coverage

Generated from `docs/IMPLEMENTATION-MATRIX.toml` by `cargo xtask spec-coverage --format markdown --output docs/SPEC-COVERAGE.md`. Do not edit by hand.

Matrix version 1. Last reviewed 2026-06-15. 319 feature(s); 266 cite a spec section.

One row per (spec section, feature) pair, in spec order; a feature citing both a syntax and a semantics section appears under both. The canonical status source is [IMPLEMENTATION-MATRIX.toml](./IMPLEMENTATION-MATRIX.toml); the full per-feature ledger is [FEATURE-STATUS.md](./FEATURE-STATUS.md).

Legend: ✅ done · 🟡 partial · ⏳ pending external proof · ⛔ blocked · 🧪 experimental · — not applicable · (blank) todo. Diagnostics counts the rule IDs recorded in the feature's proof; emitted validator IDs are registered in [VALIDATOR-DIAGNOSTICS.md](./VALIDATOR-DIAGNOSTICS.md), and emitted decoder IDs are registered in [DECODER-DIAGNOSTICS.md](./DECODER-DIAGNOSTICS.md).

## Chapter 3 — Symbols and abbreviated terms

| Section | Spec item | Feature | Mapped | Parse | Validate | Tests | Diagnostics |
|---|---|---|:-:|:-:|:-:|:-:|:-:|
| [§ 3](./spec/av2/1.0.0/03-symbols.md#s-3) | Symbols | `AV2-8.2-SYMBOL-DECODER` | ✅ | ✅ | ✅ | ✅ | — |
| [§ 3](./spec/av2/1.0.0/03-symbols.md#s-3) | Symbols | `CONF-RECON-INTRA-PREDICTION-FUZZ` | ✅ | — | — | ✅ | — |
| [§ 3](./spec/av2/1.0.0/03-symbols.md#s-3) | Symbols | `CONF-RECON-REFERENCE-FRAME-STORE-FUZZ` | ✅ | — | — | ✅ | — |
| [§ 3](./spec/av2/1.0.0/03-symbols.md#s-3) | Symbols | `DECODE-COEFF-BASE-DERIVED-LEVEL-PASS` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 3](./spec/av2/1.0.0/03-symbols.md#s-3) | Symbols | `DECODE-COEFF-CDF-Q-CONTEXT-HANDOFF` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 3](./spec/av2/1.0.0/03-symbols.md#s-3) | Symbols | `DECODE-COEFF-MAX-LEVEL-DERIVE` | ✅ | — | — | ✅ | 1 |
| [§ 3](./spec/av2/1.0.0/03-symbols.md#s-3) | Symbols | `DECODE-COEFF-ORDINARY-BRANCH-CHROMA-INTER-TXTYPES-HANDOFF` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 3](./spec/av2/1.0.0/03-symbols.md#s-3) | Symbols | `DECODE-COEFF-ORDINARY-BRANCH-DIRECTIONAL-UV-HANDOFF` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 3](./spec/av2/1.0.0/03-symbols.md#s-3) | Symbols | `DECODE-COEFF-ORDINARY-BRANCH-LUMA-TXTYPES-HANDOFF` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 3](./spec/av2/1.0.0/03-symbols.md#s-3) | Symbols | `DECODE-COEFF-PARITY-TCQ-HANDOFF` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 3](./spec/av2/1.0.0/03-symbols.md#s-3) | Symbols | `DECODE-COEFF-USE-FSC-CONDITION-HANDOFF` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 3](./spec/av2/1.0.0/03-symbols.md#s-3) | Symbols | `DECODE-COEFF-USE-FSC-SHARED-FACTS-HANDOFF` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 3](./spec/av2/1.0.0/03-symbols.md#s-3) | Symbols | `DECODE-MODE-TO-TXFM-SYMBOLIC-TABLE` | ✅ | — | — | ✅ | 1 |
| [§ 3](./spec/av2/1.0.0/03-symbols.md#s-3) | Symbols | `RECON-DEQUANT-PROCESS` | ✅ | — | ✅ | ✅ | — |
| [§ 3](./spec/av2/1.0.0/03-symbols.md#s-3) | Symbols | `RECON-DEQUANT-QUANTIZER-INDEX-RESOLUTION` | ✅ | — | — | ✅ | — |
| [§ 3](./spec/av2/1.0.0/03-symbols.md#s-3) | Symbols | `RECON-DEQUANT-QUANTIZER-LOOKUP` | ✅ | — | — | ✅ | — |
| [§ 3](./spec/av2/1.0.0/03-symbols.md#s-3) | Symbols | `RECON-INTRA-DC-RECTANGULAR-PREDICTION` | ✅ | — | ✅ | ✅ | — |
| [§ 3](./spec/av2/1.0.0/03-symbols.md#s-3) | Symbols | `RECON-INTRA-DC-SUBSAMPLED-PREDICTION` | ✅ | — | ✅ | ✅ | — |
| [§ 3](./spec/av2/1.0.0/03-symbols.md#s-3) | Symbols | `RECON-INTRA-IBP-DC-PREDICTION` | ✅ | — | ✅ | ✅ | — |
| [§ 3](./spec/av2/1.0.0/03-symbols.md#s-3) | Symbols | `RECON-INTRA-SMOOTH-PREDICTION` | ✅ | — | ✅ | ✅ | — |
| [§ 3](./spec/av2/1.0.0/03-symbols.md#s-3) | Symbols | `RECON-INVERSE-TRANSFORM-1D` | ✅ | — | ✅ | ✅ | — |
| [§ 3](./spec/av2/1.0.0/03-symbols.md#s-3) | Symbols | `RECON-INVERSE-TRANSFORM-MATRIX-FREE` | ✅ | — | ✅ | ✅ | — |
| [§ 3](./spec/av2/1.0.0/03-symbols.md#s-3) | Symbols | `RECON-REFERENCE-FRAME-STORE` | ✅ | — | — | ✅ | — |

## Chapter 4 — Conventions

| Section | Spec item | Feature | Mapped | Parse | Validate | Tests | Diagnostics |
|---|---|---|:-:|:-:|:-:|:-:|:-:|
| [§ 4.8](./spec/av2/1.0.0/04-conventions.md#s-4-8) | Mathematical functions | `CONF-RECON-INTRA-PREDICTION-FUZZ` | ✅ | — | — | ✅ | — |
| [§ 4.8](./spec/av2/1.0.0/04-conventions.md#s-4-8) | Mathematical functions | `RECON-DEQUANT-PROCESS` | ✅ | — | ✅ | ✅ | — |
| [§ 4.8](./spec/av2/1.0.0/04-conventions.md#s-4-8) | Mathematical functions | `RECON-DEQUANT-QM-WEIGHT` | ✅ | — | ✅ | ✅ | — |
| [§ 4.8](./spec/av2/1.0.0/04-conventions.md#s-4-8) | Mathematical functions | `RECON-INTRA-DC-RECTANGULAR-PREDICTION` | ✅ | — | ✅ | ✅ | — |
| [§ 4.8](./spec/av2/1.0.0/04-conventions.md#s-4-8) | Mathematical functions | `RECON-INTRA-DC-SUBSAMPLED-PREDICTION` | ✅ | — | ✅ | ✅ | — |
| [§ 4.8](./spec/av2/1.0.0/04-conventions.md#s-4-8) | Mathematical functions | `RECON-INTRA-IBP-DC-PREDICTION` | ✅ | — | ✅ | ✅ | — |
| [§ 4.8](./spec/av2/1.0.0/04-conventions.md#s-4-8) | Mathematical functions | `RECON-INTRA-MIDDLE-DIRECTIONAL-ANGLE-PREDICTION` | ✅ | — | ✅ | ✅ | — |
| [§ 4.8](./spec/av2/1.0.0/04-conventions.md#s-4-8) | Mathematical functions | `RECON-INTRA-ONE-SIDED-DIRECTIONAL-ANGLE-PREDICTION` | ✅ | — | ✅ | ✅ | — |
| [§ 4.8](./spec/av2/1.0.0/04-conventions.md#s-4-8) | Mathematical functions | `RECON-INTRA-SMOOTH-PREDICTION` | ✅ | — | ✅ | ✅ | — |
| [§ 4.8](./spec/av2/1.0.0/04-conventions.md#s-4-8) | Mathematical functions | `RECON-INVERSE-TRANSFORM-1D` | ✅ | — | ✅ | ✅ | — |
| [§ 4.8](./spec/av2/1.0.0/04-conventions.md#s-4-8) | Mathematical functions | `RECON-INVERSE-TRANSFORM-2D` | ✅ | — | ✅ | ✅ | — |
| [§ 4.8](./spec/av2/1.0.0/04-conventions.md#s-4-8) | Mathematical functions | `RECON-INVERSE-TRANSFORM-2D-OUTER` | ✅ | — | ✅ | ✅ | — |
| [§ 4.8](./spec/av2/1.0.0/04-conventions.md#s-4-8) | Mathematical functions | `RECON-INVERSE-TRANSFORM-MATRIX-FREE` | ✅ | — | ✅ | ✅ | — |
| [§ 4.8](./spec/av2/1.0.0/04-conventions.md#s-4-8) | Mathematical functions | `RECON-RESIDUAL-ADDITION` | ✅ | — | ✅ | ✅ | — |
| [§ 4.8](./spec/av2/1.0.0/04-conventions.md#s-4-8) | Mathematical functions | `RECON-WORKSPACE-DIRECTIONAL-ANGLE-PREDICTION` | ✅ | — | ✅ | ✅ | — |
| [§ 4.11.3](./spec/av2/1.0.0/04-conventions.md#s-4-11-3) | uvlc() | `AV2-4.11.3-UVLC` | ✅ | ✅ | — | ✅ | — |
| [§ 4.11.4](./spec/av2/1.0.0/04-conventions.md#s-4-11-4) | svlc() | `AV2-4.11.4-SVLC` | ✅ | ✅ | — | ✅ | — |
| [§ 4.11.5](./spec/av2/1.0.0/04-conventions.md#s-4-11-5) | le(n) | `AV2-4.11.5-LE` | ✅ | ✅ | — | ✅ | — |
| [§ 4.11.6](./spec/av2/1.0.0/04-conventions.md#s-4-11-6) | leb128() | `AV2-4.11.6-LEB128` | ✅ | ✅ | ✅ | ✅ | 1 |
| [§ 4.11.6](./spec/av2/1.0.0/04-conventions.md#s-4-11-6) | leb128() | `DECODE-BYTE-STREAM-PLANNER` | ✅ | ✅ | — | ✅ | 3 |
| [§ 4.11.6](./spec/av2/1.0.0/04-conventions.md#s-4-11-6) | leb128() | `DECODE-LIMITS-RUNTIME-API` | ✅ | — | — | ✅ | — |
| [§ 4.11.7](./spec/av2/1.0.0/04-conventions.md#s-4-11-7) | su(n) | `AV2-4.11.7-SU` | ✅ | ✅ | — | ✅ | — |
| [§ 4.11.8](./spec/av2/1.0.0/04-conventions.md#s-4-11-8) | ns(n) | `AV2-4.11.8-NS` | ✅ | ✅ | — | ✅ | — |
| [§ 4.11.10](./spec/av2/1.0.0/04-conventions.md#s-4-11-10) | rg(n) | `AV2-4.11.10-RG` | ✅ | ✅ | — | ✅ | — |
| [§ 4.11.11](./spec/av2/1.0.0/04-conventions.md#s-4-11-11) | L(n) | `CONF-SYMBOL-DECODER-FUZZ` | ✅ | — | — | ✅ | — |
| [§ 4.11.12](./spec/av2/1.0.0/04-conventions.md#s-4-11-12) | S() | `CONF-SYMBOL-DECODER-FUZZ` | ✅ | — | — | ✅ | — |

## Chapter 5 — Syntax structures

| Section | Spec item | Feature | Mapped | Parse | Validate | Tests | Diagnostics |
|---|---|---|:-:|:-:|:-:|:-:|:-:|
| [§ 5.2](./spec/av2/1.0.0/05-syntax-structures.md#s-5-2) | OBU syntax | `CONF-DECODE-RUNTIME-HASH-FUZZ` | ✅ | — | — | ✅ | 3 |
| [§ 5.2](./spec/av2/1.0.0/05-syntax-structures.md#s-5-2) | OBU syntax | `CONF-DECODE-RUNTIME-RAW-FUZZ` | ✅ | — | — | ✅ | 4 |
| [§ 5.2](./spec/av2/1.0.0/05-syntax-structures.md#s-5-2) | OBU syntax | `CONF-DECODE-RUNTIME-Y4M-FUZZ` | ✅ | — | — | ✅ | 4 |
| [§ 5.2](./spec/av2/1.0.0/05-syntax-structures.md#s-5-2) | OBU syntax | `DECODE-MINIMAL-RAW-RUNTIME-OUTPUT` | ✅ | 🟡 | ✅ | ✅ | 4 |
| [§ 5.2](./spec/av2/1.0.0/05-syntax-structures.md#s-5-2) | OBU syntax | `DECODE-MINIMAL-TIER-RUNTIME-SUCCESS` | ✅ | 🟡 | ✅ | ✅ | 3 |
| [§ 5.2](./spec/av2/1.0.0/05-syntax-structures.md#s-5-2) | OBU syntax | `DECODE-Y4M-RUNTIME-OUTPUT` | ✅ | 🟡 | ✅ | ✅ | 4 |
| [§ 5.2.1](./spec/av2/1.0.0/05-syntax-structures.md#s-5-2-1) | General OBU syntax | `AV2-5.2.1-OBU-DISPATCH` | ✅ | 🟡 | — | ✅ | — |
| [§ 5.2.1](./spec/av2/1.0.0/05-syntax-structures.md#s-5-2-1) | General OBU syntax | `AV2-5.2.1-OBU-TYPE` | ✅ | ✅ | ✅ | ✅ | 5 |
| [§ 5.2.1](./spec/av2/1.0.0/05-syntax-structures.md#s-5-2-1) | General OBU syntax | `DECODE-BYTE-STREAM-PLANNER` | ✅ | ✅ | — | ✅ | 3 |
| [§ 5.2.1](./spec/av2/1.0.0/05-syntax-structures.md#s-5-2-1) | General OBU syntax | `DECODE-LIMITS-RUNTIME-API` | ✅ | — | — | ✅ | — |
| [§ 5.2.1](./spec/av2/1.0.0/05-syntax-structures.md#s-5-2-1) | General OBU syntax | `DECODE-STREAM-STATE-PLANNER` | ✅ | — | — | ✅ | 1 |
| [§ 5.2.1](./spec/av2/1.0.0/05-syntax-structures.md#s-5-2-1) | General OBU syntax | `DECODE-TILE-PAYLOAD-INPUT-DERIVATION` | ✅ | 🟡 | — | ✅ | 2 |
| [§ 5.2.2](./spec/av2/1.0.0/05-syntax-structures.md#s-5-2-2) | OBU header syntax | `AV2-5.2.2-OBU-HEADER` | ✅ | ✅ | ✅ | ✅ | 6 |
| [§ 5.2.2](./spec/av2/1.0.0/05-syntax-structures.md#s-5-2-2) | OBU header syntax | `DECODE-BYTE-STREAM-PLANNER` | ✅ | ✅ | — | ✅ | 3 |
| [§ 5.2.3](./spec/av2/1.0.0/05-syntax-structures.md#s-5-2-3) | Trailing bits syntax | `AV2-5.2.3-TRAILING-BITS` | ✅ | ✅ | 🟡 | ✅ | 2 |
| [§ 5.2.4](./spec/av2/1.0.0/05-syntax-structures.md#s-5-2-4) | Byte alignment syntax | `AV2-5.2.4-BYTE-ALIGNMENT` | ✅ | ✅ | 🟡 | ✅ | 1 |
| [§ 5.3](./spec/av2/1.0.0/05-syntax-structures.md#s-5-3) | Reserved OBU syntax | `AV2-5.3-RESERVED-OBU` | ✅ | — | ✅ | ✅ | 2 |
| [§ 5.4](./spec/av2/1.0.0/05-syntax-structures.md#s-5-4) | Sequence header OBU syntax | `AV2-5.4-SEQUENCE-HEADER` | ✅ | 🟡 | 🟡 | 🟡 | 1 |
| [§ 5.4](./spec/av2/1.0.0/05-syntax-structures.md#s-5-4) | Sequence header OBU syntax | `ENC-MINIMAL-HEADER-PLAN` | ✅ | — | — | ✅ | — |
| [§ 5.4.1](./spec/av2/1.0.0/05-syntax-structures.md#s-5-4-1) | General sequence header OBU syntax | `AV2-5.4.1-SEQUENCE-HEADER-GENERAL` | ✅ | ✅ | ✅ | ✅ | 6 |
| [§ 5.4.2](./spec/av2/1.0.0/05-syntax-structures.md#s-5-4-2) | Sequence tile config syntax | `AV2-5.4.2-SEQUENCE-TILE-CONFIG` | ✅ | ✅ | ✅ | ✅ | 4 |
| [§ 5.4.3](./spec/av2/1.0.0/05-syntax-structures.md#s-5-4-3) | Sequence partition config syntax | `AV2-5.4.3-SEQUENCE-PARTITION-CONFIG` | ✅ | ✅ | 🟡 | ✅ | — |
| [§ 5.4.4](./spec/av2/1.0.0/05-syntax-structures.md#s-5-4-4) | Sequence segment config syntax | `AV2-5.4.4-SEQUENCE-SEGMENT-CONFIG` | ✅ | ✅ | 🟡 | ✅ | — |
| [§ 5.4.5](./spec/av2/1.0.0/05-syntax-structures.md#s-5-4-5) | Sequence intra config syntax | `AV2-5.4.5-SEQUENCE-INTRA-CONFIG` | ✅ | ✅ | 🟡 | ✅ | — |
| [§ 5.4.6](./spec/av2/1.0.0/05-syntax-structures.md#s-5-4-6) | Sequence inter config syntax | `AV2-5.4.6-SEQUENCE-INTER-CONFIG` | ✅ | ✅ | ✅ | ✅ | 1 |
| [§ 5.4.6](./spec/av2/1.0.0/05-syntax-structures.md#s-5-4-6) | Sequence inter config syntax | `RECON-REFERENCE-FRAME-STORE` | ✅ | — | — | ✅ | — |
| [§ 5.4.7](./spec/av2/1.0.0/05-syntax-structures.md#s-5-4-7) | Sequence screen content config syntax | `AV2-5.4.7-SEQUENCE-SCC-CONFIG` | ✅ | ✅ | 🟡 | ✅ | — |
| [§ 5.4.8](./spec/av2/1.0.0/05-syntax-structures.md#s-5-4-8) | Sequence transform quant entropy config syntax | `AV2-5.4.8-SEQUENCE-TQ-ENTROPY-CONFIG` | ✅ | ✅ | 🟡 | ✅ | — |
| [§ 5.4.8](./spec/av2/1.0.0/05-syntax-structures.md#s-5-4-8) | Sequence transform quant entropy config syntax | `DECODE-COEFF-FRAME-FACTS-HANDOFF` | ✅ | 🟡 | — | ✅ | 1 |
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
| [§ 5.8.3](./spec/av2/1.0.0/05-syntax-structures.md#s-5-8-3) | LCR aggregate info syntax | `AV2-5.8.3-LCR-AGGREGATE-INFO` | ✅ | ✅ | ✅ | ✅ | — |
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
| [§ 5.17.12](./spec/av2/1.0.0/05-syntax-structures.md#s-5-17-12) | Metadata decoded frame hash syntax | `DECODE-MINIMAL-RAW-RUNTIME-OUTPUT` | ✅ | 🟡 | ✅ | ✅ | 4 |
| [§ 5.17.12](./spec/av2/1.0.0/05-syntax-structures.md#s-5-17-12) | Metadata decoded frame hash syntax | `DECODE-MINIMAL-TIER-RUNTIME-SUCCESS` | ✅ | 🟡 | ✅ | ✅ | 3 |
| [§ 5.17.12](./spec/av2/1.0.0/05-syntax-structures.md#s-5-17-12) | Metadata decoded frame hash syntax | `DECODE-Y4M-RUNTIME-OUTPUT` | ✅ | 🟡 | ✅ | ✅ | 4 |
| [§ 5.17.12](./spec/av2/1.0.0/05-syntax-structures.md#s-5-17-12) | Metadata decoded frame hash syntax | `DOC-DECODER-OUTPUT-EQUIVALENCE-CONTRACT` | ✅ | — | — | ✅ | — |
| [§ 5.17.13](./spec/av2/1.0.0/05-syntax-structures.md#s-5-17-13) | Metadata user data unregistered syntax | `AV2-5.17.13-METADATA-USER-DATA-UNREGISTERED` | ✅ | ✅ | ✅ | ✅ | 4 |
| [§ 5.18](./spec/av2/1.0.0/05-syntax-structures.md#s-5-18) | Frame header syntax | `AV2-5.18-FRAME-HEADER` | ✅ | 🟡 | 🟡 | ✅ | 4 |
| [§ 5.18](./spec/av2/1.0.0/05-syntax-structures.md#s-5-18) | Frame header syntax | `ENC-MINIMAL-HEADER-PLAN` | ✅ | — | — | ✅ | — |
| [§ 5.18.1](./spec/av2/1.0.0/05-syntax-structures.md#s-5-18-1) | General frame header syntax | `AV2-5.18.1-FRAME-HEADER-GENERAL` | ✅ | 🟡 | 🟡 | ✅ | 2 |
| [§ 5.18.1](./spec/av2/1.0.0/05-syntax-structures.md#s-5-18-1) | General frame header syntax | `DECODE-TILE-PAYLOAD-INPUT-DERIVATION` | ✅ | 🟡 | — | ✅ | 2 |
| [§ 5.18.2](./spec/av2/1.0.0/05-syntax-structures.md#s-5-18-2) | Frame header info syntax | `AV2-5.18.2-FRAME-HEADER-INFO` | ✅ | 🟡 | 🟡 | ✅ | 9 |
| [§ 5.18.2](./spec/av2/1.0.0/05-syntax-structures.md#s-5-18-2) | Frame header info syntax | `DECODE-COEFF-FRAME-FACTS-HANDOFF` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 5.18.2](./spec/av2/1.0.0/05-syntax-structures.md#s-5-18-2) | Frame header info syntax | `DECODE-COEFF-PARITY-TCQ-HANDOFF` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 5.18.2](./spec/av2/1.0.0/05-syntax-structures.md#s-5-18-2) | Frame header info syntax | `DECODE-MINIMAL-RAW-RUNTIME-OUTPUT` | ✅ | 🟡 | ✅ | ✅ | 4 |
| [§ 5.18.2](./spec/av2/1.0.0/05-syntax-structures.md#s-5-18-2) | Frame header info syntax | `DECODE-MINIMAL-TIER-RUNTIME-SUCCESS` | ✅ | 🟡 | ✅ | ✅ | 3 |
| [§ 5.18.2](./spec/av2/1.0.0/05-syntax-structures.md#s-5-18-2) | Frame header info syntax | `DECODE-TILE-PAYLOAD-INPUT-DERIVATION` | ✅ | 🟡 | — | ✅ | 2 |
| [§ 5.18.2](./spec/av2/1.0.0/05-syntax-structures.md#s-5-18-2) | Frame header info syntax | `DECODE-Y4M-RUNTIME-OUTPUT` | ✅ | 🟡 | ✅ | ✅ | 4 |
| [§ 5.18.2](./spec/av2/1.0.0/05-syntax-structures.md#s-5-18-2) | Frame header info syntax | `RECON-REFERENCE-FRAME-STORE` | ✅ | — | — | ✅ | — |
| [§ 5.18.3](./spec/av2/1.0.0/05-syntax-structures.md#s-5-18-3) | Frame configuration structures | `AV2-5.18.3-FRAME-CONFIGURATION` | ✅ | 🟡 | — | ✅ | — |
| [§ 5.18.4](./spec/av2/1.0.0/05-syntax-structures.md#s-5-18-4) | Frame size structures | `AV2-5.18.4-FRAME-SIZE` | ✅ | 🟡 | 🟡 | ✅ | 2 |
| [§ 5.18.4.1](./spec/av2/1.0.0/05-syntax-structures.md#s-5-18-4-1) | Frame size syntax | `DECODE-LIMITS-RUNTIME-API` | ✅ | — | — | ✅ | — |
| [§ 5.18.4.4](./spec/av2/1.0.0/05-syntax-structures.md#s-5-18-4-4) | Compute image size function | `DECODE-LIMITS-RUNTIME-API` | ✅ | — | — | ✅ | — |
| [§ 5.18.5](./spec/av2/1.0.0/05-syntax-structures.md#s-5-18-5) | Filtering structures | `AV2-5.18.5-FILTERING` | ✅ | 🟡 | — | 🟡 | — |
| [§ 5.18.5.2](./spec/av2/1.0.0/05-syntax-structures.md#s-5-18-5-2) | Deblocking filter params syntax | `AV2-5.18.5-FILTERING` | ✅ | 🟡 | — | 🟡 | — |
| [§ 5.18.6](./spec/av2/1.0.0/05-syntax-structures.md#s-5-18-6) | Quantization structures | `AV2-5.18.6-QUANTIZATION` | ✅ | 🟡 | 🟡 | 🟡 | 4 |
| [§ 5.18.6.1](./spec/av2/1.0.0/05-syntax-structures.md#s-5-18-6-1) | Quantization params syntax | `DECODE-TILE-PAYLOAD-INPUT-DERIVATION` | ✅ | 🟡 | — | ✅ | 2 |
| [§ 5.18.7](./spec/av2/1.0.0/05-syntax-structures.md#s-5-18-7) | Segmentation and tiling structures | `AV2-5.18.7-SEGMENTATION-TILING` | ✅ | 🟡 | 🟡 | 🟡 | 5 |
| [§ 5.18.7.2](./spec/av2/1.0.0/05-syntax-structures.md#s-5-18-7-2) | Tile info syntax | `DECODE-LIMITS-RUNTIME-API` | ✅ | — | — | ✅ | — |
| [§ 5.18.7.2](./spec/av2/1.0.0/05-syntax-structures.md#s-5-18-7-2) | Tile info syntax | `DECODE-TILE-PAYLOAD-INPUT-DERIVATION` | ✅ | 🟡 | — | ✅ | 2 |
| [§ 5.18.7.3](./spec/av2/1.0.0/05-syntax-structures.md#s-5-18-7-3) | Tile params syntax | `AV2-5.18.7.3-TILE-PARAMS` | ✅ | ✅ | ✅ | ✅ | 4 |
| [§ 5.18.7.5](./spec/av2/1.0.0/05-syntax-structures.md#s-5-18-7-5) | Uniform spacing function | `AV2-5.18.7.3-TILE-PARAMS` | ✅ | ✅ | ✅ | ✅ | 4 |
| [§ 5.18.7.7](./spec/av2/1.0.0/05-syntax-structures.md#s-5-18-7-7) | Tile size calculation function | `AV2-5.18.7.3-TILE-PARAMS` | ✅ | ✅ | ✅ | ✅ | 4 |
| [§ 5.18.7.8](./spec/av2/1.0.0/05-syntax-structures.md#s-5-18-7-8) | Quantizer index delta parameters syntax | `AV2-5.18.6-QUANTIZATION` | ✅ | 🟡 | 🟡 | 🟡 | 4 |
| [§ 5.18.8](./spec/av2/1.0.0/05-syntax-structures.md#s-5-18-8) | Transform and coding mode structures | `AV2-5.18.8-TRANSFORM-CODING-MODES` | ✅ | 🟡 | — | ✅ | — |
| [§ 5.18.9](./spec/av2/1.0.0/05-syntax-structures.md#s-5-18-9) | Global motion structures | `AV2-5.18.9-GLOBAL-MOTION` | ✅ | 🟡 | — | ✅ | — |
| [§ 5.18.10](./spec/av2/1.0.0/05-syntax-structures.md#s-5-18-10) | Film grain structures | `AV2-5.18.10-FILM-GRAIN-STRUCTURES` | ✅ | ✅ | 🟡 | ✅ | 1 |
| [§ 5.18.10.2](./spec/av2/1.0.0/05-syntax-structures.md#s-5-18-10-2) | Film grain model syntax | `AV2-5.14-FILM-GRAIN` | ✅ | ✅ | 🟡 | ✅ | 6 |
| [§ 5.19](./spec/av2/1.0.0/05-syntax-structures.md#s-5-19) | Tile group OBU syntax | `AV2-5.19-TILE-GROUP` | ✅ | 🟡 | 🟡 | ✅ | 7 |
| [§ 5.19](./spec/av2/1.0.0/05-syntax-structures.md#s-5-19) | Tile group OBU syntax | `CONF-DECODE-RUNTIME-HASH-FUZZ` | ✅ | — | — | ✅ | 3 |
| [§ 5.19](./spec/av2/1.0.0/05-syntax-structures.md#s-5-19) | Tile group OBU syntax | `CONF-DECODE-RUNTIME-RAW-FUZZ` | ✅ | — | — | ✅ | 4 |
| [§ 5.19](./spec/av2/1.0.0/05-syntax-structures.md#s-5-19) | Tile group OBU syntax | `CONF-DECODE-RUNTIME-Y4M-FUZZ` | ✅ | — | — | ✅ | 4 |
| [§ 5.19](./spec/av2/1.0.0/05-syntax-structures.md#s-5-19) | Tile group OBU syntax | `CONF-TILE-PAYLOAD-DECODE-FUZZ` | ✅ | — | — | ✅ | 2 |
| [§ 5.19](./spec/av2/1.0.0/05-syntax-structures.md#s-5-19) | Tile group OBU syntax | `DECODE-LIMITS-RUNTIME-API` | ✅ | — | — | ✅ | — |
| [§ 5.19](./spec/av2/1.0.0/05-syntax-structures.md#s-5-19) | Tile group OBU syntax | `DECODE-MINIMAL-RAW-RUNTIME-OUTPUT` | ✅ | 🟡 | ✅ | ✅ | 4 |
| [§ 5.19](./spec/av2/1.0.0/05-syntax-structures.md#s-5-19) | Tile group OBU syntax | `DECODE-MINIMAL-TIER-RUNTIME-SUCCESS` | ✅ | 🟡 | ✅ | ✅ | 3 |
| [§ 5.19](./spec/av2/1.0.0/05-syntax-structures.md#s-5-19) | Tile group OBU syntax | `DECODE-TILE-PAYLOAD-INPUT-DERIVATION` | ✅ | 🟡 | — | ✅ | 2 |
| [§ 5.19](./spec/av2/1.0.0/05-syntax-structures.md#s-5-19) | Tile group OBU syntax | `DECODE-Y4M-RUNTIME-OUTPUT` | ✅ | 🟡 | ✅ | ✅ | 4 |
| [§ 5.19](./spec/av2/1.0.0/05-syntax-structures.md#s-5-19) | Tile group OBU syntax | `ENC-MINIMAL-HEADER-PLAN` | ✅ | — | — | ✅ | — |
| [§ 5.20](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20) | Tile group payload syntax | `AV2-5.19-TILE-GROUP` | ✅ | 🟡 | 🟡 | ✅ | 7 |
| [§ 5.20](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20) | Tile group payload syntax | `AV2-5.20-TILE-GROUP-PAYLOAD` | ✅ | 🟡 | 🟡 | ✅ | 3 |
| [§ 5.20.1](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-1) | General tile group payload syntax | `CONF-DECODE-RUNTIME-HASH-FUZZ` | ✅ | — | — | ✅ | 3 |
| [§ 5.20.1](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-1) | General tile group payload syntax | `CONF-DECODE-RUNTIME-RAW-FUZZ` | ✅ | — | — | ✅ | 4 |
| [§ 5.20.1](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-1) | General tile group payload syntax | `CONF-DECODE-RUNTIME-Y4M-FUZZ` | ✅ | — | — | ✅ | 4 |
| [§ 5.20.1](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-1) | General tile group payload syntax | `CONF-TILE-PAYLOAD-DECODE-FUZZ` | ✅ | — | — | ✅ | 2 |
| [§ 5.20.1](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-1) | General tile group payload syntax | `DECODE-CONTEXT-TILE-PAYLOAD-HANDOFF` | ✅ | — | — | ✅ | 2 |
| [§ 5.20.1](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-1) | General tile group payload syntax | `DECODE-LIMITS-RUNTIME-API` | ✅ | — | — | ✅ | — |
| [§ 5.20.1](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-1) | General tile group payload syntax | `DECODE-MINIMAL-RAW-RUNTIME-OUTPUT` | ✅ | 🟡 | ✅ | ✅ | 4 |
| [§ 5.20.1](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-1) | General tile group payload syntax | `DECODE-MINIMAL-TIER-RUNTIME-SUCCESS` | ✅ | 🟡 | ✅ | ✅ | 3 |
| [§ 5.20.1](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-1) | General tile group payload syntax | `DECODE-TILE-CDF-SAVE-LIFECYCLE-BOUNDARY` | ✅ | ✅ | — | ✅ | 2 |
| [§ 5.20.1](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-1) | General tile group payload syntax | `DECODE-TILE-CDF-SELECTION-BOUNDARY` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 5.20.1](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-1) | General tile group payload syntax | `DECODE-TILE-PAYLOAD-BOUNDARY` | ✅ | 🟡 | — | ✅ | 2 |
| [§ 5.20.1](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-1) | General tile group payload syntax | `DECODE-TILE-PAYLOAD-INPUT-DERIVATION` | ✅ | 🟡 | — | ✅ | 2 |
| [§ 5.20.1](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-1) | General tile group payload syntax | `DECODE-Y4M-RUNTIME-OUTPUT` | ✅ | 🟡 | ✅ | ✅ | 4 |
| [§ 5.20.1](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-1) | General tile group payload syntax | `ENC-MINIMAL-HEADER-PLAN` | ✅ | — | — | ✅ | — |
| [§ 5.20.2.1](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-2-1) | Decode tile syntax | `CONF-TILE-PAYLOAD-DECODE-FUZZ` | ✅ | — | — | ✅ | 2 |
| [§ 5.20.2.1](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-2-1) | Decode tile syntax | `DECODE-CONTEXT-TILE-PAYLOAD-HANDOFF` | ✅ | — | — | ✅ | 2 |
| [§ 5.20.2.1](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-2-1) | Decode tile syntax | `DECODE-TILE-CDF-SAVE-LIFECYCLE-BOUNDARY` | ✅ | ✅ | — | ✅ | 2 |
| [§ 5.20.2.1](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-2-1) | Decode tile syntax | `DECODE-TILE-CDF-SELECTION-BOUNDARY` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 5.20.2.1](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-2-1) | Decode tile syntax | `DECODE-TILE-PARTITION-TRAVERSAL-BOUNDARY` | ✅ | ✅ | — | ✅ | — |
| [§ 5.20.2.1](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-2-1) | Decode tile syntax | `DECODE-TILE-PAYLOAD-BOUNDARY` | ✅ | 🟡 | — | ✅ | 2 |
| [§ 5.20.2.1](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-2-1) | Decode tile syntax | `DECODE-TILE-PAYLOAD-INPUT-DERIVATION` | ✅ | 🟡 | — | ✅ | 2 |
| [§ 5.20.3.1](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-3-1) | Decode partition syntax | `CONF-TILE-PAYLOAD-DECODE-FUZZ` | ✅ | — | — | ✅ | 2 |
| [§ 5.20.3.1](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-3-1) | Decode partition syntax | `DECODE-MINIMAL-RAW-RUNTIME-OUTPUT` | ✅ | 🟡 | ✅ | ✅ | 4 |
| [§ 5.20.3.1](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-3-1) | Decode partition syntax | `DECODE-MINIMAL-TIER-RUNTIME-SUCCESS` | ✅ | 🟡 | ✅ | ✅ | 3 |
| [§ 5.20.3.1](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-3-1) | Decode partition syntax | `DECODE-TILE-PARTITION-SIZE-TABLE-BOUNDARY` | ✅ | — | — | ✅ | — |
| [§ 5.20.3.1](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-3-1) | Decode partition syntax | `DECODE-TILE-PARTITION-TRAVERSAL-BOUNDARY` | ✅ | ✅ | — | ✅ | — |
| [§ 5.20.3.1](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-3-1) | Decode partition syntax | `DECODE-Y4M-RUNTIME-OUTPUT` | ✅ | 🟡 | ✅ | ✅ | 4 |
| [§ 5.20.3.2](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-3-2) | Read partition syntax | `CONF-TILE-PAYLOAD-DECODE-FUZZ` | ✅ | — | — | ✅ | 2 |
| [§ 5.20.3.2](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-3-2) | Read partition syntax | `DECODE-MINIMAL-RAW-RUNTIME-OUTPUT` | ✅ | 🟡 | ✅ | ✅ | 4 |
| [§ 5.20.3.2](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-3-2) | Read partition syntax | `DECODE-MINIMAL-TIER-RUNTIME-SUCCESS` | ✅ | 🟡 | ✅ | ✅ | 3 |
| [§ 5.20.3.2](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-3-2) | Read partition syntax | `DECODE-TILE-CDF-SELECTION-BOUNDARY` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 5.20.3.2](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-3-2) | Read partition syntax | `DECODE-TILE-PARTITION-ALLOWED-BOUNDARY` | ✅ | — | — | ✅ | — |
| [§ 5.20.3.2](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-3-2) | Read partition syntax | `DECODE-TILE-PARTITION-DECISION-BOUNDARY` | ✅ | ✅ | — | ✅ | — |
| [§ 5.20.3.2](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-3-2) | Read partition syntax | `DECODE-TILE-PARTITION-SIZE-TABLE-BOUNDARY` | ✅ | — | — | ✅ | — |
| [§ 5.20.3.2](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-3-2) | Read partition syntax | `DECODE-TILE-PARTITION-SYMBOL-READ-BOUNDARY` | ✅ | ✅ | — | ✅ | — |
| [§ 5.20.3.2](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-3-2) | Read partition syntax | `DECODE-TILE-PARTITION-TRAVERSAL-BOUNDARY` | ✅ | ✅ | — | ✅ | — |
| [§ 5.20.3.2](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-3-2) | Read partition syntax | `DECODE-Y4M-RUNTIME-OUTPUT` | ✅ | 🟡 | ✅ | ✅ | 4 |
| [§ 5.20.4.1](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-4-1) | Decode block syntax | `DECODE-MINIMAL-BLOCK-SYNTAX-FRONTIER` | ✅ | 🟡 | — | ✅ | 4 |
| [§ 5.20.4.1](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-4-1) | Decode block syntax | `DECODE-TILE-COEFF-STATE-BUFFERS` | ✅ | — | — | ✅ | — |
| [§ 5.20.4.1](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-4-1) | Decode block syntax | `DECODE-TILE-MI-SIZE-STATE-BOUNDARY` | ✅ | — | — | ✅ | — |
| [§ 5.20.5.1](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-5-1) | Mode info syntax | `DECODE-MINIMAL-BLOCK-SYNTAX-FRONTIER` | ✅ | 🟡 | — | ✅ | 4 |
| [§ 5.20.5.3](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-5-3) | Intra frame mode info syntax | `DECODE-MINIMAL-BLOCK-SYNTAX-FRONTIER` | ✅ | 🟡 | — | ✅ | 4 |
| [§ 5.20.5.3](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-5-3) | Intra frame mode info syntax | `DECODE-MINIMAL-INTRA-RECONSTRUCTION-FRONTIER` | ✅ | 🟡 | ✅ | ✅ | 4 |
| [§ 5.20.5.3](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-5-3) | Intra frame mode info syntax | `ENC-INTRA-BLOCK-MODE-TRACE` | ✅ | — | — | ✅ | — |
| [§ 5.20.5.3](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-5-3) | Intra frame mode info syntax | `ENC-INTRA-BLOCK-TRACE-CHROMA-SKIP` | ✅ | — | — | ✅ | — |
| [§ 5.20.5.3](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-5-3) | Intra frame mode info syntax | `ENC-INTRA-BLOCK-TRACE-CODED-CHROMA-DC` | ✅ | — | — | ✅ | — |
| [§ 5.20.5.3](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-5-3) | Intra frame mode info syntax | `ENC-INTRA-BLOCK-TRACE-CODED-DC` | ✅ | — | — | ✅ | — |
| [§ 5.20.5.3](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-5-3) | Intra frame mode info syntax | `ENC-INTRA-BLOCK-TRACE-LUMA-SKIP` | ✅ | — | — | ✅ | — |
| [§ 5.20.5.5](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-5-5) | Read intra Y mode syntax | `DECODE-MINIMAL-BLOCK-SYNTAX-FRONTIER` | ✅ | 🟡 | — | ✅ | 4 |
| [§ 5.20.5.5](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-5-5) | Read intra Y mode syntax | `DECODE-MINIMAL-INTRA-RECONSTRUCTION-FRONTIER` | ✅ | 🟡 | ✅ | ✅ | 4 |
| [§ 5.20.5.5](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-5-5) | Read intra Y mode syntax | `ENC-INTRA-MODE-SYMBOL-EMISSION` | ✅ | — | — | ✅ | — |
| [§ 5.20.5.6](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-5-6) | Read intra UV mode syntax | `DECODE-MINIMAL-BLOCK-SYNTAX-FRONTIER` | ✅ | 🟡 | — | ✅ | 4 |
| [§ 5.20.5.6](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-5-6) | Read intra UV mode syntax | `DECODE-MINIMAL-INTRA-RECONSTRUCTION-FRONTIER` | ✅ | 🟡 | ✅ | ✅ | 4 |
| [§ 5.20.5.6](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-5-6) | Read intra UV mode syntax | `ENC-UV-MODE-SYMBOL-EMISSION` | ✅ | — | — | ✅ | — |
| [§ 5.20.6.1](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-6-1) | TX size syntax | `DECODE-MINIMAL-BLOCK-SYNTAX-FRONTIER` | ✅ | 🟡 | — | ✅ | 4 |
| [§ 5.20.6.2](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-6-2) | Block TX size syntax | `DECODE-MINIMAL-BLOCK-SYNTAX-FRONTIER` | ✅ | 🟡 | — | ✅ | 4 |
| [§ 5.20.7.23](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-23) | Residual syntax | `DECODE-MINIMAL-BLOCK-SYNTAX-FRONTIER` | ✅ | 🟡 | — | ✅ | 4 |
| [§ 5.20.7.24](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-24) | Transform block syntax | `DECODE-MINIMAL-BLOCK-SYNTAX-FRONTIER` | ✅ | 🟡 | — | ✅ | 4 |
| [§ 5.20.7.24](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-24) | Transform block syntax | `DECODE-MINIMAL-INTRA-RECONSTRUCTION-FRONTIER` | ✅ | 🟡 | ✅ | ✅ | 4 |
| [§ 5.20.7.26](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-26) | Get plane residual size function | `DECODE-TILE-PARTITION-ALLOWED-BOUNDARY` | ✅ | — | — | ✅ | — |
| [§ 5.20.7.27](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-27) | Coefficients syntax | `DECODE-COEFF-ALL-ZERO-BLOCK-STATE` | ✅ | — | — | ✅ | 1 |
| [§ 5.20.7.27](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-27) | Coefficients syntax | `DECODE-COEFF-ALL-ZERO-CONTEXT-STATE` | ✅ | — | — | ✅ | 1 |
| [§ 5.20.7.27](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-27) | Coefficients syntax | `DECODE-COEFF-BASE-DERIVED-LEVEL-PASS` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 5.20.7.27](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-27) | Coefficients syntax | `DECODE-COEFF-BASE-PH-CDF-ROW` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 5.20.7.27](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-27) | Coefficients syntax | `DECODE-COEFF-BASE-SYMBOL-READ` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 5.20.7.27](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-27) | Coefficients syntax | `DECODE-COEFF-CDF-Q-CONTEXT-HANDOFF` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 5.20.7.27](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-27) | Coefficients syntax | `DECODE-COEFF-EOB-BRANCH-HANDOFF` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 5.20.7.27](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-27) | Coefficients syntax | `DECODE-COEFF-EOB-DERIVED-SYMBOL-READ` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 5.20.7.27](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-27) | Coefficients syntax | `DECODE-COEFF-EOB-SIZE-CONTEXT` | ✅ | — | — | ✅ | 1 |
| [§ 5.20.7.27](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-27) | Coefficients syntax | `DECODE-COEFF-EOB-SYMBOL-READ` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 5.20.7.27](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-27) | Coefficients syntax | `DECODE-COEFF-EOB-VALUE-STATE` | ✅ | — | — | ✅ | 1 |
| [§ 5.20.7.27](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-27) | Coefficients syntax | `DECODE-COEFF-FRAME-FACTS-HANDOFF` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 5.20.7.27](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-27) | Coefficients syntax | `DECODE-COEFF-FSC-BRANCH-HANDOFF` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 5.20.7.27](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-27) | Coefficients syntax | `DECODE-COEFF-FSC-BRANCH-SCAN-ORDER` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 5.20.7.27](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-27) | Coefficients syntax | `DECODE-COEFF-FSC-BRANCH-SEG-EOB-HANDOFF` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 5.20.7.27](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-27) | Coefficients syntax | `DECODE-COEFF-FSC-BRANCH-TX-SIZE-HANDOFF` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 5.20.7.27](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-27) | Coefficients syntax | `DECODE-COEFF-FSC-CONTEXT-COMMIT` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 5.20.7.27](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-27) | Coefficients syntax | `DECODE-COEFF-FSC-LEVEL-PASS` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 5.20.7.27](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-27) | Coefficients syntax | `DECODE-COEFF-FSC-QUANT-PASS` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 5.20.7.27](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-27) | Coefficients syntax | `DECODE-COEFF-FSC-SCAN-WALK` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 5.20.7.27](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-27) | Coefficients syntax | `DECODE-COEFF-FSC-SIGN-PASS` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 5.20.7.27](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-27) | Coefficients syntax | `DECODE-COEFF-IDTX-CDF-ROWS` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 5.20.7.27](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-27) | Coefficients syntax | `DECODE-COEFF-LEVEL-STATE-WRITE` | ✅ | — | — | ✅ | 1 |
| [§ 5.20.7.27](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-27) | Coefficients syntax | `DECODE-COEFF-MAX-LEVEL-DERIVE` | ✅ | — | — | ✅ | 1 |
| [§ 5.20.7.27](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-27) | Coefficients syntax | `DECODE-COEFF-NONZERO-BLOCK-STATE` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 5.20.7.27](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-27) | Coefficients syntax | `DECODE-COEFF-NONZERO-CONTEXT-COMMIT` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 5.20.7.27](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-27) | Coefficients syntax | `DECODE-COEFF-ORDINARY-BRANCH-ADJUSTED-TX-SIZE` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 5.20.7.27](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-27) | Coefficients syntax | `DECODE-COEFF-ORDINARY-BRANCH-CHROMA-INTER-TXTYPES-HANDOFF` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 5.20.7.27](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-27) | Coefficients syntax | `DECODE-COEFF-ORDINARY-BRANCH-COEFFS-GEOMETRY-HANDOFF` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 5.20.7.27](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-27) | Coefficients syntax | `DECODE-COEFF-ORDINARY-BRANCH-DIRECTIONAL-UV-HANDOFF` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 5.20.7.27](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-27) | Coefficients syntax | `DECODE-COEFF-ORDINARY-BRANCH-GEOMETRY-HANDOFF` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 5.20.7.27](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-27) | Coefficients syntax | `DECODE-COEFF-ORDINARY-BRANCH-HANDOFF` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 5.20.7.27](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-27) | Coefficients syntax | `DECODE-COEFF-ORDINARY-BRANCH-LOSSLESS-HANDOFF` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 5.20.7.27](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-27) | Coefficients syntax | `DECODE-COEFF-ORDINARY-BRANCH-LUMA-TXTYPES-HANDOFF` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 5.20.7.27](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-27) | Coefficients syntax | `DECODE-COEFF-ORDINARY-BRANCH-MODE-TO-TXFM-HANDOFF` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 5.20.7.27](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-27) | Coefficients syntax | `DECODE-COEFF-ORDINARY-BRANCH-PLANE-TYPE-HANDOFF` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 5.20.7.27](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-27) | Coefficients syntax | `DECODE-COEFF-ORDINARY-BRANCH-SCAN-ORDER` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 5.20.7.27](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-27) | Coefficients syntax | `DECODE-COEFF-ORDINARY-BRANCH-TX-CLASS-HANDOFF` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 5.20.7.27](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-27) | Coefficients syntax | `DECODE-COEFF-ORDINARY-BRANCH-TX-SET-HANDOFF` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 5.20.7.27](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-27) | Coefficients syntax | `DECODE-COEFF-ORDINARY-BRANCH-TX-SIZE-CONTEXT` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 5.20.7.27](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-27) | Coefficients syntax | `DECODE-COEFF-ORDINARY-BRANCH-TX-SIZE-DIMENSIONS` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 5.20.7.27](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-27) | Coefficients syntax | `DECODE-COEFF-ORDINARY-DERIVED-BASE-PASS` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 5.20.7.27](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-27) | Coefficients syntax | `DECODE-COEFF-ORDINARY-DERIVED-SIGN-PASS` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 5.20.7.27](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-27) | Coefficients syntax | `DECODE-COEFF-ORDINARY-PASS-COMPOSE` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 5.20.7.27](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-27) | Coefficients syntax | `DECODE-COEFF-PARITY-TCQ-HANDOFF` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 5.20.7.27](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-27) | Coefficients syntax | `DECODE-COEFF-QUANT-PASS-COMPOSE` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 5.20.7.27](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-27) | Coefficients syntax | `DECODE-COEFF-QUANT-PASS-MAXLEVEL-HANDOFF` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 5.20.7.27](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-27) | Coefficients syntax | `DECODE-COEFF-QUANT-STATE-WRITE` | ✅ | — | — | ✅ | 1 |
| [§ 5.20.7.27](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-27) | Coefficients syntax | `DECODE-COEFF-RUNTIME-FRAME-ENTRY-HANDOFF` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 5.20.7.27](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-27) | Coefficients syntax | `DECODE-COEFF-RUNTIME-TX-SIZE-GEOMETRY-HANDOFF` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 5.20.7.27](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-27) | Coefficients syntax | `DECODE-COEFF-SCAN-WALK` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 5.20.7.27](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-27) | Coefficients syntax | `DECODE-COEFF-SIGN-SOURCE-DERIVE` | ✅ | — | — | ✅ | 1 |
| [§ 5.20.7.27](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-27) | Coefficients syntax | `DECODE-COEFF-SIGN-SYMBOL-READ` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 5.20.7.27](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-27) | Coefficients syntax | `DECODE-COEFF-STATE-CONTEXT-HANDOFF` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 5.20.7.27](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-27) | Coefficients syntax | `DECODE-COEFF-TX-CLASS-DERIVE` | ✅ | — | — | ✅ | 1 |
| [§ 5.20.7.27](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-27) | Coefficients syntax | `DECODE-COEFF-USE-FSC-BRANCH-HANDOFF` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 5.20.7.27](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-27) | Coefficients syntax | `DECODE-COEFF-USE-FSC-CONDITION-HANDOFF` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 5.20.7.27](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-27) | Coefficients syntax | `DECODE-COEFF-USE-FSC-SHARED-FACTS-HANDOFF` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 5.20.7.27](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-27) | Coefficients syntax | `DECODE-MINIMAL-BLOCK-SYNTAX-FRONTIER` | ✅ | 🟡 | — | ✅ | 4 |
| [§ 5.20.7.27](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-27) | Coefficients syntax | `DECODE-MINIMAL-INTRA-RECONSTRUCTION-FRONTIER` | ✅ | 🟡 | ✅ | ✅ | 4 |
| [§ 5.20.7.27](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-27) | Coefficients syntax | `DECODE-TILE-COEFF-STATE-BUFFERS` | ✅ | — | — | ✅ | — |
| [§ 5.20.7.27](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-27) | Coefficients syntax | `ENC-COEFF-BASE-LF-CONTEXT` | ✅ | — | — | ✅ | — |
| [§ 5.20.7.27](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-27) | Coefficients syntax | `ENC-COEFF-BASE-LF-TOKEN` | ✅ | — | — | ✅ | — |
| [§ 5.20.7.27](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-27) | Coefficients syntax | `ENC-COEFF-MULTI-TOKENS` | ✅ | — | — | ✅ | — |
| [§ 5.20.7.27](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-27) | Coefficients syntax | `ENC-COEFFICIENT-TOKENIZATION-MINIMAL` | ✅ | — | — | ✅ | — |
| [§ 5.20.7.27](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-27) | Coefficients syntax | `ENC-INTRA-BLOCK-TRACE-BYPASS-LITERAL` | ✅ | — | — | ✅ | — |
| [§ 5.20.7.27](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-27) | Coefficients syntax | `ENC-INTRA-BLOCK-TRACE-CHROMA-SKIP` | ✅ | — | — | ✅ | — |
| [§ 5.20.7.27](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-27) | Coefficients syntax | `ENC-INTRA-BLOCK-TRACE-CODED-BR` | ✅ | — | — | ✅ | — |
| [§ 5.20.7.27](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-27) | Coefficients syntax | `ENC-INTRA-BLOCK-TRACE-CODED-CHROMA-DC` | ✅ | — | — | ✅ | — |
| [§ 5.20.7.27](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-27) | Coefficients syntax | `ENC-INTRA-BLOCK-TRACE-CODED-DC` | ✅ | — | — | ✅ | — |
| [§ 5.20.7.27](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-27) | Coefficients syntax | `ENC-INTRA-BLOCK-TRACE-GOLOMB-FINITE` | ✅ | — | — | ✅ | — |
| [§ 5.20.7.27](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-27) | Coefficients syntax | `ENC-INTRA-BLOCK-TRACE-GOLOMB-PREFIX` | ✅ | — | — | ✅ | — |
| [§ 5.20.7.27](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-27) | Coefficients syntax | `ENC-INTRA-BLOCK-TRACE-LUMA-SKIP` | ✅ | — | — | ✅ | — |
| [§ 5.20.7.27](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-27) | Coefficients syntax | `ENC-INTRA-BLOCK-TRACE-TWO-COEFF` | ✅ | — | — | ✅ | — |
| [§ 5.20.7.27](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-27) | Coefficients syntax | `ENC-INTRA-BLOCK-TRACE-TWO-COEFF-TX-TYPE` | ✅ | — | — | ✅ | — |
| [§ 5.20.7.27](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-27) | Coefficients syntax | `ENC-INTRA-TX-TYPE-TOKEN` | ✅ | — | — | ✅ | — |
| [§ 5.20.7.28](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-28) | Read quantized coefficient syntax | `DECODE-COEFF-FSC-BRANCH-HANDOFF` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 5.20.7.28](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-28) | Read quantized coefficient syntax | `DECODE-COEFF-FSC-BRANCH-SEG-EOB-HANDOFF` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 5.20.7.28](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-28) | Read quantized coefficient syntax | `DECODE-COEFF-FSC-CONTEXT-COMMIT` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 5.20.7.28](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-28) | Read quantized coefficient syntax | `DECODE-COEFF-FSC-QUANT-PASS` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 5.20.7.28](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-28) | Read quantized coefficient syntax | `DECODE-COEFF-NONZERO-CONTEXT-COMMIT` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 5.20.7.28](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-28) | Read quantized coefficient syntax | `DECODE-COEFF-ORDINARY-BRANCH-HANDOFF` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 5.20.7.28](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-28) | Read quantized coefficient syntax | `DECODE-COEFF-ORDINARY-DERIVED-BASE-PASS` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 5.20.7.28](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-28) | Read quantized coefficient syntax | `DECODE-COEFF-ORDINARY-DERIVED-SIGN-PASS` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 5.20.7.28](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-28) | Read quantized coefficient syntax | `DECODE-COEFF-ORDINARY-PASS-COMPOSE` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 5.20.7.28](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-28) | Read quantized coefficient syntax | `DECODE-COEFF-QUANT-PASS-COMPOSE` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 5.20.7.28](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-28) | Read quantized coefficient syntax | `DECODE-COEFF-QUANT-PASS-MAXLEVEL-HANDOFF` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 5.20.7.28](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-28) | Read quantized coefficient syntax | `DECODE-COEFF-QUANT-STATE-WRITE` | ✅ | — | — | ✅ | 1 |
| [§ 5.20.7.28](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-28) | Read quantized coefficient syntax | `DECODE-COEFF-READ-QUANT-SYNTAX` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 5.20.7.28](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-28) | Read quantized coefficient syntax | `DECODE-COEFF-STATE-CONTEXT-HANDOFF` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 5.20.7.28](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-28) | Read quantized coefficient syntax | `ENC-COEFFICIENT-TOKENIZATION-MINIMAL` | ✅ | — | — | ✅ | — |
| [§ 5.20.7.28](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-28) | Read quantized coefficient syntax | `ENC-INTRA-BLOCK-TRACE-BYPASS-LITERAL` | ✅ | — | — | ✅ | — |
| [§ 5.20.7.28](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-28) | Read quantized coefficient syntax | `ENC-INTRA-BLOCK-TRACE-GOLOMB-FINITE` | ✅ | — | — | ✅ | — |
| [§ 5.20.7.28](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-28) | Read quantized coefficient syntax | `ENC-INTRA-BLOCK-TRACE-GOLOMB-PREFIX` | ✅ | — | — | ✅ | — |
| [§ 5.20.7.29](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-29) | Compute transform type function | `DECODE-COEFF-ORDINARY-BRANCH-CHROMA-INTER-TXTYPES-HANDOFF` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 5.20.7.29](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-29) | Compute transform type function | `DECODE-COEFF-ORDINARY-BRANCH-DIRECTIONAL-UV-HANDOFF` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 5.20.7.29](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-29) | Compute transform type function | `DECODE-COEFF-ORDINARY-BRANCH-LOSSLESS-HANDOFF` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 5.20.7.29](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-29) | Compute transform type function | `DECODE-COEFF-ORDINARY-BRANCH-LUMA-TXTYPES-HANDOFF` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 5.20.7.29](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-29) | Compute transform type function | `DECODE-COEFF-ORDINARY-BRANCH-MODE-TO-TXFM-HANDOFF` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 5.20.7.29](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-29) | Compute transform type function | `DECODE-COEFF-ORDINARY-BRANCH-TX-SET-HANDOFF` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 5.20.7.29](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-29) | Compute transform type function | `ENC-INTRA-BLOCK-TRACE-TWO-COEFF` | ✅ | — | — | ✅ | — |
| [§ 5.20.7.30](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-30) | Get scan function | `DECODE-COEFF-FSC-BRANCH-SCAN-ORDER` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 5.20.7.30](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-30) | Get scan function | `DECODE-COEFF-FSC-BRANCH-SEG-EOB-HANDOFF` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 5.20.7.30](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-30) | Get scan function | `DECODE-COEFF-FSC-BRANCH-TX-SIZE-HANDOFF` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 5.20.7.30](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-30) | Get scan function | `DECODE-COEFF-ORDINARY-BRANCH-SCAN-ORDER` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 5.20.7.30](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-30) | Get scan function | `ENC-INTRA-BLOCK-TRACE-TWO-COEFF` | ✅ | — | — | ✅ | — |
| [§ 5.20.7.30](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-7-30) | Get scan function | `RECON-COEFFICIENT-SCAN-ORDER` | ✅ | — | ✅ | ✅ | — |
| [§ 5.20.8.2](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-8-2) | Transform type syntax | `ENC-INTRA-BLOCK-TRACE-TWO-COEFF-TX-TYPE` | ✅ | — | — | ✅ | — |
| [§ 5.20.8.2](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-8-2) | Transform type syntax | `ENC-INTRA-TX-TYPE-TOKEN` | ✅ | — | — | ✅ | — |
| [§ 5.20.8.2](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-8-2) | Transform type syntax | `ENC-SEC-TX-TYPE-TOKEN` | ✅ | — | — | ✅ | — |
| [§ 5.20.8.3](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-8-3) | Get transform set function | `DECODE-COEFF-ORDINARY-BRANCH-LOSSLESS-HANDOFF` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 5.20.8.3](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-8-3) | Get transform set function | `DECODE-COEFF-ORDINARY-BRANCH-TX-SET-HANDOFF` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 5.20.9.1](./spec/av2/1.0.0/05-syntax-structures.md#s-5-20-9-1) | Is inside function | `DECODE-TILE-PARTITION-TRAVERSAL-BOUNDARY` | ✅ | ✅ | — | ✅ | — |

## Chapter 6 — Syntax structures semantics

| Section | Spec item | Feature | Mapped | Parse | Validate | Tests | Diagnostics |
|---|---|---|:-:|:-:|:-:|:-:|:-:|
| [§ 6.2](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-2) | OBU semantics | `DECODE-MINIMAL-RAW-RUNTIME-OUTPUT` | ✅ | 🟡 | ✅ | ✅ | 4 |
| [§ 6.2](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-2) | OBU semantics | `DECODE-MINIMAL-TIER-RUNTIME-SUCCESS` | ✅ | 🟡 | ✅ | ✅ | 3 |
| [§ 6.2](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-2) | OBU semantics | `DECODE-Y4M-RUNTIME-OUTPUT` | ✅ | 🟡 | ✅ | ✅ | 4 |
| [§ 6.2.1](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-2-1) | General OBU semantics | `DECODE-BYTE-STREAM-PLANNER` | ✅ | ✅ | — | ✅ | 3 |
| [§ 6.2.2](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-2-2) | OBU header semantics | `AV2-5.2.1-OBU-TYPE` | ✅ | ✅ | ✅ | ✅ | 5 |
| [§ 6.2.2](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-2-2) | OBU header semantics | `AV2-5.2.2-OBU-HEADER` | ✅ | ✅ | ✅ | ✅ | 6 |
| [§ 6.2.2](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-2-2) | OBU header semantics | `AV2-6.2.2-OBU-HEADER-ACTIVATED-SEQUENCE-LIMITS` | ✅ | ✅ | ✅ | ✅ | 3 |
| [§ 6.2.2](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-2-2) | OBU header semantics | `DECODE-BYTE-STREAM-PLANNER` | ✅ | ✅ | — | ✅ | 3 |
| [§ 6.2.2](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-2-2) | OBU header semantics | `DECODE-STREAM-STATE-PLANNER` | ✅ | — | — | ✅ | 1 |
| [§ 6.2.3](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-2-3) | Trailing bits semantics | `AV2-5.2.3-TRAILING-BITS` | ✅ | ✅ | 🟡 | ✅ | 2 |
| [§ 6.2.3](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-2-3) | Trailing bits semantics | `AV2-5.3-RESERVED-OBU` | ✅ | — | ✅ | ✅ | 2 |
| [§ 6.2.4](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-2-4) | Byte alignment semantics | `AV2-5.2.4-BYTE-ALIGNMENT` | ✅ | ✅ | 🟡 | ✅ | 1 |
| [§ 6.4](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-4) | Sequence header OBU semantics | `AV2-5.4-SEQUENCE-HEADER` | ✅ | 🟡 | 🟡 | 🟡 | 1 |
| [§ 6.4](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-4) | Sequence header OBU semantics | `AV2-6.2.2-OBU-HEADER-ACTIVATED-SEQUENCE-LIMITS` | ✅ | ✅ | ✅ | ✅ | 3 |
| [§ 6.4](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-4) | Sequence header OBU semantics | `AV2-6.4-SEQUENCE-HEADER-SEMANTICS` | ✅ | ✅ | 🟡 | ✅ | 10 |
| [§ 6.4.1](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-4-1) | General sequence header OBU semantics | `AV2-5.4.1-SEQUENCE-HEADER-GENERAL` | ✅ | ✅ | ✅ | ✅ | 6 |
| [§ 6.4.1](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-4-1) | General sequence header OBU semantics | `CONF-DECODE-RUNTIME-RAW-FUZZ` | ✅ | — | — | ✅ | 4 |
| [§ 6.4.1](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-4-1) | General sequence header OBU semantics | `CONF-DECODE-RUNTIME-Y4M-FUZZ` | ✅ | — | — | ✅ | 4 |
| [§ 6.4.1](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-4-1) | General sequence header OBU semantics | `CONF-RECON-FRAME-HASH-FUZZ` | ✅ | — | — | ✅ | — |
| [§ 6.4.1](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-4-1) | General sequence header OBU semantics | `CONF-RECON-FRAME-PLANE-TYPES-FUZZ` | ✅ | — | — | ✅ | — |
| [§ 6.4.1](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-4-1) | General sequence header OBU semantics | `CONF-RECON-INTRA-PREDICTION-FUZZ` | ✅ | — | — | ✅ | — |
| [§ 6.4.1](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-4-1) | General sequence header OBU semantics | `CONF-RECON-Y4M-OUTPUT-FUZZ` | ✅ | — | — | ✅ | — |
| [§ 6.4.1](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-4-1) | General sequence header OBU semantics | `DECODE-LIMITS-RUNTIME-API` | ✅ | — | — | ✅ | — |
| [§ 6.4.1](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-4-1) | General sequence header OBU semantics | `DECODE-MINIMAL-RAW-RUNTIME-OUTPUT` | ✅ | 🟡 | ✅ | ✅ | 4 |
| [§ 6.4.1](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-4-1) | General sequence header OBU semantics | `DECODE-MINIMAL-TIER-RUNTIME-SUCCESS` | ✅ | 🟡 | ✅ | ✅ | 3 |
| [§ 6.4.1](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-4-1) | General sequence header OBU semantics | `DECODE-Y4M-RUNTIME-OUTPUT` | ✅ | 🟡 | ✅ | ✅ | 4 |
| [§ 6.4.1](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-4-1) | General sequence header OBU semantics | `DOC-DECODER-OUTPUT-EQUIVALENCE-CONTRACT` | ✅ | — | — | ✅ | — |
| [§ 6.4.1](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-4-1) | General sequence header OBU semantics | `INFRA-RECON-FRAME-PLANE-TYPES` | ✅ | — | — | ✅ | — |
| [§ 6.4.1](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-4-1) | General sequence header OBU semantics | `RECON-CURRENT-FRAME-WORKSPACE` | ✅ | — | ✅ | ✅ | — |
| [§ 6.4.1](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-4-1) | General sequence header OBU semantics | `RECON-DEQUANT-QUANTIZER-INDEX-RESOLUTION` | ✅ | — | — | ✅ | — |
| [§ 6.4.1](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-4-1) | General sequence header OBU semantics | `RECON-DEQUANT-QUANTIZER-LOOKUP` | ✅ | — | — | ✅ | — |
| [§ 6.4.1](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-4-1) | General sequence header OBU semantics | `RECON-Y4M-OUTPUT-WRITER` | ✅ | — | ✅ | ✅ | — |
| [§ 6.4.2](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-4-2) | Sequence tile config semantics | `AV2-5.4.2-SEQUENCE-TILE-CONFIG` | ✅ | ✅ | ✅ | ✅ | 4 |
| [§ 6.4.3](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-4-3) | Sequence partition config semantics | `AV2-5.4.3-SEQUENCE-PARTITION-CONFIG` | ✅ | ✅ | 🟡 | ✅ | — |
| [§ 6.4.4](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-4-4) | Sequence segment config semantics | `AV2-5.4.4-SEQUENCE-SEGMENT-CONFIG` | ✅ | ✅ | 🟡 | ✅ | — |
| [§ 6.4.5](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-4-5) | Sequence intra config semantics | `AV2-5.4.5-SEQUENCE-INTRA-CONFIG` | ✅ | ✅ | 🟡 | ✅ | — |
| [§ 6.4.6](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-4-6) | Sequence inter config semantics | `AV2-5.4.6-SEQUENCE-INTER-CONFIG` | ✅ | ✅ | ✅ | ✅ | 1 |
| [§ 6.4.6](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-4-6) | Sequence inter config semantics | `AV2-6.17.2-FRAME-HEADER-INFO-SEMANTICS` | ✅ | — | 🟡 | ✅ | 18 |
| [§ 6.4.6](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-4-6) | Sequence inter config semantics | `DECODE-LIMITS-RUNTIME-API` | ✅ | — | — | ✅ | — |
| [§ 6.4.6](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-4-6) | Sequence inter config semantics | `RECON-REFERENCE-FRAME-STORE` | ✅ | — | — | ✅ | — |
| [§ 6.4.7](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-4-7) | Sequence screen content config semantics | `AV2-5.4.7-SEQUENCE-SCC-CONFIG` | ✅ | ✅ | 🟡 | ✅ | — |
| [§ 6.4.8](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-4-8) | Sequence transform quant entropy config semantics | `AV2-5.4.8-SEQUENCE-TQ-ENTROPY-CONFIG` | ✅ | ✅ | 🟡 | ✅ | — |
| [§ 6.4.8](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-4-8) | Sequence transform quant entropy config semantics | `DECODE-COEFF-FRAME-FACTS-HANDOFF` | ✅ | 🟡 | — | ✅ | 1 |
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
| [§ 6.8.4](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-8-4) | LCR aggregate info semantics | `AV2-5.8.3-LCR-AGGREGATE-INFO` | ✅ | ✅ | ✅ | ✅ | — |
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
| [§ 6.16.13](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-16-13) | Metadata decoded frame hash semantics | `CONF-DECODE-RUNTIME-RAW-FUZZ` | ✅ | — | — | ✅ | 4 |
| [§ 6.16.13](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-16-13) | Metadata decoded frame hash semantics | `CONF-DECODE-RUNTIME-Y4M-FUZZ` | ✅ | — | — | ✅ | 4 |
| [§ 6.16.13](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-16-13) | Metadata decoded frame hash semantics | `CONF-RECON-FRAME-HASH-FUZZ` | ✅ | — | — | ✅ | — |
| [§ 6.16.13](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-16-13) | Metadata decoded frame hash semantics | `CONF-RECON-Y4M-OUTPUT-FUZZ` | ✅ | — | — | ✅ | — |
| [§ 6.16.13](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-16-13) | Metadata decoded frame hash semantics | `DECODE-MINIMAL-RAW-RUNTIME-OUTPUT` | ✅ | 🟡 | ✅ | ✅ | 4 |
| [§ 6.16.13](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-16-13) | Metadata decoded frame hash semantics | `DECODE-MINIMAL-TIER-RUNTIME-SUCCESS` | ✅ | 🟡 | ✅ | ✅ | 3 |
| [§ 6.16.13](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-16-13) | Metadata decoded frame hash semantics | `DECODE-Y4M-RUNTIME-OUTPUT` | ✅ | 🟡 | ✅ | ✅ | 4 |
| [§ 6.16.13](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-16-13) | Metadata decoded frame hash semantics | `DOC-DECODER-OUTPUT-EQUIVALENCE-CONTRACT` | ✅ | — | — | ✅ | — |
| [§ 6.16.13](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-16-13) | Metadata decoded frame hash semantics | `RECON-FRAME-HASH-DIGEST` | ✅ | — | — | ✅ | — |
| [§ 6.16.13](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-16-13) | Metadata decoded frame hash semantics | `RECON-HASH-INPUT-SERIALIZATION` | ✅ | — | — | ✅ | — |
| [§ 6.16.13](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-16-13) | Metadata decoded frame hash semantics | `RECON-Y4M-OUTPUT-WRITER` | ✅ | — | ✅ | ✅ | — |
| [§ 6.17](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-17) | Frame header OBU semantics | `AV2-5.18-FRAME-HEADER` | ✅ | 🟡 | 🟡 | ✅ | 4 |
| [§ 6.17](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-17) | Frame header OBU semantics | `AV2-5.18.1-FRAME-HEADER-GENERAL` | ✅ | 🟡 | 🟡 | ✅ | 2 |
| [§ 6.17](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-17) | Frame header OBU semantics | `AV2-5.18.8-TRANSFORM-CODING-MODES` | ✅ | 🟡 | — | ✅ | — |
| [§ 6.17](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-17) | Frame header OBU semantics | `AV2-5.18.9-GLOBAL-MOTION` | ✅ | 🟡 | — | ✅ | — |
| [§ 6.17.1](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-17-1) | General frame header semantics | `DECODE-TILE-PAYLOAD-INPUT-DERIVATION` | ✅ | 🟡 | — | ✅ | 2 |
| [§ 6.17.2](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-17-2) | Frame header info semantics | `AV2-5.18.2-FRAME-HEADER-INFO` | ✅ | 🟡 | 🟡 | ✅ | 9 |
| [§ 6.17.2](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-17-2) | Frame header info semantics | `AV2-6.17.2-FRAME-HEADER-INFO-SEMANTICS` | ✅ | — | 🟡 | ✅ | 18 |
| [§ 6.17.2](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-17-2) | Frame header info semantics | `AV2-7.23-REFERENCE-FRAME-UPDATE` | ✅ | — | 🟡 | ✅ | 1 |
| [§ 6.17.2](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-17-2) | Frame header info semantics | `AV2-7.3.9-LONG-TERM-REFERENCE-AVAILABILITY` | ✅ | — | 🟡 | 🟡 | 1 |
| [§ 6.17.2](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-17-2) | Frame header info semantics | `DECODE-COEFF-CDF-Q-CONTEXT-HANDOFF` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 6.17.2](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-17-2) | Frame header info semantics | `DECODE-COEFF-FRAME-FACTS-HANDOFF` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 6.17.2](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-17-2) | Frame header info semantics | `DECODE-MINIMAL-RAW-RUNTIME-OUTPUT` | ✅ | 🟡 | ✅ | ✅ | 4 |
| [§ 6.17.2](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-17-2) | Frame header info semantics | `DECODE-MINIMAL-TIER-RUNTIME-SUCCESS` | ✅ | 🟡 | ✅ | ✅ | 3 |
| [§ 6.17.2](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-17-2) | Frame header info semantics | `DECODE-Y4M-RUNTIME-OUTPUT` | ✅ | 🟡 | ✅ | ✅ | 4 |
| [§ 6.17.2](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-17-2) | Frame header info semantics | `RECON-REFERENCE-FRAME-STORE` | ✅ | — | — | ✅ | — |
| [§ 6.17.4.1](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-17-4-1) | Frame size semantics | `AV2-5.18.4-FRAME-SIZE` | ✅ | 🟡 | 🟡 | ✅ | 2 |
| [§ 6.17.4.1](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-17-4-1) | Frame size semantics | `AV2-6.17.2-FRAME-HEADER-INFO-SEMANTICS` | ✅ | — | 🟡 | ✅ | 18 |
| [§ 6.17.4.1](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-17-4-1) | Frame size semantics | `CONF-RECON-FRAME-PLANE-TYPES-FUZZ` | ✅ | — | — | ✅ | — |
| [§ 6.17.4.1](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-17-4-1) | Frame size semantics | `CONF-RECON-INTRA-PREDICTION-FUZZ` | ✅ | — | — | ✅ | — |
| [§ 6.17.4.1](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-17-4-1) | Frame size semantics | `DECODE-LIMITS-RUNTIME-API` | ✅ | — | — | ✅ | — |
| [§ 6.17.4.1](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-17-4-1) | Frame size semantics | `DECODE-MINIMAL-RAW-RUNTIME-OUTPUT` | ✅ | 🟡 | ✅ | ✅ | 4 |
| [§ 6.17.4.1](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-17-4-1) | Frame size semantics | `DECODE-MINIMAL-TIER-RUNTIME-SUCCESS` | ✅ | 🟡 | ✅ | ✅ | 3 |
| [§ 6.17.4.1](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-17-4-1) | Frame size semantics | `DECODE-Y4M-RUNTIME-OUTPUT` | ✅ | 🟡 | ✅ | ✅ | 4 |
| [§ 6.17.4.1](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-17-4-1) | Frame size semantics | `DOC-DECODER-OUTPUT-EQUIVALENCE-CONTRACT` | ✅ | — | — | ✅ | — |
| [§ 6.17.4.1](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-17-4-1) | Frame size semantics | `INFRA-RECON-FRAME-PLANE-TYPES` | ✅ | — | — | ✅ | — |
| [§ 6.17.4.1](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-17-4-1) | Frame size semantics | `RECON-CURRENT-FRAME-WORKSPACE` | ✅ | — | ✅ | ✅ | — |
| [§ 6.17.4.4](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-17-4-4) | Compute image size function semantics | `CONF-DECODE-RUNTIME-RAW-FUZZ` | ✅ | — | — | ✅ | 4 |
| [§ 6.17.4.4](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-17-4-4) | Compute image size function semantics | `CONF-DECODE-RUNTIME-Y4M-FUZZ` | ✅ | — | — | ✅ | 4 |
| [§ 6.17.4.4](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-17-4-4) | Compute image size function semantics | `CONF-RECON-FRAME-PLANE-TYPES-FUZZ` | ✅ | — | — | ✅ | — |
| [§ 6.17.4.4](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-17-4-4) | Compute image size function semantics | `CONF-RECON-INTRA-PREDICTION-FUZZ` | ✅ | — | — | ✅ | — |
| [§ 6.17.4.4](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-17-4-4) | Compute image size function semantics | `CONF-RECON-Y4M-OUTPUT-FUZZ` | ✅ | — | — | ✅ | — |
| [§ 6.17.4.4](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-17-4-4) | Compute image size function semantics | `DECODE-LIMITS-RUNTIME-API` | ✅ | — | — | ✅ | — |
| [§ 6.17.4.4](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-17-4-4) | Compute image size function semantics | `DECODE-MINIMAL-RAW-RUNTIME-OUTPUT` | ✅ | 🟡 | ✅ | ✅ | 4 |
| [§ 6.17.4.4](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-17-4-4) | Compute image size function semantics | `DECODE-Y4M-RUNTIME-OUTPUT` | ✅ | 🟡 | ✅ | ✅ | 4 |
| [§ 6.17.4.4](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-17-4-4) | Compute image size function semantics | `DOC-DECODER-OUTPUT-EQUIVALENCE-CONTRACT` | ✅ | — | — | ✅ | — |
| [§ 6.17.4.4](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-17-4-4) | Compute image size function semantics | `INFRA-RECON-FRAME-PLANE-TYPES` | ✅ | — | — | ✅ | — |
| [§ 6.17.4.4](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-17-4-4) | Compute image size function semantics | `RECON-CURRENT-FRAME-WORKSPACE` | ✅ | — | ✅ | ✅ | — |
| [§ 6.17.4.4](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-17-4-4) | Compute image size function semantics | `RECON-Y4M-OUTPUT-WRITER` | ✅ | — | ✅ | ✅ | — |
| [§ 6.17.5.2](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-17-5-2) | Deblocking filter params semantics | `AV2-5.18.5-FILTERING` | ✅ | 🟡 | — | 🟡 | — |
| [§ 6.17.6](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-17-6) | Quantization structures | `AV2-5.18.6-QUANTIZATION` | ✅ | 🟡 | 🟡 | 🟡 | 4 |
| [§ 6.17.7](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-17-7) | Segmentation and tiling structures | `AV2-5.18.7-SEGMENTATION-TILING` | ✅ | 🟡 | 🟡 | 🟡 | 5 |
| [§ 6.17.7](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-17-7) | Segmentation and tiling structures | `AV2-5.18.7.3-TILE-PARAMS` | ✅ | ✅ | ✅ | ✅ | 4 |
| [§ 6.17.7.2](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-17-7-2) | Tile info semantics | `DECODE-LIMITS-RUNTIME-API` | ✅ | — | — | ✅ | — |
| [§ 6.17.7.2](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-17-7-2) | Tile info semantics | `DECODE-MINIMAL-RAW-RUNTIME-OUTPUT` | ✅ | 🟡 | ✅ | ✅ | 4 |
| [§ 6.17.7.2](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-17-7-2) | Tile info semantics | `DECODE-MINIMAL-TIER-RUNTIME-SUCCESS` | ✅ | 🟡 | ✅ | ✅ | 3 |
| [§ 6.17.7.2](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-17-7-2) | Tile info semantics | `DECODE-TILE-PAYLOAD-INPUT-DERIVATION` | ✅ | 🟡 | — | ✅ | 2 |
| [§ 6.17.7.2](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-17-7-2) | Tile info semantics | `DECODE-Y4M-RUNTIME-OUTPUT` | ✅ | 🟡 | ✅ | ✅ | 4 |
| [§ 6.17.10.1](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-17-10-1) | Film grain config semantics | `AV2-5.18.10-FILM-GRAIN-STRUCTURES` | ✅ | ✅ | 🟡 | ✅ | 1 |
| [§ 6.17.10.2](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-17-10-2) | Film grain model semantics | `AV2-5.14-FILM-GRAIN` | ✅ | ✅ | 🟡 | ✅ | 6 |
| [§ 6.18](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-18) | Tile group OBU semantics | `AV2-5.19-TILE-GROUP` | ✅ | 🟡 | 🟡 | ✅ | 7 |
| [§ 6.18](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-18) | Tile group OBU semantics | `DECODE-LIMITS-RUNTIME-API` | ✅ | — | — | ✅ | — |
| [§ 6.18](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-18) | Tile group OBU semantics | `DECODE-MINIMAL-RAW-RUNTIME-OUTPUT` | ✅ | 🟡 | ✅ | ✅ | 4 |
| [§ 6.18](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-18) | Tile group OBU semantics | `DECODE-MINIMAL-TIER-RUNTIME-SUCCESS` | ✅ | 🟡 | ✅ | ✅ | 3 |
| [§ 6.18](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-18) | Tile group OBU semantics | `DECODE-TILE-PAYLOAD-INPUT-DERIVATION` | ✅ | 🟡 | — | ✅ | 2 |
| [§ 6.18](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-18) | Tile group OBU semantics | `DECODE-Y4M-RUNTIME-OUTPUT` | ✅ | 🟡 | ✅ | ✅ | 4 |
| [§ 6.19](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-19) | Tile group payload semantics | `AV2-5.19-TILE-GROUP` | ✅ | 🟡 | 🟡 | ✅ | 7 |
| [§ 6.19](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-19) | Tile group payload semantics | `AV2-5.20-TILE-GROUP-PAYLOAD` | ✅ | 🟡 | 🟡 | ✅ | 3 |
| [§ 6.19.1](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-19-1) | General tile group payload semantics | `CONF-TILE-PAYLOAD-DECODE-FUZZ` | ✅ | — | — | ✅ | 2 |
| [§ 6.19.1](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-19-1) | General tile group payload semantics | `DECODE-CONTEXT-TILE-PAYLOAD-HANDOFF` | ✅ | — | — | ✅ | 2 |
| [§ 6.19.1](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-19-1) | General tile group payload semantics | `DECODE-LIMITS-RUNTIME-API` | ✅ | — | — | ✅ | — |
| [§ 6.19.1](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-19-1) | General tile group payload semantics | `DECODE-MINIMAL-RAW-RUNTIME-OUTPUT` | ✅ | 🟡 | ✅ | ✅ | 4 |
| [§ 6.19.1](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-19-1) | General tile group payload semantics | `DECODE-MINIMAL-TIER-RUNTIME-SUCCESS` | ✅ | 🟡 | ✅ | ✅ | 3 |
| [§ 6.19.1](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-19-1) | General tile group payload semantics | `DECODE-TILE-CDF-SAVE-LIFECYCLE-BOUNDARY` | ✅ | ✅ | — | ✅ | 2 |
| [§ 6.19.1](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-19-1) | General tile group payload semantics | `DECODE-TILE-CDF-SELECTION-BOUNDARY` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 6.19.1](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-19-1) | General tile group payload semantics | `DECODE-TILE-PAYLOAD-BOUNDARY` | ✅ | 🟡 | — | ✅ | 2 |
| [§ 6.19.1](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-19-1) | General tile group payload semantics | `DECODE-TILE-PAYLOAD-INPUT-DERIVATION` | ✅ | 🟡 | — | ✅ | 2 |
| [§ 6.19.1](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-19-1) | General tile group payload semantics | `DECODE-Y4M-RUNTIME-OUTPUT` | ✅ | 🟡 | ✅ | ✅ | 4 |
| [§ 6.19.2.1](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-19-2-1) | Decode tile semantics | `CONF-TILE-PAYLOAD-DECODE-FUZZ` | ✅ | — | — | ✅ | 2 |
| [§ 6.19.2.1](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-19-2-1) | Decode tile semantics | `DECODE-TILE-MI-SIZE-STATE-BOUNDARY` | ✅ | — | — | ✅ | — |
| [§ 6.19.2.1](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-19-2-1) | Decode tile semantics | `DECODE-TILE-PARTITION-TRAVERSAL-BOUNDARY` | ✅ | ✅ | — | ✅ | — |
| [§ 6.19.3](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-19-3) | Partition structures | `CONF-TILE-PAYLOAD-DECODE-FUZZ` | ✅ | — | — | ✅ | 2 |
| [§ 6.19.3](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-19-3) | Partition structures | `DECODE-TILE-PARTITION-ALLOWED-BOUNDARY` | ✅ | — | — | ✅ | — |
| [§ 6.19.3](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-19-3) | Partition structures | `DECODE-TILE-PARTITION-DECISION-BOUNDARY` | ✅ | ✅ | — | ✅ | — |
| [§ 6.19.3](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-19-3) | Partition structures | `DECODE-TILE-PARTITION-SIZE-TABLE-BOUNDARY` | ✅ | — | — | ✅ | — |
| [§ 6.19.3](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-19-3) | Partition structures | `DECODE-TILE-PARTITION-TRAVERSAL-BOUNDARY` | ✅ | ✅ | — | ✅ | — |
| [§ 6.19.6.1](./spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-19-6-1) | TX size semantics | `DECODE-TX-SIZE-SYMBOLIC-TABLES` | ✅ | — | — | ✅ | 1 |

## Chapter 7 — Decoding process

| Section | Spec item | Feature | Mapped | Parse | Validate | Tests | Diagnostics |
|---|---|---|:-:|:-:|:-:|:-:|:-:|
| [§ 7.1](./spec/av2/1.0.0/07-decoding-process.md#s-7-1) | General decoding process | `CLI-DECODE` | ✅ | — | — | ✅ | 4 |
| [§ 7.1](./spec/av2/1.0.0/07-decoding-process.md#s-7-1) | General decoding process | `CLI-DECODE-HASH-OUTPUT` | ✅ | — | — | ✅ | 3 |
| [§ 7.1](./spec/av2/1.0.0/07-decoding-process.md#s-7-1) | General decoding process | `CONF-DECODE-RUNTIME-HASH-FUZZ` | ✅ | — | — | ✅ | 3 |
| [§ 7.1](./spec/av2/1.0.0/07-decoding-process.md#s-7-1) | General decoding process | `CONF-DECODE-RUNTIME-RAW-FUZZ` | ✅ | — | — | ✅ | 4 |
| [§ 7.1](./spec/av2/1.0.0/07-decoding-process.md#s-7-1) | General decoding process | `CONF-DECODE-RUNTIME-Y4M-FUZZ` | ✅ | — | — | ✅ | 4 |
| [§ 7.1](./spec/av2/1.0.0/07-decoding-process.md#s-7-1) | General decoding process | `CONF-RECON-FRAME-PLANE-TYPES-FUZZ` | ✅ | — | — | ✅ | — |
| [§ 7.1](./spec/av2/1.0.0/07-decoding-process.md#s-7-1) | General decoding process | `DECODE-BYTE-STREAM-PLANNER` | ✅ | ✅ | — | ✅ | 3 |
| [§ 7.1](./spec/av2/1.0.0/07-decoding-process.md#s-7-1) | General decoding process | `DECODE-CONTEXT-TILE-PAYLOAD-HANDOFF` | ✅ | — | — | ✅ | 2 |
| [§ 7.1](./spec/av2/1.0.0/07-decoding-process.md#s-7-1) | General decoding process | `DECODE-LIMITS-RUNTIME-API` | ✅ | — | — | ✅ | — |
| [§ 7.1](./spec/av2/1.0.0/07-decoding-process.md#s-7-1) | General decoding process | `DECODE-MINIMAL-RAW-RUNTIME-OUTPUT` | ✅ | 🟡 | ✅ | ✅ | 4 |
| [§ 7.1](./spec/av2/1.0.0/07-decoding-process.md#s-7-1) | General decoding process | `DECODE-MINIMAL-TIER-RUNTIME-SUCCESS` | ✅ | 🟡 | ✅ | ✅ | 3 |
| [§ 7.1](./spec/av2/1.0.0/07-decoding-process.md#s-7-1) | General decoding process | `DECODE-STREAM-STATE-PLANNER` | ✅ | — | — | ✅ | 1 |
| [§ 7.1](./spec/av2/1.0.0/07-decoding-process.md#s-7-1) | General decoding process | `DECODE-TILE-PAYLOAD-BOUNDARY` | ✅ | 🟡 | — | ✅ | 2 |
| [§ 7.1](./spec/av2/1.0.0/07-decoding-process.md#s-7-1) | General decoding process | `DECODE-UNSUPPORTED-DIAGNOSTIC-API` | ✅ | — | — | ✅ | 1 |
| [§ 7.1](./spec/av2/1.0.0/07-decoding-process.md#s-7-1) | General decoding process | `DECODE-Y4M-RUNTIME-OUTPUT` | ✅ | 🟡 | ✅ | ✅ | 4 |
| [§ 7.1](./spec/av2/1.0.0/07-decoding-process.md#s-7-1) | General decoding process | `INFRA-RECON-FRAME-PLANE-TYPES` | ✅ | — | — | ✅ | — |
| [§ 7.3](./spec/av2/1.0.0/07-decoding-process.md#s-7-3) | Ordering of OBUs | `AV2-7.3-OBU-ORDERING` | ✅ | — | 🟡 | ✅ | 4 |
| [§ 7.3](./spec/av2/1.0.0/07-decoding-process.md#s-7-3) | Ordering of OBUs | `DECODE-BYTE-STREAM-PLANNER` | ✅ | ✅ | — | ✅ | 3 |
| [§ 7.3](./spec/av2/1.0.0/07-decoding-process.md#s-7-3) | Ordering of OBUs | `DECODE-STREAM-STATE-PLANNER` | ✅ | — | — | ✅ | 1 |
| [§ 7.3.2](./spec/av2/1.0.0/07-decoding-process.md#s-7-3-2) | Coded multistream video sequence boundaries | `AV2-7.3.2-CMVS-BOUNDARIES` | ✅ | — | 🟡 | 🟡 | 1 |
| [§ 7.3.3](./spec/av2/1.0.0/07-decoding-process.md#s-7-3-3) | Coded output frame unit | `AV2-7.3.3-CODED-OUTPUT-FRAME-UNIT` | ✅ | — | 🟡 | ✅ | 7 |
| [§ 7.3.4](./spec/av2/1.0.0/07-decoding-process.md#s-7-3-4) | Coded non-output frame unit | `AV2-7.3.4-CODED-NONOUTPUT-FRAME-UNIT` | ✅ | — | 🟡 | ✅ | 1 |
| [§ 7.3.5](./spec/av2/1.0.0/07-decoding-process.md#s-7-3-5) | Coded frame unit | `AV2-7.3.5-CODED-FRAME-UNIT` | ✅ | — | 🟡 | ✅ | — |
| [§ 7.3.6](./spec/av2/1.0.0/07-decoding-process.md#s-7-3-6) | Coded extended layer unit | `AV2-7.3.6-CODED-EXTENDED-LAYER-UNIT` | ✅ | — | 🟡 | ✅ | 13 |
| [§ 7.3.7](./spec/av2/1.0.0/07-decoding-process.md#s-7-3-7) | Temporal unit | `AV2-7.3.7-TEMPORAL-UNIT-ORDER` | ✅ | ✅ | 🟡 | ✅ | 7 |
| [§ 7.3.8](./spec/av2/1.0.0/07-decoding-process.md#s-7-3-8) | Availability of high level syntax OBUs | `AV2-6.2.2-OBU-HEADER-ACTIVATED-SEQUENCE-LIMITS` | ✅ | ✅ | ✅ | ✅ | 3 |
| [§ 7.3.8](./spec/av2/1.0.0/07-decoding-process.md#s-7-3-8) | Availability of high level syntax OBUs | `AV2-7.3.8-HLS-AVAILABILITY` | ✅ | ✅ | 🟡 | ✅ | 14 |
| [§ 7.3.8.10](./spec/av2/1.0.0/07-decoding-process.md#s-7-3-8-10) | Content interpretation OBU availability | `AV2-7.3.3-CODED-OUTPUT-FRAME-UNIT` | ✅ | — | 🟡 | ✅ | 7 |
| [§ 7.3.9](./spec/av2/1.0.0/07-decoding-process.md#s-7-3-9) | Availability of long-term reference frames | `AV2-7.3.9-LONG-TERM-REFERENCE-AVAILABILITY` | ✅ | — | 🟡 | 🟡 | 1 |
| [§ 7.3.9](./spec/av2/1.0.0/07-decoding-process.md#s-7-3-9) | Availability of long-term reference frames | `AV2-7.4-RANDOM-ACCESS` | ✅ | — | 🟡 | 🟡 | 2 |
| [§ 7.4](./spec/av2/1.0.0/07-decoding-process.md#s-7-4) | Random access decoding | `DECODE-STREAM-STATE-PLANNER` | ✅ | — | — | ✅ | 1 |
| [§ 7.4.2](./spec/av2/1.0.0/07-decoding-process.md#s-7-4-2) | Random access and use of long-term reference frames | `AV2-7.4-RANDOM-ACCESS` | ✅ | — | 🟡 | 🟡 | 2 |
| [§ 7.4.4](./spec/av2/1.0.0/07-decoding-process.md#s-7-4-4) | Open Random Access | `AV2-7.4-RANDOM-ACCESS` | ✅ | — | 🟡 | 🟡 | 2 |
| [§ 7.4.5](./spec/av2/1.0.0/07-decoding-process.md#s-7-4-5) | Random Access Switch | `AV2-7.3.9-LONG-TERM-REFERENCE-AVAILABILITY` | ✅ | — | 🟡 | 🟡 | 1 |
| [§ 7.4.5](./spec/av2/1.0.0/07-decoding-process.md#s-7-4-5) | Random Access Switch | `AV2-7.4-RANDOM-ACCESS` | ✅ | — | 🟡 | 🟡 | 2 |
| [§ 7.4.6](./spec/av2/1.0.0/07-decoding-process.md#s-7-4-6) | Multistream Random Access | `AV2-7.3.7-TEMPORAL-UNIT-ORDER` | ✅ | ✅ | 🟡 | ✅ | 7 |
| [§ 7.5](./spec/av2/1.0.0/07-decoding-process.md#s-7-5) | Frame end update CDF process | `DECODE-TILE-CDF-SAVE-LIFECYCLE-BOUNDARY` | ✅ | ✅ | — | ✅ | 2 |
| [§ 7.13.2.1](./spec/av2/1.0.0/07-decoding-process.md#s-7-13-2-1) | General | `CONF-RECON-INTRA-PREDICTION-FUZZ` | ✅ | — | — | ✅ | — |
| [§ 7.13.2.1](./spec/av2/1.0.0/07-decoding-process.md#s-7-13-2-1) | General | `DECODE-MINIMAL-INTRA-RECONSTRUCTION-FRONTIER` | ✅ | 🟡 | ✅ | ✅ | 4 |
| [§ 7.13.2.1](./spec/av2/1.0.0/07-decoding-process.md#s-7-13-2-1) | General | `RECON-CURRENT-FRAME-WORKSPACE` | ✅ | — | ✅ | ✅ | — |
| [§ 7.13.2.1](./spec/av2/1.0.0/07-decoding-process.md#s-7-13-2-1) | General | `RECON-INTRA-CARDINAL-DIRECTIONAL-PREDICTION` | ✅ | — | ✅ | ✅ | — |
| [§ 7.13.2.1](./spec/av2/1.0.0/07-decoding-process.md#s-7-13-2-1) | General | `RECON-INTRA-DC-SUBSAMPLED-PREDICTION` | ✅ | — | ✅ | ✅ | — |
| [§ 7.13.2.1](./spec/av2/1.0.0/07-decoding-process.md#s-7-13-2-1) | General | `RECON-INTRA-IBP-DC-PREDICTION` | ✅ | — | ✅ | ✅ | — |
| [§ 7.13.2.1](./spec/av2/1.0.0/07-decoding-process.md#s-7-13-2-1) | General | `RECON-INTRA-MIDDLE-DIRECTIONAL-ANGLE-PREDICTION` | ✅ | — | ✅ | ✅ | — |
| [§ 7.13.2.1](./spec/av2/1.0.0/07-decoding-process.md#s-7-13-2-1) | General | `RECON-INTRA-ONE-SIDED-DIRECTIONAL-ANGLE-PREDICTION` | ✅ | — | ✅ | ✅ | — |
| [§ 7.13.2.1](./spec/av2/1.0.0/07-decoding-process.md#s-7-13-2-1) | General | `RECON-WORKSPACE-DIRECTIONAL-ANGLE-PREDICTION` | ✅ | — | ✅ | ✅ | — |
| [§ 7.13.2.2](./spec/av2/1.0.0/07-decoding-process.md#s-7-13-2-2) | Basic intra prediction process | `CONF-RECON-INTRA-PREDICTION-FUZZ` | ✅ | — | — | ✅ | — |
| [§ 7.13.2.2](./spec/av2/1.0.0/07-decoding-process.md#s-7-13-2-2) | Basic intra prediction process | `RECON-CURRENT-FRAME-WORKSPACE` | ✅ | — | ✅ | ✅ | — |
| [§ 7.13.2.2](./spec/av2/1.0.0/07-decoding-process.md#s-7-13-2-2) | Basic intra prediction process | `RECON-INTRA-BASIC-PAETH-PREDICTION` | ✅ | — | ✅ | ✅ | — |
| [§ 7.13.2.7](./spec/av2/1.0.0/07-decoding-process.md#s-7-13-2-7) | Directional intra prediction process | `RECON-INTRA-CARDINAL-DIRECTIONAL-PREDICTION` | ✅ | — | ✅ | ✅ | — |
| [§ 7.13.2.7](./spec/av2/1.0.0/07-decoding-process.md#s-7-13-2-7) | Directional intra prediction process | `RECON-INTRA-MIDDLE-DIRECTIONAL-ANGLE-PREDICTION` | ✅ | — | ✅ | ✅ | — |
| [§ 7.13.2.7](./spec/av2/1.0.0/07-decoding-process.md#s-7-13-2-7) | Directional intra prediction process | `RECON-INTRA-ONE-SIDED-DIRECTIONAL-ANGLE-PREDICTION` | ✅ | — | ✅ | ✅ | — |
| [§ 7.13.2.7](./spec/av2/1.0.0/07-decoding-process.md#s-7-13-2-7) | Directional intra prediction process | `RECON-WORKSPACE-DIRECTIONAL-ANGLE-PREDICTION` | ✅ | — | ✅ | ✅ | — |
| [§ 7.13.2.8](./spec/av2/1.0.0/07-decoding-process.md#s-7-13-2-8) | Single directional prediction process | `CONF-RECON-INTRA-PREDICTION-FUZZ` | ✅ | — | — | ✅ | — |
| [§ 7.13.2.8](./spec/av2/1.0.0/07-decoding-process.md#s-7-13-2-8) | Single directional prediction process | `DECODE-MINIMAL-INTRA-RECONSTRUCTION-FRONTIER` | ✅ | 🟡 | ✅ | ✅ | 4 |
| [§ 7.13.2.8](./spec/av2/1.0.0/07-decoding-process.md#s-7-13-2-8) | Single directional prediction process | `RECON-CURRENT-FRAME-WORKSPACE` | ✅ | — | ✅ | ✅ | — |
| [§ 7.13.2.8](./spec/av2/1.0.0/07-decoding-process.md#s-7-13-2-8) | Single directional prediction process | `RECON-INTRA-CARDINAL-DIRECTIONAL-PREDICTION` | ✅ | — | ✅ | ✅ | — |
| [§ 7.13.2.8](./spec/av2/1.0.0/07-decoding-process.md#s-7-13-2-8) | Single directional prediction process | `RECON-INTRA-MIDDLE-DIRECTIONAL-ANGLE-PREDICTION` | ✅ | — | ✅ | ✅ | — |
| [§ 7.13.2.8](./spec/av2/1.0.0/07-decoding-process.md#s-7-13-2-8) | Single directional prediction process | `RECON-INTRA-ONE-SIDED-DIRECTIONAL-ANGLE-PREDICTION` | ✅ | — | ✅ | ✅ | — |
| [§ 7.13.2.8](./spec/av2/1.0.0/07-decoding-process.md#s-7-13-2-8) | Single directional prediction process | `RECON-WORKSPACE-DIRECTIONAL-ANGLE-PREDICTION` | ✅ | — | ✅ | ✅ | — |
| [§ 7.13.2.10](./spec/av2/1.0.0/07-decoding-process.md#s-7-13-2-10) | DC intra prediction process | `CONF-RECON-INTRA-PREDICTION-FUZZ` | ✅ | — | — | ✅ | — |
| [§ 7.13.2.10](./spec/av2/1.0.0/07-decoding-process.md#s-7-13-2-10) | DC intra prediction process | `DECODE-MINIMAL-INTRA-RECONSTRUCTION-FRONTIER` | ✅ | 🟡 | ✅ | ✅ | 4 |
| [§ 7.13.2.10](./spec/av2/1.0.0/07-decoding-process.md#s-7-13-2-10) | DC intra prediction process | `ENC-CLOSED-LOOP-RECONSTRUCTION-MINIMAL` | ✅ | — | — | ✅ | — |
| [§ 7.13.2.10](./spec/av2/1.0.0/07-decoding-process.md#s-7-13-2-10) | DC intra prediction process | `RECON-CURRENT-FRAME-WORKSPACE` | ✅ | — | ✅ | ✅ | — |
| [§ 7.13.2.10](./spec/av2/1.0.0/07-decoding-process.md#s-7-13-2-10) | DC intra prediction process | `RECON-INTRA-DC-RECTANGULAR-PREDICTION` | ✅ | — | ✅ | ✅ | — |
| [§ 7.13.2.10](./spec/av2/1.0.0/07-decoding-process.md#s-7-13-2-10) | DC intra prediction process | `RECON-INTRA-DC-SQUARE-PREDICTION` | ✅ | — | ✅ | ✅ | — |
| [§ 7.13.2.10](./spec/av2/1.0.0/07-decoding-process.md#s-7-13-2-10) | DC intra prediction process | `RECON-INTRA-IBP-DC-PREDICTION` | ✅ | — | ✅ | ✅ | — |
| [§ 7.13.2.11](./spec/av2/1.0.0/07-decoding-process.md#s-7-13-2-11) | DC intra prediction subsampled process | `CONF-RECON-INTRA-PREDICTION-FUZZ` | ✅ | — | — | ✅ | — |
| [§ 7.13.2.11](./spec/av2/1.0.0/07-decoding-process.md#s-7-13-2-11) | DC intra prediction subsampled process | `RECON-CURRENT-FRAME-WORKSPACE` | ✅ | — | ✅ | ✅ | — |
| [§ 7.13.2.11](./spec/av2/1.0.0/07-decoding-process.md#s-7-13-2-11) | DC intra prediction subsampled process | `RECON-INTRA-DC-SUBSAMPLED-PREDICTION` | ✅ | — | ✅ | ✅ | — |
| [§ 7.13.2.12](./spec/av2/1.0.0/07-decoding-process.md#s-7-13-2-12) | IBP DC process | `CONF-RECON-INTRA-PREDICTION-FUZZ` | ✅ | — | — | ✅ | — |
| [§ 7.13.2.12](./spec/av2/1.0.0/07-decoding-process.md#s-7-13-2-12) | IBP DC process | `RECON-CURRENT-FRAME-WORKSPACE` | ✅ | — | ✅ | ✅ | — |
| [§ 7.13.2.12](./spec/av2/1.0.0/07-decoding-process.md#s-7-13-2-12) | IBP DC process | `RECON-INTRA-IBP-DC-PREDICTION` | ✅ | — | ✅ | ✅ | — |
| [§ 7.13.2.13](./spec/av2/1.0.0/07-decoding-process.md#s-7-13-2-13) | Smooth intra prediction process | `CONF-RECON-INTRA-PREDICTION-FUZZ` | ✅ | — | — | ✅ | — |
| [§ 7.13.2.13](./spec/av2/1.0.0/07-decoding-process.md#s-7-13-2-13) | Smooth intra prediction process | `RECON-CURRENT-FRAME-WORKSPACE` | ✅ | — | ✅ | ✅ | — |
| [§ 7.13.2.13](./spec/av2/1.0.0/07-decoding-process.md#s-7-13-2-13) | Smooth intra prediction process | `RECON-INTRA-SMOOTH-PREDICTION` | ✅ | — | ✅ | ✅ | — |
| [§ 7.13.3.22](./spec/av2/1.0.0/07-decoding-process.md#s-7-13-3-22) | Resolve divisor process | `CONF-RECON-INTRA-PREDICTION-FUZZ` | ✅ | — | — | ✅ | — |
| [§ 7.13.3.22](./spec/av2/1.0.0/07-decoding-process.md#s-7-13-3-22) | Resolve divisor process | `RECON-INTRA-DC-RECTANGULAR-PREDICTION` | ✅ | — | ✅ | ✅ | — |
| [§ 7.13.3.22](./spec/av2/1.0.0/07-decoding-process.md#s-7-13-3-22) | Resolve divisor process | `RECON-INTRA-DC-SQUARE-PREDICTION` | ✅ | — | ✅ | ✅ | — |
| [§ 7.13.3.22](./spec/av2/1.0.0/07-decoding-process.md#s-7-13-3-22) | Resolve divisor process | `RECON-INTRA-DC-SUBSAMPLED-PREDICTION` | ✅ | — | ✅ | ✅ | — |
| [§ 7.14.2](./spec/av2/1.0.0/07-decoding-process.md#s-7-14-2) | Dequantization functions | `ENC-CLOSED-LOOP-RECONSTRUCTION-MINIMAL` | ✅ | — | — | ✅ | — |
| [§ 7.14.2](./spec/av2/1.0.0/07-decoding-process.md#s-7-14-2) | Dequantization functions | `ENC-QUANTIZATION-V0` | ✅ | — | — | ✅ | — |
| [§ 7.14.2](./spec/av2/1.0.0/07-decoding-process.md#s-7-14-2) | Dequantization functions | `RECON-DEQUANT-QUANTIZER-INDEX-RESOLUTION` | ✅ | — | — | ✅ | — |
| [§ 7.14.2](./spec/av2/1.0.0/07-decoding-process.md#s-7-14-2) | Dequantization functions | `RECON-DEQUANT-QUANTIZER-LOOKUP` | ✅ | — | — | ✅ | — |
| [§ 7.14.3](./spec/av2/1.0.0/07-decoding-process.md#s-7-14-3) | Reconstruct process | `ENC-CLOSED-LOOP-RECONSTRUCTION-MINIMAL` | ✅ | — | — | ✅ | — |
| [§ 7.14.3](./spec/av2/1.0.0/07-decoding-process.md#s-7-14-3) | Reconstruct process | `ENC-RESIDUAL-FOUNDATION` | ✅ | — | — | ✅ | — |
| [§ 7.14.3](./spec/av2/1.0.0/07-decoding-process.md#s-7-14-3) | Reconstruct process | `RECON-RECONSTRUCT-TRANSFORM-BLOCK` | ✅ | — | ✅ | ✅ | — |
| [§ 7.14.3](./spec/av2/1.0.0/07-decoding-process.md#s-7-14-3) | Reconstruct process | `RECON-RESIDUAL-ADDITION` | ✅ | — | ✅ | ✅ | — |
| [§ 7.14.4](./spec/av2/1.0.0/07-decoding-process.md#s-7-14-4) | Dequantization process | `ENC-CLOSED-LOOP-RECONSTRUCTION-MINIMAL` | ✅ | — | — | ✅ | — |
| [§ 7.14.4](./spec/av2/1.0.0/07-decoding-process.md#s-7-14-4) | Dequantization process | `ENC-QUANTIZATION-V0` | ✅ | — | — | ✅ | — |
| [§ 7.14.4](./spec/av2/1.0.0/07-decoding-process.md#s-7-14-4) | Dequantization process | `RECON-DEQUANT-PROCESS` | ✅ | — | ✅ | ✅ | — |
| [§ 7.14.4](./spec/av2/1.0.0/07-decoding-process.md#s-7-14-4) | Dequantization process | `RECON-DEQUANT-QM-WEIGHT` | ✅ | — | ✅ | ✅ | — |
| [§ 7.14.4](./spec/av2/1.0.0/07-decoding-process.md#s-7-14-4) | Dequantization process | `RECON-RECONSTRUCT-TRANSFORM-BLOCK` | ✅ | — | ✅ | ✅ | — |
| [§ 7.15.2.1](./spec/av2/1.0.0/07-decoding-process.md#s-7-15-2-1) | 1d inverse transform process | `RECON-INVERSE-TRANSFORM-1D` | ✅ | — | ✅ | ✅ | — |
| [§ 7.15.2.2](./spec/av2/1.0.0/07-decoding-process.md#s-7-15-2-2) | Inverse Walsh-Hadamard transform process | `RECON-INVERSE-TRANSFORM-MATRIX-FREE` | ✅ | — | ✅ | ✅ | — |
| [§ 7.15.2.3](./spec/av2/1.0.0/07-decoding-process.md#s-7-15-2-3) | Inverse identity transform process | `RECON-INVERSE-TRANSFORM-MATRIX-FREE` | ✅ | — | ✅ | ✅ | — |
| [§ 7.15.3](./spec/av2/1.0.0/07-decoding-process.md#s-7-15-3) | Secondary transform process | `RECON-SECONDARY-INVERSE-TRANSFORM` | ✅ | — | ✅ | ✅ | — |
| [§ 7.15.4](./spec/av2/1.0.0/07-decoding-process.md#s-7-15-4) | 2D inverse transform process | `ENC-CLOSED-LOOP-RECONSTRUCTION-MINIMAL` | ✅ | — | — | ✅ | — |
| [§ 7.15.4](./spec/av2/1.0.0/07-decoding-process.md#s-7-15-4) | 2D inverse transform process | `ENC-FORWARD-TRANSFORM-FOUNDATION` | ✅ | — | — | ✅ | — |
| [§ 7.15.4](./spec/av2/1.0.0/07-decoding-process.md#s-7-15-4) | 2D inverse transform process | `ENC-QUANTIZATION-V0` | ✅ | — | — | ✅ | — |
| [§ 7.15.4](./spec/av2/1.0.0/07-decoding-process.md#s-7-15-4) | 2D inverse transform process | `RECON-DPCM-DIRECTION` | ✅ | — | ✅ | ✅ | — |
| [§ 7.15.4](./spec/av2/1.0.0/07-decoding-process.md#s-7-15-4) | 2D inverse transform process | `RECON-GET-TRANSFORM-1D-TYPE` | ✅ | — | ✅ | ✅ | — |
| [§ 7.15.4](./spec/av2/1.0.0/07-decoding-process.md#s-7-15-4) | 2D inverse transform process | `RECON-INVERSE-TRANSFORM-2D-OUTER` | ✅ | — | ✅ | ✅ | — |
| [§ 7.15.4](./spec/av2/1.0.0/07-decoding-process.md#s-7-15-4) | 2D inverse transform process | `RECON-RECONSTRUCT-TRANSFORM-BLOCK` | ✅ | — | ✅ | ✅ | — |
| [§ 7.15.4](./spec/av2/1.0.0/07-decoding-process.md#s-7-15-4) | 2D inverse transform process | `RECON-RESOLVE-2D-TRANSFORM-PARAMS` | ✅ | — | ✅ | ✅ | — |
| [§ 7.15.4](./spec/av2/1.0.0/07-decoding-process.md#s-7-15-4) | 2D inverse transform process | `RECON-TRANSFORM-SHIFT-LOOKUP` | ✅ | — | ✅ | ✅ | — |
| [§ 7.15.4.1](./spec/av2/1.0.0/07-decoding-process.md#s-7-15-4-1) | 2D matrix transform process | `RECON-INVERSE-TRANSFORM-1D` | ✅ | — | ✅ | ✅ | — |
| [§ 7.15.4.1](./spec/av2/1.0.0/07-decoding-process.md#s-7-15-4-1) | 2D matrix transform process | `RECON-INVERSE-TRANSFORM-2D` | ✅ | — | ✅ | ✅ | — |
| [§ 7.17.3](./spec/av2/1.0.0/07-decoding-process.md#s-7-17-3) | Filter maximum width process | `RECON-DEBLOCK-FILTER-MAX-WIDTH` | ✅ | — | ✅ | ✅ | — |
| [§ 7.17.5](./spec/av2/1.0.0/07-decoding-process.md#s-7-17-5) | Adaptive filter strength process | `RECON-DEBLOCK-ADAPTIVE-STRENGTH` | ✅ | — | ✅ | ✅ | — |
| [§ 7.17.7.1](./spec/av2/1.0.0/07-decoding-process.md#s-7-17-7-1) | General | `RECON-DEBLOCK-SAMPLE-FILTER` | ✅ | — | ✅ | ✅ | — |
| [§ 7.17.7.2](./spec/av2/1.0.0/07-decoding-process.md#s-7-17-7-2) | Filter choice process | `RECON-DEBLOCK-FILTER-CHOICE` | ✅ | — | ✅ | ✅ | — |
| [§ 7.21](./spec/av2/1.0.0/07-decoding-process.md#s-7-21) | Output processes | `CLI-DECODE-HASH-OUTPUT` | ✅ | — | — | ✅ | 3 |
| [§ 7.21](./spec/av2/1.0.0/07-decoding-process.md#s-7-21) | Output processes | `CONF-DECODE-RUNTIME-HASH-FUZZ` | ✅ | — | — | ✅ | 3 |
| [§ 7.21](./spec/av2/1.0.0/07-decoding-process.md#s-7-21) | Output processes | `DECODE-LIMITS-RUNTIME-API` | ✅ | — | — | ✅ | — |
| [§ 7.21](./spec/av2/1.0.0/07-decoding-process.md#s-7-21) | Output processes | `DOC-DECODER-OUTPUT-EQUIVALENCE-CONTRACT` | ✅ | — | — | ✅ | — |
| [§ 7.21.1](./spec/av2/1.0.0/07-decoding-process.md#s-7-21-1) | Output process | `CONF-DECODE-RUNTIME-RAW-FUZZ` | ✅ | — | — | ✅ | 4 |
| [§ 7.21.1](./spec/av2/1.0.0/07-decoding-process.md#s-7-21-1) | Output process | `CONF-DECODE-RUNTIME-Y4M-FUZZ` | ✅ | — | — | ✅ | 4 |
| [§ 7.21.1](./spec/av2/1.0.0/07-decoding-process.md#s-7-21-1) | Output process | `CONF-RECON-FRAME-PLANE-TYPES-FUZZ` | ✅ | — | — | ✅ | — |
| [§ 7.21.1](./spec/av2/1.0.0/07-decoding-process.md#s-7-21-1) | Output process | `CONF-RECON-Y4M-OUTPUT-FUZZ` | ✅ | — | — | ✅ | — |
| [§ 7.21.1](./spec/av2/1.0.0/07-decoding-process.md#s-7-21-1) | Output process | `DECODE-MINIMAL-INTRA-RECONSTRUCTION-FRONTIER` | ✅ | 🟡 | ✅ | ✅ | 4 |
| [§ 7.21.1](./spec/av2/1.0.0/07-decoding-process.md#s-7-21-1) | Output process | `DECODE-MINIMAL-RAW-RUNTIME-OUTPUT` | ✅ | 🟡 | ✅ | ✅ | 4 |
| [§ 7.21.1](./spec/av2/1.0.0/07-decoding-process.md#s-7-21-1) | Output process | `DECODE-MINIMAL-TIER-RUNTIME-SUCCESS` | ✅ | 🟡 | ✅ | ✅ | 3 |
| [§ 7.21.1](./spec/av2/1.0.0/07-decoding-process.md#s-7-21-1) | Output process | `DECODE-Y4M-RUNTIME-OUTPUT` | ✅ | 🟡 | ✅ | ✅ | 4 |
| [§ 7.21.1](./spec/av2/1.0.0/07-decoding-process.md#s-7-21-1) | Output process | `DOC-DECODER-OUTPUT-EQUIVALENCE-CONTRACT` | ✅ | — | — | ✅ | — |
| [§ 7.21.1](./spec/av2/1.0.0/07-decoding-process.md#s-7-21-1) | Output process | `INFRA-RECON-FRAME-PLANE-TYPES` | ✅ | — | — | ✅ | — |
| [§ 7.21.1](./spec/av2/1.0.0/07-decoding-process.md#s-7-21-1) | Output process | `RECON-Y4M-OUTPUT-WRITER` | ✅ | — | ✅ | ✅ | — |
| [§ 7.21.2](./spec/av2/1.0.0/07-decoding-process.md#s-7-21-2) | Intermediate output preparation process | `CONF-DECODE-RUNTIME-RAW-FUZZ` | ✅ | — | — | ✅ | 4 |
| [§ 7.21.2](./spec/av2/1.0.0/07-decoding-process.md#s-7-21-2) | Intermediate output preparation process | `CONF-DECODE-RUNTIME-Y4M-FUZZ` | ✅ | — | — | ✅ | 4 |
| [§ 7.21.2](./spec/av2/1.0.0/07-decoding-process.md#s-7-21-2) | Intermediate output preparation process | `CONF-RECON-FRAME-HASH-FUZZ` | ✅ | — | — | ✅ | — |
| [§ 7.21.2](./spec/av2/1.0.0/07-decoding-process.md#s-7-21-2) | Intermediate output preparation process | `CONF-RECON-FRAME-PLANE-TYPES-FUZZ` | ✅ | — | — | ✅ | — |
| [§ 7.21.2](./spec/av2/1.0.0/07-decoding-process.md#s-7-21-2) | Intermediate output preparation process | `CONF-RECON-Y4M-OUTPUT-FUZZ` | ✅ | — | — | ✅ | — |
| [§ 7.21.2](./spec/av2/1.0.0/07-decoding-process.md#s-7-21-2) | Intermediate output preparation process | `DECODE-MINIMAL-INTRA-RECONSTRUCTION-FRONTIER` | ✅ | 🟡 | ✅ | ✅ | 4 |
| [§ 7.21.2](./spec/av2/1.0.0/07-decoding-process.md#s-7-21-2) | Intermediate output preparation process | `DECODE-MINIMAL-RAW-RUNTIME-OUTPUT` | ✅ | 🟡 | ✅ | ✅ | 4 |
| [§ 7.21.2](./spec/av2/1.0.0/07-decoding-process.md#s-7-21-2) | Intermediate output preparation process | `DECODE-MINIMAL-TIER-RUNTIME-SUCCESS` | ✅ | 🟡 | ✅ | ✅ | 3 |
| [§ 7.21.2](./spec/av2/1.0.0/07-decoding-process.md#s-7-21-2) | Intermediate output preparation process | `DECODE-Y4M-RUNTIME-OUTPUT` | ✅ | 🟡 | ✅ | ✅ | 4 |
| [§ 7.21.2](./spec/av2/1.0.0/07-decoding-process.md#s-7-21-2) | Intermediate output preparation process | `DOC-DECODER-OUTPUT-EQUIVALENCE-CONTRACT` | ✅ | — | — | ✅ | — |
| [§ 7.21.2](./spec/av2/1.0.0/07-decoding-process.md#s-7-21-2) | Intermediate output preparation process | `INFRA-RECON-FRAME-PLANE-TYPES` | ✅ | — | — | ✅ | — |
| [§ 7.21.2](./spec/av2/1.0.0/07-decoding-process.md#s-7-21-2) | Intermediate output preparation process | `RECON-FRAME-HASH-DIGEST` | ✅ | — | — | ✅ | — |
| [§ 7.21.2](./spec/av2/1.0.0/07-decoding-process.md#s-7-21-2) | Intermediate output preparation process | `RECON-HASH-INPUT-SERIALIZATION` | ✅ | — | — | ✅ | — |
| [§ 7.21.2](./spec/av2/1.0.0/07-decoding-process.md#s-7-21-2) | Intermediate output preparation process | `RECON-Y4M-OUTPUT-WRITER` | ✅ | — | ✅ | ✅ | — |
| [§ 7.21.3](./spec/av2/1.0.0/07-decoding-process.md#s-7-21-3) | Output successive frames process | `DOC-DECODER-OUTPUT-EQUIVALENCE-CONTRACT` | ✅ | — | — | ✅ | — |
| [§ 7.21.4](./spec/av2/1.0.0/07-decoding-process.md#s-7-21-4) | Output implicit output frame process | `DOC-DECODER-OUTPUT-EQUIVALENCE-CONTRACT` | ✅ | — | — | ✅ | — |
| [§ 7.21.5](./spec/av2/1.0.0/07-decoding-process.md#s-7-21-5) | Flush implicit output frames process | `DOC-DECODER-OUTPUT-EQUIVALENCE-CONTRACT` | ✅ | — | — | ✅ | — |
| [§ 7.21.6](./spec/av2/1.0.0/07-decoding-process.md#s-7-21-6) | Output frame buffers process | `DOC-DECODER-OUTPUT-EQUIVALENCE-CONTRACT` | ✅ | — | — | ✅ | — |
| [§ 7.21.7](./spec/av2/1.0.0/07-decoding-process.md#s-7-21-7) | Film grain synthesis process | `DOC-DECODER-OUTPUT-EQUIVALENCE-CONTRACT` | ✅ | — | — | ✅ | — |
| [§ 7.22](./spec/av2/1.0.0/07-decoding-process.md#s-7-22) | Motion field motion vector storage process | `DOC-DECODER-OUTPUT-EQUIVALENCE-CONTRACT` | ✅ | — | — | ✅ | — |
| [§ 7.23](./spec/av2/1.0.0/07-decoding-process.md#s-7-23) | Reference frame update process | `AV2-7.23-REFERENCE-FRAME-UPDATE` | ✅ | — | 🟡 | ✅ | 1 |
| [§ 7.23](./spec/av2/1.0.0/07-decoding-process.md#s-7-23) | Reference frame update process | `CONF-RECON-FRAME-PLANE-TYPES-FUZZ` | ✅ | — | — | ✅ | — |
| [§ 7.23](./spec/av2/1.0.0/07-decoding-process.md#s-7-23) | Reference frame update process | `DECODE-LIMITS-RUNTIME-API` | ✅ | — | — | ✅ | — |
| [§ 7.23](./spec/av2/1.0.0/07-decoding-process.md#s-7-23) | Reference frame update process | `DOC-DECODER-OUTPUT-EQUIVALENCE-CONTRACT` | ✅ | — | — | ✅ | — |
| [§ 7.23](./spec/av2/1.0.0/07-decoding-process.md#s-7-23) | Reference frame update process | `INFRA-RECON-FRAME-PLANE-TYPES` | ✅ | — | — | ✅ | — |
| [§ 7.23](./spec/av2/1.0.0/07-decoding-process.md#s-7-23) | Reference frame update process | `RECON-REFERENCE-FRAME-STORE` | ✅ | — | — | ✅ | — |

## Chapter 8 — Parsing process

| Section | Spec item | Feature | Mapped | Parse | Validate | Tests | Diagnostics |
|---|---|---|:-:|:-:|:-:|:-:|:-:|
| [§ 8.2](./spec/av2/1.0.0/08-parsing-process.md#s-8-2) | Parsing process for symbol decoder | `ENC-COEFF-BASE-LF-TOKEN` | ✅ | — | — | ✅ | — |
| [§ 8.2](./spec/av2/1.0.0/08-parsing-process.md#s-8-2) | Parsing process for symbol decoder | `ENC-COEFF-MULTI-TOKENS` | ✅ | — | — | ✅ | — |
| [§ 8.2](./spec/av2/1.0.0/08-parsing-process.md#s-8-2) | Parsing process for symbol decoder | `ENC-COEFFICIENT-TOKENIZATION-MINIMAL` | ✅ | — | — | ✅ | — |
| [§ 8.2](./spec/av2/1.0.0/08-parsing-process.md#s-8-2) | Parsing process for symbol decoder | `ENC-INTRA-BLOCK-MODE-TRACE` | ✅ | — | — | ✅ | — |
| [§ 8.2](./spec/av2/1.0.0/08-parsing-process.md#s-8-2) | Parsing process for symbol decoder | `ENC-INTRA-BLOCK-TRACE-BYPASS-LITERAL` | ✅ | — | — | ✅ | — |
| [§ 8.2](./spec/av2/1.0.0/08-parsing-process.md#s-8-2) | Parsing process for symbol decoder | `ENC-INTRA-BLOCK-TRACE-CHROMA-SKIP` | ✅ | — | — | ✅ | — |
| [§ 8.2](./spec/av2/1.0.0/08-parsing-process.md#s-8-2) | Parsing process for symbol decoder | `ENC-INTRA-BLOCK-TRACE-CODED-BR` | ✅ | — | — | ✅ | — |
| [§ 8.2](./spec/av2/1.0.0/08-parsing-process.md#s-8-2) | Parsing process for symbol decoder | `ENC-INTRA-BLOCK-TRACE-CODED-CHROMA-DC` | ✅ | — | — | ✅ | — |
| [§ 8.2](./spec/av2/1.0.0/08-parsing-process.md#s-8-2) | Parsing process for symbol decoder | `ENC-INTRA-BLOCK-TRACE-CODED-DC` | ✅ | — | — | ✅ | — |
| [§ 8.2](./spec/av2/1.0.0/08-parsing-process.md#s-8-2) | Parsing process for symbol decoder | `ENC-INTRA-BLOCK-TRACE-GOLOMB-FINITE` | ✅ | — | — | ✅ | — |
| [§ 8.2](./spec/av2/1.0.0/08-parsing-process.md#s-8-2) | Parsing process for symbol decoder | `ENC-INTRA-BLOCK-TRACE-GOLOMB-PREFIX` | ✅ | — | — | ✅ | — |
| [§ 8.2](./spec/av2/1.0.0/08-parsing-process.md#s-8-2) | Parsing process for symbol decoder | `ENC-INTRA-BLOCK-TRACE-LUMA-SKIP` | ✅ | — | — | ✅ | — |
| [§ 8.2](./spec/av2/1.0.0/08-parsing-process.md#s-8-2) | Parsing process for symbol decoder | `ENC-INTRA-BLOCK-TRACE-TWO-COEFF` | ✅ | — | — | ✅ | — |
| [§ 8.2](./spec/av2/1.0.0/08-parsing-process.md#s-8-2) | Parsing process for symbol decoder | `ENC-INTRA-BLOCK-TRACE-TWO-COEFF-TX-TYPE` | ✅ | — | — | ✅ | — |
| [§ 8.2](./spec/av2/1.0.0/08-parsing-process.md#s-8-2) | Parsing process for symbol decoder | `ENC-INTRA-TX-TYPE-TOKEN` | ✅ | — | — | ✅ | — |
| [§ 8.2](./spec/av2/1.0.0/08-parsing-process.md#s-8-2) | Parsing process for symbol decoder | `ENC-SEC-TX-TYPE-TOKEN` | ✅ | — | — | ✅ | — |
| [§ 8.2.2](./spec/av2/1.0.0/08-parsing-process.md#s-8-2-2) | Initialization process for symbol decoder | `AV2-8.2-SYMBOL-DECODER` | ✅ | ✅ | ✅ | ✅ | — |
| [§ 8.2.2](./spec/av2/1.0.0/08-parsing-process.md#s-8-2-2) | Initialization process for symbol decoder | `CONF-SYMBOL-DECODER-FUZZ` | ✅ | — | — | ✅ | — |
| [§ 8.2.2](./spec/av2/1.0.0/08-parsing-process.md#s-8-2-2) | Initialization process for symbol decoder | `CONF-TILE-PAYLOAD-DECODE-FUZZ` | ✅ | — | — | ✅ | 2 |
| [§ 8.2.2](./spec/av2/1.0.0/08-parsing-process.md#s-8-2-2) | Initialization process for symbol decoder | `DECODE-CONTEXT-TILE-PAYLOAD-HANDOFF` | ✅ | — | — | ✅ | 2 |
| [§ 8.2.2](./spec/av2/1.0.0/08-parsing-process.md#s-8-2-2) | Initialization process for symbol decoder | `DECODE-MINIMAL-RAW-RUNTIME-OUTPUT` | ✅ | 🟡 | ✅ | ✅ | 4 |
| [§ 8.2.2](./spec/av2/1.0.0/08-parsing-process.md#s-8-2-2) | Initialization process for symbol decoder | `DECODE-MINIMAL-TIER-RUNTIME-SUCCESS` | ✅ | 🟡 | ✅ | ✅ | 3 |
| [§ 8.2.2](./spec/av2/1.0.0/08-parsing-process.md#s-8-2-2) | Initialization process for symbol decoder | `DECODE-TILE-CDF-SAVE-LIFECYCLE-BOUNDARY` | ✅ | ✅ | — | ✅ | 2 |
| [§ 8.2.2](./spec/av2/1.0.0/08-parsing-process.md#s-8-2-2) | Initialization process for symbol decoder | `DECODE-TILE-CDF-SELECTION-BOUNDARY` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 8.2.2](./spec/av2/1.0.0/08-parsing-process.md#s-8-2-2) | Initialization process for symbol decoder | `DECODE-TILE-PAYLOAD-BOUNDARY` | ✅ | 🟡 | — | ✅ | 2 |
| [§ 8.2.2](./spec/av2/1.0.0/08-parsing-process.md#s-8-2-2) | Initialization process for symbol decoder | `DECODE-TILE-PAYLOAD-INPUT-DERIVATION` | ✅ | 🟡 | — | ✅ | 2 |
| [§ 8.2.2](./spec/av2/1.0.0/08-parsing-process.md#s-8-2-2) | Initialization process for symbol decoder | `DECODE-Y4M-RUNTIME-OUTPUT` | ✅ | 🟡 | ✅ | ✅ | 4 |
| [§ 8.2.2](./spec/av2/1.0.0/08-parsing-process.md#s-8-2-2) | Initialization process for symbol decoder | `ENC-BITSTREAM-WRITER` | 🟡 | — | — | 🟡 | — |
| [§ 8.2.3](./spec/av2/1.0.0/08-parsing-process.md#s-8-2-3) | Boolean decoding process | `AV2-8.2-SYMBOL-DECODER` | ✅ | ✅ | ✅ | ✅ | — |
| [§ 8.2.3](./spec/av2/1.0.0/08-parsing-process.md#s-8-2-3) | Boolean decoding process | `CONF-SYMBOL-DECODER-FUZZ` | ✅ | — | — | ✅ | — |
| [§ 8.2.3](./spec/av2/1.0.0/08-parsing-process.md#s-8-2-3) | Boolean decoding process | `ENC-BITSTREAM-WRITER` | 🟡 | — | — | 🟡 | — |
| [§ 8.2.4](./spec/av2/1.0.0/08-parsing-process.md#s-8-2-4) | Exit process for symbol decoder | `AV2-8.2-SYMBOL-DECODER` | ✅ | ✅ | ✅ | ✅ | — |
| [§ 8.2.4](./spec/av2/1.0.0/08-parsing-process.md#s-8-2-4) | Exit process for symbol decoder | `CONF-SYMBOL-DECODER-FUZZ` | ✅ | — | — | ✅ | — |
| [§ 8.2.4](./spec/av2/1.0.0/08-parsing-process.md#s-8-2-4) | Exit process for symbol decoder | `CONF-TILE-PAYLOAD-DECODE-FUZZ` | ✅ | — | — | ✅ | 2 |
| [§ 8.2.4](./spec/av2/1.0.0/08-parsing-process.md#s-8-2-4) | Exit process for symbol decoder | `DECODE-MINIMAL-BLOCK-SYNTAX-FRONTIER` | ✅ | 🟡 | — | ✅ | 4 |
| [§ 8.2.4](./spec/av2/1.0.0/08-parsing-process.md#s-8-2-4) | Exit process for symbol decoder | `DECODE-MINIMAL-RAW-RUNTIME-OUTPUT` | ✅ | 🟡 | ✅ | ✅ | 4 |
| [§ 8.2.4](./spec/av2/1.0.0/08-parsing-process.md#s-8-2-4) | Exit process for symbol decoder | `DECODE-MINIMAL-TIER-RUNTIME-SUCCESS` | ✅ | 🟡 | ✅ | ✅ | 3 |
| [§ 8.2.4](./spec/av2/1.0.0/08-parsing-process.md#s-8-2-4) | Exit process for symbol decoder | `DECODE-TILE-CDF-SAVE-LIFECYCLE-BOUNDARY` | ✅ | ✅ | — | ✅ | 2 |
| [§ 8.2.4](./spec/av2/1.0.0/08-parsing-process.md#s-8-2-4) | Exit process for symbol decoder | `DECODE-TILE-CDF-SELECTION-BOUNDARY` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 8.2.4](./spec/av2/1.0.0/08-parsing-process.md#s-8-2-4) | Exit process for symbol decoder | `DECODE-TILE-PAYLOAD-INPUT-DERIVATION` | ✅ | 🟡 | — | ✅ | 2 |
| [§ 8.2.4](./spec/av2/1.0.0/08-parsing-process.md#s-8-2-4) | Exit process for symbol decoder | `DECODE-Y4M-RUNTIME-OUTPUT` | ✅ | 🟡 | ✅ | ✅ | 4 |
| [§ 8.2.4](./spec/av2/1.0.0/08-parsing-process.md#s-8-2-4) | Exit process for symbol decoder | `ENC-BITSTREAM-WRITER` | 🟡 | — | — | 🟡 | — |
| [§ 8.2.5](./spec/av2/1.0.0/08-parsing-process.md#s-8-2-5) | Parsing process for read_literal | `AV2-8.2-SYMBOL-DECODER` | ✅ | ✅ | ✅ | ✅ | — |
| [§ 8.2.5](./spec/av2/1.0.0/08-parsing-process.md#s-8-2-5) | Parsing process for read_literal | `CONF-SYMBOL-DECODER-FUZZ` | ✅ | — | — | ✅ | — |
| [§ 8.2.5](./spec/av2/1.0.0/08-parsing-process.md#s-8-2-5) | Parsing process for read_literal | `DECODE-COEFF-BASE-DERIVED-LEVEL-PASS` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 8.2.5](./spec/av2/1.0.0/08-parsing-process.md#s-8-2-5) | Parsing process for read_literal | `DECODE-COEFF-BASE-PH-CDF-ROW` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 8.2.5](./spec/av2/1.0.0/08-parsing-process.md#s-8-2-5) | Parsing process for read_literal | `DECODE-COEFF-BASE-SYMBOL-READ` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 8.2.5](./spec/av2/1.0.0/08-parsing-process.md#s-8-2-5) | Parsing process for read_literal | `DECODE-COEFF-EOB-BRANCH-HANDOFF` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 8.2.5](./spec/av2/1.0.0/08-parsing-process.md#s-8-2-5) | Parsing process for read_literal | `DECODE-COEFF-EOB-DERIVED-SYMBOL-READ` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 8.2.5](./spec/av2/1.0.0/08-parsing-process.md#s-8-2-5) | Parsing process for read_literal | `DECODE-COEFF-EOB-SYMBOL-READ` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 8.2.5](./spec/av2/1.0.0/08-parsing-process.md#s-8-2-5) | Parsing process for read_literal | `DECODE-COEFF-FSC-BRANCH-HANDOFF` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 8.2.5](./spec/av2/1.0.0/08-parsing-process.md#s-8-2-5) | Parsing process for read_literal | `DECODE-COEFF-FSC-BRANCH-SEG-EOB-HANDOFF` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 8.2.5](./spec/av2/1.0.0/08-parsing-process.md#s-8-2-5) | Parsing process for read_literal | `DECODE-COEFF-FSC-CONTEXT-COMMIT` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 8.2.5](./spec/av2/1.0.0/08-parsing-process.md#s-8-2-5) | Parsing process for read_literal | `DECODE-COEFF-FSC-LEVEL-PASS` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 8.2.5](./spec/av2/1.0.0/08-parsing-process.md#s-8-2-5) | Parsing process for read_literal | `DECODE-COEFF-FSC-QUANT-PASS` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 8.2.5](./spec/av2/1.0.0/08-parsing-process.md#s-8-2-5) | Parsing process for read_literal | `DECODE-COEFF-FSC-SIGN-PASS` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 8.2.5](./spec/av2/1.0.0/08-parsing-process.md#s-8-2-5) | Parsing process for read_literal | `DECODE-COEFF-IDTX-CDF-ROWS` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 8.2.5](./spec/av2/1.0.0/08-parsing-process.md#s-8-2-5) | Parsing process for read_literal | `DECODE-COEFF-NONZERO-BLOCK-STATE` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 8.2.5](./spec/av2/1.0.0/08-parsing-process.md#s-8-2-5) | Parsing process for read_literal | `DECODE-COEFF-NONZERO-CONTEXT-COMMIT` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 8.2.5](./spec/av2/1.0.0/08-parsing-process.md#s-8-2-5) | Parsing process for read_literal | `DECODE-COEFF-ORDINARY-BRANCH-HANDOFF` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 8.2.5](./spec/av2/1.0.0/08-parsing-process.md#s-8-2-5) | Parsing process for read_literal | `DECODE-COEFF-ORDINARY-DERIVED-BASE-PASS` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 8.2.5](./spec/av2/1.0.0/08-parsing-process.md#s-8-2-5) | Parsing process for read_literal | `DECODE-COEFF-ORDINARY-DERIVED-SIGN-PASS` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 8.2.5](./spec/av2/1.0.0/08-parsing-process.md#s-8-2-5) | Parsing process for read_literal | `DECODE-COEFF-ORDINARY-PASS-COMPOSE` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 8.2.5](./spec/av2/1.0.0/08-parsing-process.md#s-8-2-5) | Parsing process for read_literal | `DECODE-COEFF-QUANT-PASS-COMPOSE` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 8.2.5](./spec/av2/1.0.0/08-parsing-process.md#s-8-2-5) | Parsing process for read_literal | `DECODE-COEFF-QUANT-PASS-MAXLEVEL-HANDOFF` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 8.2.5](./spec/av2/1.0.0/08-parsing-process.md#s-8-2-5) | Parsing process for read_literal | `DECODE-COEFF-READ-QUANT-SYNTAX` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 8.2.5](./spec/av2/1.0.0/08-parsing-process.md#s-8-2-5) | Parsing process for read_literal | `DECODE-COEFF-SIGN-SYMBOL-READ` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 8.2.5](./spec/av2/1.0.0/08-parsing-process.md#s-8-2-5) | Parsing process for read_literal | `DECODE-COEFF-STATE-CONTEXT-HANDOFF` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 8.2.5](./spec/av2/1.0.0/08-parsing-process.md#s-8-2-5) | Parsing process for read_literal | `DECODE-TILE-PARTITION-DECISION-BOUNDARY` | ✅ | ✅ | — | ✅ | — |
| [§ 8.2.5](./spec/av2/1.0.0/08-parsing-process.md#s-8-2-5) | Parsing process for read_literal | `DECODE-TILE-PARTITION-TRAVERSAL-BOUNDARY` | ✅ | ✅ | — | ✅ | — |
| [§ 8.2.5](./spec/av2/1.0.0/08-parsing-process.md#s-8-2-5) | Parsing process for read_literal | `ENC-BITSTREAM-WRITER` | 🟡 | — | — | 🟡 | — |
| [§ 8.2.6](./spec/av2/1.0.0/08-parsing-process.md#s-8-2-6) | Symbol decoding process | `AV2-8.2-SYMBOL-DECODER` | ✅ | ✅ | ✅ | ✅ | — |
| [§ 8.2.6](./spec/av2/1.0.0/08-parsing-process.md#s-8-2-6) | Symbol decoding process | `CONF-SYMBOL-DECODER-FUZZ` | ✅ | — | — | ✅ | — |
| [§ 8.2.6](./spec/av2/1.0.0/08-parsing-process.md#s-8-2-6) | Symbol decoding process | `CONF-TILE-PAYLOAD-DECODE-FUZZ` | ✅ | — | — | ✅ | 2 |
| [§ 8.2.6](./spec/av2/1.0.0/08-parsing-process.md#s-8-2-6) | Symbol decoding process | `DECODE-COEFF-BASE-DERIVED-LEVEL-PASS` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 8.2.6](./spec/av2/1.0.0/08-parsing-process.md#s-8-2-6) | Symbol decoding process | `DECODE-COEFF-BASE-PH-CDF-ROW` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 8.2.6](./spec/av2/1.0.0/08-parsing-process.md#s-8-2-6) | Symbol decoding process | `DECODE-COEFF-BASE-SYMBOL-READ` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 8.2.6](./spec/av2/1.0.0/08-parsing-process.md#s-8-2-6) | Symbol decoding process | `DECODE-COEFF-EOB-BRANCH-HANDOFF` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 8.2.6](./spec/av2/1.0.0/08-parsing-process.md#s-8-2-6) | Symbol decoding process | `DECODE-COEFF-EOB-DERIVED-SYMBOL-READ` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 8.2.6](./spec/av2/1.0.0/08-parsing-process.md#s-8-2-6) | Symbol decoding process | `DECODE-COEFF-EOB-SYMBOL-READ` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 8.2.6](./spec/av2/1.0.0/08-parsing-process.md#s-8-2-6) | Symbol decoding process | `DECODE-COEFF-FSC-LEVEL-PASS` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 8.2.6](./spec/av2/1.0.0/08-parsing-process.md#s-8-2-6) | Symbol decoding process | `DECODE-COEFF-FSC-SIGN-PASS` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 8.2.6](./spec/av2/1.0.0/08-parsing-process.md#s-8-2-6) | Symbol decoding process | `DECODE-COEFF-IDTX-CDF-ROWS` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 8.2.6](./spec/av2/1.0.0/08-parsing-process.md#s-8-2-6) | Symbol decoding process | `DECODE-COEFF-NONZERO-BLOCK-STATE` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 8.2.6](./spec/av2/1.0.0/08-parsing-process.md#s-8-2-6) | Symbol decoding process | `DECODE-COEFF-SIGN-SYMBOL-READ` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 8.2.6](./spec/av2/1.0.0/08-parsing-process.md#s-8-2-6) | Symbol decoding process | `DECODE-MINIMAL-BLOCK-SYNTAX-FRONTIER` | ✅ | 🟡 | — | ✅ | 4 |
| [§ 8.2.6](./spec/av2/1.0.0/08-parsing-process.md#s-8-2-6) | Symbol decoding process | `DECODE-MINIMAL-RAW-RUNTIME-OUTPUT` | ✅ | 🟡 | ✅ | ✅ | 4 |
| [§ 8.2.6](./spec/av2/1.0.0/08-parsing-process.md#s-8-2-6) | Symbol decoding process | `DECODE-MINIMAL-TIER-RUNTIME-SUCCESS` | ✅ | 🟡 | ✅ | ✅ | 3 |
| [§ 8.2.6](./spec/av2/1.0.0/08-parsing-process.md#s-8-2-6) | Symbol decoding process | `DECODE-TILE-CDF-SAVE-LIFECYCLE-BOUNDARY` | ✅ | ✅ | — | ✅ | 2 |
| [§ 8.2.6](./spec/av2/1.0.0/08-parsing-process.md#s-8-2-6) | Symbol decoding process | `DECODE-TILE-CDF-SELECTION-BOUNDARY` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 8.2.6](./spec/av2/1.0.0/08-parsing-process.md#s-8-2-6) | Symbol decoding process | `DECODE-TILE-PARTITION-DECISION-BOUNDARY` | ✅ | ✅ | — | ✅ | — |
| [§ 8.2.6](./spec/av2/1.0.0/08-parsing-process.md#s-8-2-6) | Symbol decoding process | `DECODE-TILE-PARTITION-SYMBOL-READ-BOUNDARY` | ✅ | ✅ | — | ✅ | — |
| [§ 8.2.6](./spec/av2/1.0.0/08-parsing-process.md#s-8-2-6) | Symbol decoding process | `DECODE-TILE-PARTITION-TRAVERSAL-BOUNDARY` | ✅ | ✅ | — | ✅ | — |
| [§ 8.2.6](./spec/av2/1.0.0/08-parsing-process.md#s-8-2-6) | Symbol decoding process | `DECODE-Y4M-RUNTIME-OUTPUT` | ✅ | 🟡 | ✅ | ✅ | 4 |
| [§ 8.2.6](./spec/av2/1.0.0/08-parsing-process.md#s-8-2-6) | Symbol decoding process | `ENC-BITSTREAM-WRITER` | 🟡 | — | — | 🟡 | — |
| [§ 8.3](./spec/av2/1.0.0/08-parsing-process.md#s-8-3) | Parsing process for CDF encoded syntax elements | `DECODE-CONTEXT-TILE-PAYLOAD-HANDOFF` | ✅ | — | — | ✅ | 2 |
| [§ 8.3](./spec/av2/1.0.0/08-parsing-process.md#s-8-3) | Parsing process for CDF encoded syntax elements | `DECODE-TILE-PAYLOAD-BOUNDARY` | ✅ | 🟡 | — | ✅ | 2 |
| [§ 8.3](./spec/av2/1.0.0/08-parsing-process.md#s-8-3) | Parsing process for CDF encoded syntax elements | `DECODE-TILE-PAYLOAD-INPUT-DERIVATION` | ✅ | 🟡 | — | ✅ | 2 |
| [§ 8.3.1](./spec/av2/1.0.0/08-parsing-process.md#s-8-3-1) | General | `CONF-TILE-PAYLOAD-DECODE-FUZZ` | ✅ | — | — | ✅ | 2 |
| [§ 8.3.1](./spec/av2/1.0.0/08-parsing-process.md#s-8-3-1) | General | `DECODE-MINIMAL-BLOCK-SYNTAX-FRONTIER` | ✅ | 🟡 | — | ✅ | 4 |
| [§ 8.3.1](./spec/av2/1.0.0/08-parsing-process.md#s-8-3-1) | General | `DECODE-TILE-CDF-SAVE-LIFECYCLE-BOUNDARY` | ✅ | ✅ | — | ✅ | 2 |
| [§ 8.3.1](./spec/av2/1.0.0/08-parsing-process.md#s-8-3-1) | General | `DECODE-TILE-CDF-SELECTION-BOUNDARY` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 8.3.1](./spec/av2/1.0.0/08-parsing-process.md#s-8-3-1) | General | `DECODE-TILE-PARTITION-DECISION-BOUNDARY` | ✅ | ✅ | — | ✅ | — |
| [§ 8.3.1](./spec/av2/1.0.0/08-parsing-process.md#s-8-3-1) | General | `DECODE-TILE-PARTITION-SYMBOL-READ-BOUNDARY` | ✅ | ✅ | — | ✅ | — |
| [§ 8.3.1](./spec/av2/1.0.0/08-parsing-process.md#s-8-3-1) | General | `DECODE-TILE-PARTITION-TRAVERSAL-BOUNDARY` | ✅ | ✅ | — | ✅ | — |
| [§ 8.3.2](./spec/av2/1.0.0/08-parsing-process.md#s-8-3-2) | Cdf selection process | `CONF-TILE-PAYLOAD-DECODE-FUZZ` | ✅ | — | — | ✅ | 2 |
| [§ 8.3.2](./spec/av2/1.0.0/08-parsing-process.md#s-8-3-2) | Cdf selection process | `DECODE-COEFF-ALL-ZERO-BLOCK-STATE` | ✅ | — | — | ✅ | 1 |
| [§ 8.3.2](./spec/av2/1.0.0/08-parsing-process.md#s-8-3-2) | Cdf selection process | `DECODE-COEFF-ALL-ZERO-CONTEXT-STATE` | ✅ | — | — | ✅ | 1 |
| [§ 8.3.2](./spec/av2/1.0.0/08-parsing-process.md#s-8-3-2) | Cdf selection process | `DECODE-COEFF-BASE-CDF-ROWS` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 8.3.2](./spec/av2/1.0.0/08-parsing-process.md#s-8-3-2) | Cdf selection process | `DECODE-COEFF-BASE-DERIVED-LEVEL-PASS` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 8.3.2](./spec/av2/1.0.0/08-parsing-process.md#s-8-3-2) | Cdf selection process | `DECODE-COEFF-BASE-PH-CDF-ROW` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 8.3.2](./spec/av2/1.0.0/08-parsing-process.md#s-8-3-2) | Cdf selection process | `DECODE-COEFF-BASE-SYMBOL-READ` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 8.3.2](./spec/av2/1.0.0/08-parsing-process.md#s-8-3-2) | Cdf selection process | `DECODE-COEFF-EOB-BRANCH-HANDOFF` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 8.3.2](./spec/av2/1.0.0/08-parsing-process.md#s-8-3-2) | Cdf selection process | `DECODE-COEFF-EOB-DERIVED-SYMBOL-READ` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 8.3.2](./spec/av2/1.0.0/08-parsing-process.md#s-8-3-2) | Cdf selection process | `DECODE-COEFF-EOB-SIZE-CONTEXT` | ✅ | — | — | ✅ | 1 |
| [§ 8.3.2](./spec/av2/1.0.0/08-parsing-process.md#s-8-3-2) | Cdf selection process | `DECODE-COEFF-EOB-SYMBOL-READ` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 8.3.2](./spec/av2/1.0.0/08-parsing-process.md#s-8-3-2) | Cdf selection process | `DECODE-COEFF-FSC-BRANCH-SCAN-ORDER` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 8.3.2](./spec/av2/1.0.0/08-parsing-process.md#s-8-3-2) | Cdf selection process | `DECODE-COEFF-FSC-BRANCH-TX-SIZE-HANDOFF` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 8.3.2](./spec/av2/1.0.0/08-parsing-process.md#s-8-3-2) | Cdf selection process | `DECODE-COEFF-FSC-LEVEL-PASS` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 8.3.2](./spec/av2/1.0.0/08-parsing-process.md#s-8-3-2) | Cdf selection process | `DECODE-COEFF-FSC-SIGN-PASS` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 8.3.2](./spec/av2/1.0.0/08-parsing-process.md#s-8-3-2) | Cdf selection process | `DECODE-COEFF-IDTX-CDF-ROWS` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 8.3.2](./spec/av2/1.0.0/08-parsing-process.md#s-8-3-2) | Cdf selection process | `DECODE-COEFF-NONZERO-BLOCK-STATE` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 8.3.2](./spec/av2/1.0.0/08-parsing-process.md#s-8-3-2) | Cdf selection process | `DECODE-COEFF-ORDINARY-BRANCH-ADJUSTED-TX-SIZE` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 8.3.2](./spec/av2/1.0.0/08-parsing-process.md#s-8-3-2) | Cdf selection process | `DECODE-COEFF-ORDINARY-BRANCH-HANDOFF` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 8.3.2](./spec/av2/1.0.0/08-parsing-process.md#s-8-3-2) | Cdf selection process | `DECODE-COEFF-ORDINARY-BRANCH-SCAN-ORDER` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 8.3.2](./spec/av2/1.0.0/08-parsing-process.md#s-8-3-2) | Cdf selection process | `DECODE-COEFF-ORDINARY-BRANCH-TX-CLASS-HANDOFF` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 8.3.2](./spec/av2/1.0.0/08-parsing-process.md#s-8-3-2) | Cdf selection process | `DECODE-COEFF-ORDINARY-DERIVED-BASE-PASS` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 8.3.2](./spec/av2/1.0.0/08-parsing-process.md#s-8-3-2) | Cdf selection process | `DECODE-COEFF-ORDINARY-DERIVED-SIGN-PASS` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 8.3.2](./spec/av2/1.0.0/08-parsing-process.md#s-8-3-2) | Cdf selection process | `DECODE-COEFF-PARITY-TCQ-HANDOFF` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 8.3.2](./spec/av2/1.0.0/08-parsing-process.md#s-8-3-2) | Cdf selection process | `DECODE-COEFF-RUNTIME-FRAME-ENTRY-HANDOFF` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 8.3.2](./spec/av2/1.0.0/08-parsing-process.md#s-8-3-2) | Cdf selection process | `DECODE-COEFF-RUNTIME-TX-SIZE-GEOMETRY-HANDOFF` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 8.3.2](./spec/av2/1.0.0/08-parsing-process.md#s-8-3-2) | Cdf selection process | `DECODE-COEFF-SIGN-SOURCE-DERIVE` | ✅ | — | — | ✅ | 1 |
| [§ 8.3.2](./spec/av2/1.0.0/08-parsing-process.md#s-8-3-2) | Cdf selection process | `DECODE-COEFF-SIGN-SYMBOL-READ` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 8.3.2](./spec/av2/1.0.0/08-parsing-process.md#s-8-3-2) | Cdf selection process | `DECODE-COEFF-STATE-CONTEXT-HANDOFF` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 8.3.2](./spec/av2/1.0.0/08-parsing-process.md#s-8-3-2) | Cdf selection process | `DECODE-COEFF-TX-CLASS-DERIVE` | ✅ | — | — | ✅ | 1 |
| [§ 8.3.2](./spec/av2/1.0.0/08-parsing-process.md#s-8-3-2) | Cdf selection process | `DECODE-MINIMAL-BLOCK-SYNTAX-FRONTIER` | ✅ | 🟡 | — | ✅ | 4 |
| [§ 8.3.2](./spec/av2/1.0.0/08-parsing-process.md#s-8-3-2) | Cdf selection process | `DECODE-MINIMAL-RAW-RUNTIME-OUTPUT` | ✅ | 🟡 | ✅ | ✅ | 4 |
| [§ 8.3.2](./spec/av2/1.0.0/08-parsing-process.md#s-8-3-2) | Cdf selection process | `DECODE-MINIMAL-TIER-RUNTIME-SUCCESS` | ✅ | 🟡 | ✅ | ✅ | 3 |
| [§ 8.3.2](./spec/av2/1.0.0/08-parsing-process.md#s-8-3-2) | Cdf selection process | `DECODE-TILE-CDF-SAVE-LIFECYCLE-BOUNDARY` | ✅ | ✅ | — | ✅ | 2 |
| [§ 8.3.2](./spec/av2/1.0.0/08-parsing-process.md#s-8-3-2) | Cdf selection process | `DECODE-TILE-CDF-SELECTION-BOUNDARY` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 8.3.2](./spec/av2/1.0.0/08-parsing-process.md#s-8-3-2) | Cdf selection process | `DECODE-TILE-COEFF-STATE-BUFFERS` | ✅ | — | — | ✅ | — |
| [§ 8.3.2](./spec/av2/1.0.0/08-parsing-process.md#s-8-3-2) | Cdf selection process | `DECODE-TILE-MI-SIZE-STATE-BOUNDARY` | ✅ | — | — | ✅ | — |
| [§ 8.3.2](./spec/av2/1.0.0/08-parsing-process.md#s-8-3-2) | Cdf selection process | `DECODE-TILE-PARTITION-DECISION-BOUNDARY` | ✅ | ✅ | — | ✅ | — |
| [§ 8.3.2](./spec/av2/1.0.0/08-parsing-process.md#s-8-3-2) | Cdf selection process | `DECODE-TILE-PARTITION-SYMBOL-READ-BOUNDARY` | ✅ | ✅ | — | ✅ | — |
| [§ 8.3.2](./spec/av2/1.0.0/08-parsing-process.md#s-8-3-2) | Cdf selection process | `DECODE-TILE-PARTITION-TRAVERSAL-BOUNDARY` | ✅ | ✅ | — | ✅ | — |
| [§ 8.3.2](./spec/av2/1.0.0/08-parsing-process.md#s-8-3-2) | Cdf selection process | `DECODE-Y4M-RUNTIME-OUTPUT` | ✅ | 🟡 | ✅ | ✅ | 4 |
| [§ 8.3.2](./spec/av2/1.0.0/08-parsing-process.md#s-8-3-2) | Cdf selection process | `ENC-COEFF-BASE-LF-CONTEXT` | ✅ | — | — | ✅ | — |
| [§ 8.3.2](./spec/av2/1.0.0/08-parsing-process.md#s-8-3-2) | Cdf selection process | `ENC-COEFF-BASE-LF-TOKEN` | ✅ | — | — | ✅ | — |
| [§ 8.3.2](./spec/av2/1.0.0/08-parsing-process.md#s-8-3-2) | Cdf selection process | `ENC-COEFF-MULTI-TOKENS` | ✅ | — | — | ✅ | — |
| [§ 8.3.2](./spec/av2/1.0.0/08-parsing-process.md#s-8-3-2) | Cdf selection process | `ENC-COEFFICIENT-TOKENIZATION-MINIMAL` | ✅ | — | — | ✅ | — |
| [§ 8.3.2](./spec/av2/1.0.0/08-parsing-process.md#s-8-3-2) | Cdf selection process | `ENC-INTRA-BLOCK-TRACE-CHROMA-SKIP` | ✅ | — | — | ✅ | — |
| [§ 8.3.2](./spec/av2/1.0.0/08-parsing-process.md#s-8-3-2) | Cdf selection process | `ENC-INTRA-BLOCK-TRACE-CODED-BR` | ✅ | — | — | ✅ | — |
| [§ 8.3.2](./spec/av2/1.0.0/08-parsing-process.md#s-8-3-2) | Cdf selection process | `ENC-INTRA-BLOCK-TRACE-CODED-CHROMA-DC` | ✅ | — | — | ✅ | — |
| [§ 8.3.2](./spec/av2/1.0.0/08-parsing-process.md#s-8-3-2) | Cdf selection process | `ENC-INTRA-BLOCK-TRACE-CODED-DC` | ✅ | — | — | ✅ | — |
| [§ 8.3.2](./spec/av2/1.0.0/08-parsing-process.md#s-8-3-2) | Cdf selection process | `ENC-INTRA-BLOCK-TRACE-TWO-COEFF` | ✅ | — | — | ✅ | — |
| [§ 8.3.2](./spec/av2/1.0.0/08-parsing-process.md#s-8-3-2) | Cdf selection process | `ENC-INTRA-BLOCK-TRACE-TWO-COEFF-TX-TYPE` | ✅ | — | — | ✅ | — |
| [§ 8.3.2](./spec/av2/1.0.0/08-parsing-process.md#s-8-3-2) | Cdf selection process | `ENC-INTRA-MODE-SYMBOL-EMISSION` | ✅ | — | — | ✅ | — |
| [§ 8.3.2](./spec/av2/1.0.0/08-parsing-process.md#s-8-3-2) | Cdf selection process | `ENC-INTRA-TX-TYPE-TOKEN` | ✅ | — | — | ✅ | — |
| [§ 8.3.2](./spec/av2/1.0.0/08-parsing-process.md#s-8-3-2) | Cdf selection process | `ENC-SEC-TX-TYPE-TOKEN` | ✅ | — | — | ✅ | — |
| [§ 8.3.2](./spec/av2/1.0.0/08-parsing-process.md#s-8-3-2) | Cdf selection process | `ENC-UV-MODE-SYMBOL-EMISSION` | ✅ | — | — | ✅ | — |
| [§ 8.3.2](./spec/av2/1.0.0/08-parsing-process.md#s-8-3-2) | Cdf selection process | `RECON-GET-TX-CLASS` | ✅ | — | ✅ | ✅ | — |

## Chapter 9 — Additional tables

| Section | Spec item | Feature | Mapped | Parse | Validate | Tests | Diagnostics |
|---|---|---|:-:|:-:|:-:|:-:|:-:|
| [§ 9](./spec/av2/1.0.0/09-additional-tables/09-00-overview.md#s-9) | Additional tables | `AV2-9-ADDITIONAL-TABLES` | ✅ | 🟡 | — | 🟡 | — |
| [§ 9](./spec/av2/1.0.0/09-additional-tables/09-00-overview.md#s-9) | Additional tables | `INFRA-SHARED-SPEC-TABLES` | ✅ | — | — | ✅ | — |
| [§ 9.2](./spec/av2/1.0.0/09-additional-tables/09-02-conversion-tables.md#s-9-2) | Conversion tables | `AV2-8.2-SYMBOL-DECODER` | ✅ | ✅ | ✅ | ✅ | — |
| [§ 9.2](./spec/av2/1.0.0/09-additional-tables/09-02-conversion-tables.md#s-9-2) | Conversion tables | `CONF-RECON-INTRA-PREDICTION-FUZZ` | ✅ | — | — | ✅ | — |
| [§ 9.2](./spec/av2/1.0.0/09-additional-tables/09-02-conversion-tables.md#s-9-2) | Conversion tables | `CONF-SYMBOL-DECODER-FUZZ` | ✅ | — | — | ✅ | — |
| [§ 9.2](./spec/av2/1.0.0/09-additional-tables/09-02-conversion-tables.md#s-9-2) | Conversion tables | `CONF-TILE-PAYLOAD-DECODE-FUZZ` | ✅ | — | — | ✅ | 2 |
| [§ 9.2](./spec/av2/1.0.0/09-additional-tables/09-02-conversion-tables.md#s-9-2) | Conversion tables | `DECODE-COEFF-EOB-BRANCH-HANDOFF` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 9.2](./spec/av2/1.0.0/09-additional-tables/09-02-conversion-tables.md#s-9-2) | Conversion tables | `DECODE-COEFF-EOB-DERIVED-SYMBOL-READ` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 9.2](./spec/av2/1.0.0/09-additional-tables/09-02-conversion-tables.md#s-9-2) | Conversion tables | `DECODE-COEFF-EOB-SIZE-CONTEXT` | ✅ | — | — | ✅ | 1 |
| [§ 9.2](./spec/av2/1.0.0/09-additional-tables/09-02-conversion-tables.md#s-9-2) | Conversion tables | `DECODE-COEFF-FSC-BRANCH-SCAN-ORDER` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 9.2](./spec/av2/1.0.0/09-additional-tables/09-02-conversion-tables.md#s-9-2) | Conversion tables | `DECODE-COEFF-FSC-BRANCH-TX-SIZE-HANDOFF` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 9.2](./spec/av2/1.0.0/09-additional-tables/09-02-conversion-tables.md#s-9-2) | Conversion tables | `DECODE-COEFF-NONZERO-BLOCK-STATE` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 9.2](./spec/av2/1.0.0/09-additional-tables/09-02-conversion-tables.md#s-9-2) | Conversion tables | `DECODE-COEFF-ORDINARY-BRANCH-ADJUSTED-TX-SIZE` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 9.2](./spec/av2/1.0.0/09-additional-tables/09-02-conversion-tables.md#s-9-2) | Conversion tables | `DECODE-COEFF-ORDINARY-BRANCH-DIRECTIONAL-UV-HANDOFF` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 9.2](./spec/av2/1.0.0/09-additional-tables/09-02-conversion-tables.md#s-9-2) | Conversion tables | `DECODE-COEFF-ORDINARY-BRANCH-LOSSLESS-HANDOFF` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 9.2](./spec/av2/1.0.0/09-additional-tables/09-02-conversion-tables.md#s-9-2) | Conversion tables | `DECODE-COEFF-ORDINARY-BRANCH-MODE-TO-TXFM-HANDOFF` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 9.2](./spec/av2/1.0.0/09-additional-tables/09-02-conversion-tables.md#s-9-2) | Conversion tables | `DECODE-COEFF-ORDINARY-BRANCH-SCAN-ORDER` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 9.2](./spec/av2/1.0.0/09-additional-tables/09-02-conversion-tables.md#s-9-2) | Conversion tables | `DECODE-COEFF-ORDINARY-BRANCH-TX-SET-HANDOFF` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 9.2](./spec/av2/1.0.0/09-additional-tables/09-02-conversion-tables.md#s-9-2) | Conversion tables | `DECODE-COEFF-ORDINARY-BRANCH-TX-SIZE-CONTEXT` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 9.2](./spec/av2/1.0.0/09-additional-tables/09-02-conversion-tables.md#s-9-2) | Conversion tables | `DECODE-COEFF-ORDINARY-BRANCH-TX-SIZE-DIMENSIONS` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 9.2](./spec/av2/1.0.0/09-additional-tables/09-02-conversion-tables.md#s-9-2) | Conversion tables | `DECODE-COEFF-RUNTIME-FRAME-ENTRY-HANDOFF` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 9.2](./spec/av2/1.0.0/09-additional-tables/09-02-conversion-tables.md#s-9-2) | Conversion tables | `DECODE-COEFF-RUNTIME-TX-SIZE-GEOMETRY-HANDOFF` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 9.2](./spec/av2/1.0.0/09-additional-tables/09-02-conversion-tables.md#s-9-2) | Conversion tables | `DECODE-COEFF-USE-FSC-SHARED-FACTS-HANDOFF` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 9.2](./spec/av2/1.0.0/09-additional-tables/09-02-conversion-tables.md#s-9-2) | Conversion tables | `DECODE-MINIMAL-INTRA-RECONSTRUCTION-FRONTIER` | ✅ | 🟡 | ✅ | ✅ | 4 |
| [§ 9.2](./spec/av2/1.0.0/09-additional-tables/09-02-conversion-tables.md#s-9-2) | Conversion tables | `DECODE-MINIMAL-RAW-RUNTIME-OUTPUT` | ✅ | 🟡 | ✅ | ✅ | 4 |
| [§ 9.2](./spec/av2/1.0.0/09-additional-tables/09-02-conversion-tables.md#s-9-2) | Conversion tables | `DECODE-MINIMAL-TIER-RUNTIME-SUCCESS` | ✅ | 🟡 | ✅ | ✅ | 3 |
| [§ 9.2](./spec/av2/1.0.0/09-additional-tables/09-02-conversion-tables.md#s-9-2) | Conversion tables | `DECODE-MODE-TO-TXFM-SYMBOLIC-TABLE` | ✅ | — | — | ✅ | 1 |
| [§ 9.2](./spec/av2/1.0.0/09-additional-tables/09-02-conversion-tables.md#s-9-2) | Conversion tables | `DECODE-TILE-CDF-SELECTION-BOUNDARY` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 9.2](./spec/av2/1.0.0/09-additional-tables/09-02-conversion-tables.md#s-9-2) | Conversion tables | `DECODE-TILE-PARTITION-ALLOWED-BOUNDARY` | ✅ | — | — | ✅ | — |
| [§ 9.2](./spec/av2/1.0.0/09-additional-tables/09-02-conversion-tables.md#s-9-2) | Conversion tables | `DECODE-TILE-PARTITION-SIZE-TABLE-BOUNDARY` | ✅ | — | — | ✅ | — |
| [§ 9.2](./spec/av2/1.0.0/09-additional-tables/09-02-conversion-tables.md#s-9-2) | Conversion tables | `DECODE-TILE-PARTITION-TRAVERSAL-BOUNDARY` | ✅ | ✅ | — | ✅ | — |
| [§ 9.2](./spec/av2/1.0.0/09-additional-tables/09-02-conversion-tables.md#s-9-2) | Conversion tables | `DECODE-TX-SIZE-SYMBOLIC-TABLES` | ✅ | — | — | ✅ | 1 |
| [§ 9.2](./spec/av2/1.0.0/09-additional-tables/09-02-conversion-tables.md#s-9-2) | Conversion tables | `DECODE-Y4M-RUNTIME-OUTPUT` | ✅ | 🟡 | ✅ | ✅ | 4 |
| [§ 9.2](./spec/av2/1.0.0/09-additional-tables/09-02-conversion-tables.md#s-9-2) | Conversion tables | `ENC-BITSTREAM-WRITER` | 🟡 | — | — | 🟡 | — |
| [§ 9.2](./spec/av2/1.0.0/09-additional-tables/09-02-conversion-tables.md#s-9-2) | Conversion tables | `ENC-INTRA-TX-TYPE-TOKEN` | ✅ | — | — | ✅ | — |
| [§ 9.2](./spec/av2/1.0.0/09-additional-tables/09-02-conversion-tables.md#s-9-2) | Conversion tables | `RECON-INTRA-CARDINAL-DIRECTIONAL-PREDICTION` | ✅ | — | ✅ | ✅ | — |
| [§ 9.2](./spec/av2/1.0.0/09-additional-tables/09-02-conversion-tables.md#s-9-2) | Conversion tables | `RECON-INTRA-MIDDLE-DIRECTIONAL-ANGLE-PREDICTION` | ✅ | — | ✅ | ✅ | — |
| [§ 9.2](./spec/av2/1.0.0/09-additional-tables/09-02-conversion-tables.md#s-9-2) | Conversion tables | `RECON-INTRA-ONE-SIDED-DIRECTIONAL-ANGLE-PREDICTION` | ✅ | — | ✅ | ✅ | — |
| [§ 9.2](./spec/av2/1.0.0/09-additional-tables/09-02-conversion-tables.md#s-9-2) | Conversion tables | `RECON-TRANSFORM-SHIFT-LOOKUP` | ✅ | — | ✅ | ✅ | — |
| [§ 9.2](./spec/av2/1.0.0/09-additional-tables/09-02-conversion-tables.md#s-9-2) | Conversion tables | `RECON-WORKSPACE-DIRECTIONAL-ANGLE-PREDICTION` | ✅ | — | ✅ | ✅ | — |
| [§ 9.3](./spec/av2/1.0.0/09-additional-tables/09-03-default-cdf-tables.md#s-9-3) | Default CDF tables | `CONF-TILE-PAYLOAD-DECODE-FUZZ` | ✅ | — | — | ✅ | 2 |
| [§ 9.3](./spec/av2/1.0.0/09-additional-tables/09-03-default-cdf-tables.md#s-9-3) | Default CDF tables | `DECODE-COEFF-BASE-CDF-ROWS` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 9.3](./spec/av2/1.0.0/09-additional-tables/09-03-default-cdf-tables.md#s-9-3) | Default CDF tables | `DECODE-COEFF-BASE-DERIVED-LEVEL-PASS` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 9.3](./spec/av2/1.0.0/09-additional-tables/09-03-default-cdf-tables.md#s-9-3) | Default CDF tables | `DECODE-COEFF-BASE-PH-CDF-ROW` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 9.3](./spec/av2/1.0.0/09-additional-tables/09-03-default-cdf-tables.md#s-9-3) | Default CDF tables | `DECODE-COEFF-BASE-SYMBOL-READ` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 9.3](./spec/av2/1.0.0/09-additional-tables/09-03-default-cdf-tables.md#s-9-3) | Default CDF tables | `DECODE-COEFF-EOB-BRANCH-HANDOFF` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 9.3](./spec/av2/1.0.0/09-additional-tables/09-03-default-cdf-tables.md#s-9-3) | Default CDF tables | `DECODE-COEFF-EOB-DERIVED-SYMBOL-READ` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 9.3](./spec/av2/1.0.0/09-additional-tables/09-03-default-cdf-tables.md#s-9-3) | Default CDF tables | `DECODE-COEFF-EOB-SYMBOL-READ` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 9.3](./spec/av2/1.0.0/09-additional-tables/09-03-default-cdf-tables.md#s-9-3) | Default CDF tables | `DECODE-COEFF-FSC-LEVEL-PASS` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 9.3](./spec/av2/1.0.0/09-additional-tables/09-03-default-cdf-tables.md#s-9-3) | Default CDF tables | `DECODE-COEFF-FSC-SIGN-PASS` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 9.3](./spec/av2/1.0.0/09-additional-tables/09-03-default-cdf-tables.md#s-9-3) | Default CDF tables | `DECODE-COEFF-IDTX-CDF-ROWS` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 9.3](./spec/av2/1.0.0/09-additional-tables/09-03-default-cdf-tables.md#s-9-3) | Default CDF tables | `DECODE-COEFF-NONZERO-BLOCK-STATE` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 9.3](./spec/av2/1.0.0/09-additional-tables/09-03-default-cdf-tables.md#s-9-3) | Default CDF tables | `DECODE-COEFF-SIGN-SYMBOL-READ` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 9.3](./spec/av2/1.0.0/09-additional-tables/09-03-default-cdf-tables.md#s-9-3) | Default CDF tables | `DECODE-MINIMAL-BLOCK-SYNTAX-FRONTIER` | ✅ | 🟡 | — | ✅ | 4 |
| [§ 9.3](./spec/av2/1.0.0/09-additional-tables/09-03-default-cdf-tables.md#s-9-3) | Default CDF tables | `DECODE-TILE-CDF-SAVE-LIFECYCLE-BOUNDARY` | ✅ | ✅ | — | ✅ | 2 |
| [§ 9.3](./spec/av2/1.0.0/09-additional-tables/09-03-default-cdf-tables.md#s-9-3) | Default CDF tables | `DECODE-TILE-CDF-SELECTION-BOUNDARY` | ✅ | 🟡 | — | ✅ | 1 |
| [§ 9.4](./spec/av2/1.0.0/09-additional-tables/09-04-quantizer-matrix-tables.md#s-9-4) | Quantizer matrix tables | `RECON-DEQUANT-QM-WEIGHT` | ✅ | — | ✅ | ✅ | — |
| [§ 9.6](./spec/av2/1.0.0/09-additional-tables/09-06-1d-transform-tables.md#s-9-6) | 1d transform tables | `RECON-INVERSE-TRANSFORM-1D` | ✅ | — | ✅ | ✅ | — |

## Annexes

| Section | Spec item | Feature | Mapped | Parse | Validate | Tests | Diagnostics |
|---|---|---|:-:|:-:|:-:|:-:|:-:|
| [Annex A](./spec/av2/1.0.0/annex-a-profiles-levels-and-tiers.md#s-annex-a) | Profiles, levels, and tiers | `AV2-A-LEVELS-TIERS` | ✅ | — | 🟡 | ✅ | 5 |
| [Annex A](./spec/av2/1.0.0/annex-a-profiles-levels-and-tiers.md#s-annex-a) | Profiles, levels, and tiers | `AV2-A-PROFILES` | ✅ | — | 🟡 | ✅ | 7 |
| [Annex B](./spec/av2/1.0.0/annex-b-length-delimited-bitstream-format.md#s-annex-b) | Length delimited bitstream format | `AV2-B-ANNEXB-OBU-ENVELOPE` | ✅ | ✅ | ✅ | ✅ | 1 |
| [Annex B.2](./spec/av2/1.0.0/annex-b-length-delimited-bitstream-format.md#s-annex-b-2) | Length delimited bitstream syntax | `CONF-DECODE-RUNTIME-HASH-FUZZ` | ✅ | — | — | ✅ | 3 |
| [Annex B.2](./spec/av2/1.0.0/annex-b-length-delimited-bitstream-format.md#s-annex-b-2) | Length delimited bitstream syntax | `CONF-DECODE-RUNTIME-RAW-FUZZ` | ✅ | — | — | ✅ | 4 |
| [Annex B.2](./spec/av2/1.0.0/annex-b-length-delimited-bitstream-format.md#s-annex-b-2) | Length delimited bitstream syntax | `CONF-DECODE-RUNTIME-Y4M-FUZZ` | ✅ | — | — | ✅ | 4 |
| [Annex B.2](./spec/av2/1.0.0/annex-b-length-delimited-bitstream-format.md#s-annex-b-2) | Length delimited bitstream syntax | `DECODE-BYTE-STREAM-PLANNER` | ✅ | ✅ | — | ✅ | 3 |
| [Annex B.2](./spec/av2/1.0.0/annex-b-length-delimited-bitstream-format.md#s-annex-b-2) | Length delimited bitstream syntax | `DECODE-LIMITS-RUNTIME-API` | ✅ | — | — | ✅ | — |
| [Annex B.2](./spec/av2/1.0.0/annex-b-length-delimited-bitstream-format.md#s-annex-b-2) | Length delimited bitstream syntax | `DECODE-MINIMAL-RAW-RUNTIME-OUTPUT` | ✅ | 🟡 | ✅ | ✅ | 4 |
| [Annex B.2](./spec/av2/1.0.0/annex-b-length-delimited-bitstream-format.md#s-annex-b-2) | Length delimited bitstream syntax | `DECODE-MINIMAL-TIER-RUNTIME-SUCCESS` | ✅ | 🟡 | ✅ | ✅ | 3 |
| [Annex B.2](./spec/av2/1.0.0/annex-b-length-delimited-bitstream-format.md#s-annex-b-2) | Length delimited bitstream syntax | `DECODE-Y4M-RUNTIME-OUTPUT` | ✅ | 🟡 | ✅ | ✅ | 4 |
| [Annex B.3](./spec/av2/1.0.0/annex-b-length-delimited-bitstream-format.md#s-annex-b-3) | Length delimited bitstream semantics | `CONF-DECODE-RUNTIME-HASH-FUZZ` | ✅ | — | — | ✅ | 3 |
| [Annex B.3](./spec/av2/1.0.0/annex-b-length-delimited-bitstream-format.md#s-annex-b-3) | Length delimited bitstream semantics | `CONF-DECODE-RUNTIME-RAW-FUZZ` | ✅ | — | — | ✅ | 4 |
| [Annex B.3](./spec/av2/1.0.0/annex-b-length-delimited-bitstream-format.md#s-annex-b-3) | Length delimited bitstream semantics | `CONF-DECODE-RUNTIME-Y4M-FUZZ` | ✅ | — | — | ✅ | 4 |
| [Annex B.3](./spec/av2/1.0.0/annex-b-length-delimited-bitstream-format.md#s-annex-b-3) | Length delimited bitstream semantics | `DECODE-BYTE-STREAM-PLANNER` | ✅ | ✅ | — | ✅ | 3 |
| [Annex B.3](./spec/av2/1.0.0/annex-b-length-delimited-bitstream-format.md#s-annex-b-3) | Length delimited bitstream semantics | `DECODE-LIMITS-RUNTIME-API` | ✅ | — | — | ✅ | — |
| [Annex B.3](./spec/av2/1.0.0/annex-b-length-delimited-bitstream-format.md#s-annex-b-3) | Length delimited bitstream semantics | `DECODE-MINIMAL-RAW-RUNTIME-OUTPUT` | ✅ | 🟡 | ✅ | ✅ | 4 |
| [Annex B.3](./spec/av2/1.0.0/annex-b-length-delimited-bitstream-format.md#s-annex-b-3) | Length delimited bitstream semantics | `DECODE-MINIMAL-TIER-RUNTIME-SUCCESS` | ✅ | 🟡 | ✅ | ✅ | 3 |
| [Annex B.3](./spec/av2/1.0.0/annex-b-length-delimited-bitstream-format.md#s-annex-b-3) | Length delimited bitstream semantics | `DECODE-Y4M-RUNTIME-OUTPUT` | ✅ | 🟡 | ✅ | ✅ | 4 |
| [Annex E](./spec/av2/1.0.0/annex-e-decoder-model.md#s-annex-e) | Decoder model | `AV2-E-DECODER-MODEL` | ✅ | — |  |  | — |

## Features without a spec section

53 feature(s) track conformance, encoder, CLI, automation, or documentation work with no single spec section; see [FEATURE-STATUS.md](./FEATURE-STATUS.md):

- `AV2-IVF-CONTAINER` — IVF container support
- `CLI-INSPECT` — splot inspect command
- `CLI-VALIDATE` — splot validate command
- `CLI-VALIDATE-EXPLAIN` — splot explain command + diagnostic registry
- `CLI-VALIDATE-OUTPUT-CONTROLS` — splot validate output controls (--max-diagnostics / --summary-only)
- `CONF-AVM-DIFF-HARNESS` — AVM differential testing harness
- `CONF-AVM-INVALID-STREAMS` — AVM invalid stream conformance
- `CONF-AVM-PARSER-TRACES` — AVM parser trace comparison
- `CONF-AVM-VALID-STREAMS` — AVM valid stream conformance
- `CONF-CLI-SNAPSHOT-COVERAGE` — CLI help + inspect text snapshots
- `CONF-FUZZ-NO-PANIC` — Parser no-panic fuzzing
- `CONF-INSPECT-SNAPSHOTS` — Inspector snapshot tests
- `CONF-PUBLIC-VECTOR-LICENSE-REVIEW` — Public vector license review
- `CONF-PUBLIC-VECTORS` — Public AV2 vector corpus integration
- `DOC-AUDIT-PROTOCOLS` — Agent audit protocol skills
- `DOC-AV2-SPEC-MIRROR` — AV2 specification mirror
- `DOC-DECODE-LIMITS-CONTRACT` — Decode limits contract documentation
- `DOC-DECODED-FRAME-PLANE-MODEL-CONTRACT` — Decoded frame and plane model contract documentation
- `DOC-DECODER-DIAGNOSTICS` — Decoder diagnostics registry documentation
- `DOC-DECODER-FULL-CONFORMANCE-CONTRACT` — Full decoder conformance contract
- `DOC-DECODER-ROADMAP` — Decoder roadmap documentation
- `DOC-DECODER-SUPPORT-MATRIX` — Decoder support matrix documentation
- `DOC-DETERMINISTIC-FRAME-HASH-CONTRACT` — Deterministic decoded-frame hash contract documentation
- `DOC-ENCODER-PROGRAM-CONTRACT` — Encoder program contract documentation
- `DOC-ENCODER-REFERENCE-GATE` — Encoder reference gate documentation
- `DOC-FEATURE-TRACKING` — Feature tracking documentation
- `DOC-MINIMAL-DECODE-TIER-CONTRACT` — Minimal decode tier contract documentation
- `DOC-VALIDATOR-EXAMPLES` — Validator CLI worked examples (README)
- `DOC-VALIDATOR-ROADMAP` — Validator coverage roadmap documentation
- `ENC-CONTEXT-STATE-MACHINE` — Encoder context state machine
- `ENC-INTRA-TOY-V0` — Minimal toy intra encoder path
- `ENC-RATE-CONTROL-V0` — Initial rate control strategy
- `ENC-RECON-DEPENDENCY` — Encoder reconstruction dependency boundary
- `ENC-SPEED-PRESETS` — Encoder speed preset framework
- `ENC-SYNTAX-IR` — Encoder syntax planning IR
- `ENC-Y4M-INPUT` — Y4M input reader integration
- `INFRA-DECODER-CRATE-SCAFFOLDING` — Decoder and reconstruction crate scaffolding
- `INFRA-PARALLEL-RUNTIME-POLICY` — Parallel runtime policy (Rayon worker pool + bounded crossbeam queues)
- `INFRA-ZERO-COPY-MEDIA-POLICY` — Zero-copy media-buffer ownership policy
- `VALIDATOR-CONTEXT-SPLIT` — Validator context module split
- `XTASK-AUDIT-SCOPE` — Changed-file AV2 audit scope
- `XTASK-CHECK-FIXTURES` — Test-fixture manifest gate
- `XTASK-CI-QUALITY-GATES` — CI quality gates (docs build + coverage threshold)
- `XTASK-CONVENTIONAL-COMMITS` — Conventional commit enforcement
- `XTASK-DECODER-CONFORMANCE-COVERAGE` — Decoder conformance coverage gate
- `XTASK-DECODER-DIAGNOSTIC-REGISTRY` — Decoder diagnostic registry enforcement
- `XTASK-DECODER-SUPPORT-STATUS` — Decoder support status reporting and checks
- `XTASK-DIAGNOSTIC-REGISTRY` — Validator diagnostic registry enforcement
- `XTASK-FEATURE-STATUS` — xtask feature status reporting and checks
- `XTASK-GEN-TABLES` — AV2 § 9 tables code generator
- `XTASK-LOCAL-REFERENCE-EVIDENCE-MANIFEST` — Portable local-reference evidence manifest
- `XTASK-SOURCE-LINES` — Rust source-file line budget
- `XTASK-VALIDATOR-MODULE-SPLIT` — Validator module split
