# Validator gap analysis

`status: draft`  
`owner: validator`  
`primary matrix: docs/IMPLEMENTATION-MATRIX.toml`  
`primary spec: AV2 Bitstream & Decoding Process Specification v1.0.0`

> **⚠️ Stale baseline snapshot (flagged by the 2026-06-09 doc audit).** The
> "Current status" / "Feature status today" columns below describe the
> *pre-Phase-1* baseline and are now out of date — for example §5.2.3
> `trailing_bits` and §5.2.4 `byte_alignment` are implemented (`parse`/`tests`
> `done`), and the §5.4 sequence header now parses (its child configs are largely
> `parse`/`tests` `done`; the `AV2-5.4-SEQUENCE-HEADER` umbrella stays `partial`
> pending full child and validation coverage) — none are the "todo/partial" shown
> below. Treat
> [`docs/IMPLEMENTATION-MATRIX.toml`](./IMPLEMENTATION-MATRIX.toml) (and the
> generated [`docs/FEATURE-STATUS.md`](./FEATURE-STATUS.md)) as the canonical
> current status; this document is retained for its gap framing and validator
> *targets*, not as a status report.

## 1. Current validator baseline

The repository is in the intended validator-first shape. The current implemented surface is the framing layer:

| Area | Current status | Practical meaning |
|---|---:|---|
| AV2 Annex B length-delimited bitstream envelope | implemented | The validator can walk `leb128 num_bytes_in_obu + OBU bytes` records and preserve diagnostics from parseable prefixes. |
| AV2 §4.11.6 `leb128()` | implemented | 8-byte maximum, `u32` bound, byte-7-MSB rule, EOF handling. |
| AV2 §5.2.1 OBU type helpers | implemented | OBU type enum, tile-group predicate, SEF/TIP predicates, extensible-OBU predicate, global-xlayer helpers. |
| AV2 §5.2.2 `obu_header()` | implemented | AV2 header only, not AV1 header. |
| AV2 §6.2.2 header-only checks | implemented/partial | Header-only rules are checked; rules requiring an activated sequence header are still missing. |
| AV2 §5.3 / §6.2.3 reserved OBU payload rule | implemented | Reserved OBU payloads that are entirely zero are rejected. |
| `validate` and `inspect` CLI plumbing | implemented | CLI is useful for envelope/header inspection, but not payload validation. |
| Feature tracking/OpenSpec | implemented | Matrix and OpenSpec workflow exist and should be used before adding parser/checks. |

This means `splot validate` validates **framing and OBU headers**, not OBU payload contents. A syntactically legal envelope containing an illegal sequence header or illegal frame header can still pass today because those payloads are not parsed.

## 2. Highest-impact missing validator requirements

The missing validator work falls into seven dependency layers. Do not jump directly to frame/tile payloads before the earlier layers exist.

| Layer | Missing area | Why it blocks later validator work |
|---|---|---|
| 1 | Generic bit descriptors beyond fixed-width `f(n)` | `sequence_header_obu()` needs `uvlc()`, `ns(n)`, alignment, bounded bit parsing, and exact EOF diagnostics. |
| 2 | `open_bitstream_unit(sz)` payload dispatch | Current code parses envelope + header, but not the syntax selected by `obu_type`. Payload dispatch is the gateway to all §5 syntax. |
| 3 | `trailing_bits()`, `byte_alignment()`, extensible OBU payload handling | The validator cannot know whether parsed payload bits end correctly until it models trailing bits and extension payloads. |
| 4 | `sequence_header_obu()` syntax and §6.4 semantics | This is the first real payload parser. It unlocks activated-sequence state and the remaining §6.2.2 layer-range checks. |
| 5 | Stateful validation context | OBU ordering, activated sequence headers, HLS availability, layer maps, and frame checks all require state across OBUs. |
| 6 | High-level syntax OBUs | MSDO, LCR, OPS, atlas, metadata, film grain, quantization matrix, content interpretation, and buffer removal timing must be parsed before full ordering/profile checks. |
| 7 | Frame header and tile group syntax | The largest surface. Split it into child features; never mark the umbrella done until child rows are proven. |

## 3. Missing syntax coverage by AV2 §5

Use this as the human-readable roadmap. The canonical status still lives in `docs/IMPLEMENTATION-MATRIX.toml`.

| Spec area | Feature status today | Validator target |
|---|---|---|
| §5.2.1 `open_bitstream_unit(sz)` | partial: helper predicates only | Implement payload dispatch into a typed `ParsedObu` tree. |
| §5.2.2 `obu_header()` | done | Keep as foundation; add activated sequence header checks once sequence state exists. |
| §5.2.3 `trailing_bits(nbBits)` | todo/partial | Parse and validate `trailing_one_bit == 1`, zero padding, and no payload overread. |
| §5.2.4 `byte_alignment()` | todo/partial | Validate zero alignment bits where syntax requires them. |
| §5.3 `reserved_obu()` | partial/done | Keep all-zero-payload rule; do not interpret non-zero reserved payload bytes beyond retaining them for inspection. |
| §5.4 `sequence_header_obu()` | todo | First payload parser. Split into child rows for §5.4.1-§5.4.13. |
| §5.5 temporal delimiter | todo | Empty/no-payload syntax plus ordering/state effect: `FirstPictureInTU = 1`. |
| §5.6 MSDO | todo | Required for multistream boundaries, xlayer map validation, and profile checks. |
| §5.7 multi-frame header | todo | Needed before frame header reuse and frame-unit validation. |
| §5.8 layer configuration record | todo | Required for global/local layer configuration, LCR availability, and layer constraints. |
| §5.9 atlas segment info | todo | Required for atlas/LCR relationships and atlas-region constraints. |
| §5.10/§5.11 operating point set/payload | todo | Required for operating-point validation and sub-bitstream conformance. |
| §5.12 buffer removal timing | todo | Needed for decoder-model timing checks. |
| §5.13 quantizer matrix | todo | Needed for sequence/frame QM references and non-zero table checks. |
| §5.14 film grain | todo | Needed for film-grain syntax and update flag checks. |
| §5.15 content interpretation | todo | Needed for content interpretation metadata semantics. |
| §5.16 padding | todo | Needed for zero payload validation and ordering exceptions. |
| §5.17 metadata OBUs | todo | Needed for layer-specific metadata classification and metadata-specific constraints. |
| §5.18 frame header | todo umbrella | Split by §5.18.1-§5.18.10. Do not implement as one giant PR. |
| §5.19 tile group OBU | todo | Requires frame/sequence context and arithmetic payload boundary handling. |
| §5.20 tile group payload | todo umbrella | Deepest syntax. Requires entropy/exit-symbol handling and a clear validator scope. |

## 4. Missing semantics coverage by AV2 §6 and §7

The current validator implements only a small header/envelope slice of §6. The next semantic layers are:

| Spec area | Validator target |
|---|---|
| §6.2.2 remaining OBU header semantics | Enforce `obu_tlayer_id <= max_tlayer_id` and `obu_mlayer_id <= max_mlayer_id` after sequence header activation. |
| §6.2.3 trailing bits semantics | Enforce `trailing_one_bit == 1`, zero padding, and bit-count validity. |
| §6.4 sequence-header semantics | Validate `seq_header_id`, chroma/bit-depth ranges, layer counts, crop bounds, timing, decoder model, dependency maps, and every child syntax semantic constraint that is locally decidable. |
| §6.5-§6.16 HLS/non-frame OBU semantics | Add checks after each corresponding syntax parser exists. |
| §6.17 frame-header semantics | Split into child features. Requires sequence state and reference/frame context. |
| §6.18-§6.19 tile group semantics | Define validator scope first: structure, boundaries, and syntax conformance before full pixel decode. |
| §7.3 ordering of OBUs | Build a temporal-unit/coded-extended-layer state machine. Validate presence order, global OBU positions, padding exceptions, and HLS availability. |
| §7.3.8 HLS availability | Track activated or externally supplied sequence headers, MSDO, LCR, atlas, OPS, and repeated/identical requirements. |
| §7.4 random access | Validate random access points enough to support HLS availability, coded video sequence boundaries, and long-term-reference preconditions. |
| Annex A profiles/levels/tiers | Validate profile/tool/chroma/bit-depth constraints after enough sequence/frame syntax exists. |
| Annex E decoder model | Validate timing/decoder model after timing and buffer-removal OBUs are parsed. |

## 5. What should stay out of validator scope for now

The validator can grow toward complete bitstream conformance without becoming an encoder or full decoder too early. Keep these out of early validator PRs:

- encoder mode decisions, RDO, rate control, SIMD, motion search;
- copying AV1 syntax/tables from rav1e or SVT-AV1;
- generated AV2 tables by hand transcription;
- pixel-perfect frame reconstruction unless a validator check truly needs it;
- AVM-required CI. AVM differential tests should be optional until the harness and binaries are stable.

## 6. Implementation dependency graph

```text
bit descriptors + bounded bit reader
  -> trailing_bits / byte_alignment
  -> open_bitstream_unit payload dispatch
  -> sequence_header_obu parser
  -> sequence-header semantics + activated sequence state
  -> remaining header semantics: max_tlayer_id / max_mlayer_id
  -> temporal unit / OBU ordering state machine
  -> HLS OBUs: MSDO, LCR, OPS, atlas, BRT
  -> metadata / padding / film grain / QM / content interpretation
  -> frame header child features
  -> tile group and tile payload child features
  -> Annex A/E conformance + AVM differential proof
```

## 7. First PR recommendation

The highest-leverage next implementation is not the full frame header. It is:

1. descriptor support for `uvlc()`, `ns(n)`, `trailing_bits()`, and `byte_alignment()`;
2. typed `ParsedObu` / payload dispatch skeleton;
3. §5.4.1 general `sequence_header_obu()` parser;
4. local §6.4.1 sequence-header semantics;
5. validator state storing activated sequence headers enough to enforce the remaining §6.2.2 `max_tlayer_id` / `max_mlayer_id` checks.

That PR turns the validator from a header checker into a real payload parser while keeping the scope testable.
