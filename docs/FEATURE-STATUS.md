# Feature status

Generated from `docs/IMPLEMENTATION-MATRIX.toml` by `cargo xtask feature-status --format markdown`. Do not edit by hand.

Matrix version 1. Last reviewed 2026-06-06. 27 feature(s).

Status legend: `done` complete and proven, `partial` in progress, `todo` not started, `pending` waiting on external proof, `blocked` blocked, `exp` experimental, `n/a` not-applicable.

| ID | Name | Category | Kind | Mapped | Types | Parse | Validate | Write | Encode | Tests | AVM | Module |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| `AV2-4.11.6-LEB128` | LEB128 descriptor | normative | bitstream-syntax | done | done | done | partial | todo | n/a | done | pending | `crates/splot-core/src/leb128.rs` |
| `AV2-5.2.2-OBU-HEADER` | OBU header syntax | normative | bitstream-syntax | done | done | done | done | todo | n/a | done | pending | `crates/splot-core/src/obu.rs` |
| `AV2-5.2.1-OBU-TYPE` | OBU type and OBU-class predicates | normative | bitstream-syntax | done | done | done | partial | n/a | n/a | done | pending | `crates/splot-core/src/types.rs` |
| `AV2-B-ANNEXB-OBU-ENVELOPE` | Annex B length-delimited OBU envelope | normative | bitstream-syntax | done | done | done | done | todo | n/a | done | pending | `crates/splot-core/src/annexb.rs` |
| `AV2-5.2.3-TRAILING-BITS` | Trailing bits syntax and semantics | normative | bitstream-syntax | done | todo | todo | todo | todo | n/a | todo | pending | `crates/splot-core/src/obu.rs` |
| `AV2-5.2.4-BYTE-ALIGNMENT` | Byte alignment syntax and semantics | normative | bitstream-syntax | done | todo | todo | todo | todo | n/a | todo | pending | `crates/splot-core/src/bitio.rs` |
| `AV2-5.3-RESERVED-OBU` | Reserved OBU handling | normative | validator-check | done | done | n/a | done | n/a | n/a | done | pending | `crates/splot-validate/src/checks/mod.rs` |
| `AV2-5.4-SEQUENCE-HEADER` | Sequence header OBU syntax | normative | bitstream-syntax | done | todo | todo | todo | todo | todo | todo | pending | `crates/splot-core/src/headers.rs` |
| `AV2-5.8-LAYER-CONFIG-RECORD` | Layer configuration record OBU syntax | normative | bitstream-syntax | done | todo | todo | todo | todo | todo | todo | pending | `crates/splot-core/src/headers.rs` |
| `AV2-5.10-OPERATING-POINT-SET` | Operating point set OBU syntax | normative | bitstream-syntax | done | todo | todo | todo | todo | todo | todo | pending | `crates/splot-core/src/headers.rs` |
| `AV2-5.18-FRAME-HEADER` | Frame header syntax | normative | bitstream-syntax | done | todo | todo | todo | todo | todo | todo | pending | `crates/splot-core/src/headers.rs` |
| `AV2-5.19-TILE-GROUP` | Tile group OBU syntax | normative | bitstream-syntax | done | todo | todo | todo | todo | todo | todo | pending | `crates/splot-core/src/headers.rs` |
| `AV2-7.3-OBU-ORDERING` | Ordering of OBUs | normative | bitstream-semantics | done | todo | n/a | todo | n/a | n/a | todo | pending | `crates/splot-validate/src/checks/mod.rs` |
| `AV2-9-ADDITIONAL-TABLES` | Additional spec tables (codegen) | normative | bitstream-semantics | done | todo | todo | n/a | n/a | n/a | todo | pending | `crates/splot-core/src/tables.rs` |
| `ENC-BITSTREAM-WRITER` | Bitstream writer foundation | encoder | writer | partial | todo | n/a | n/a | todo | todo | todo | pending | `crates/splot-core/src/bitio.rs` |
| `ENC-Y4M-INPUT` | Y4M input reader integration | encoder | encoder-api | partial | todo | n/a | n/a | n/a | todo | todo | n/a | `crates/splot-encode/src/context.rs` |
| `ENC-INTRA-TOY-V0` | Minimal toy intra encoder path | encoder | encoder-tool | partial | todo | n/a | n/a | todo | todo | todo | pending | `crates/splot-encode/src/context.rs` |
| `ENC-RATE-CONTROL-V0` | Initial rate control strategy | encoder | encoder-tool | partial | todo | n/a | n/a | n/a | todo | todo | n/a | `crates/splot-encode/src/context.rs` |
| `ENC-SPEED-PRESETS` | Encoder speed preset framework | encoder | encoder-api | partial | todo | n/a | n/a | n/a | todo | todo | n/a | `crates/splot-encode/src/config.rs` |
| `CONF-AVM-DIFF-HARNESS` | AVM differential testing harness | conformance | conformance | partial | todo | n/a | n/a | n/a | n/a | todo | todo | `xtask/src/main.rs` |
| `CONF-PUBLIC-VECTORS` | Public AV2 vector corpus integration | conformance | conformance | partial | todo | n/a | n/a | n/a | n/a | todo | pending | `xtask/src/main.rs` |
| `CONF-INSPECT-SNAPSHOTS` | Inspector snapshot tests | conformance | conformance | partial | n/a | n/a | n/a | n/a | n/a | todo | n/a | `crates/splot-cli/src/commands/inspect.rs` |
| `CONF-FUZZ-NO-PANIC` | Parser no-panic fuzzing | conformance | conformance | done | n/a | partial | partial | n/a | n/a | done | n/a | `fuzz/fuzz_targets/parse_obu.rs` |
| `CLI-VALIDATE` | splot validate command | cli | cli | done | n/a | n/a | n/a | n/a | n/a | done | n/a | `crates/splot-cli/src/commands/validate.rs` |
| `CLI-INSPECT` | splot inspect command | cli | cli | done | n/a | n/a | n/a | n/a | n/a | done | n/a | `crates/splot-cli/src/commands/inspect.rs` |
| `XTASK-FEATURE-STATUS` | xtask feature status reporting and checks | automation | automation | done | done | n/a | done | n/a | n/a | done | n/a | `xtask/src/feature_status.rs` |
| `DOC-FEATURE-TRACKING` | Feature tracking documentation | docs | docs | done | n/a | n/a | n/a | n/a | n/a | n/a | n/a | `docs/FEATURE-TRACKING.md` |
