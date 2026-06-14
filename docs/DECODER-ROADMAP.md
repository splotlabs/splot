# Decoder Roadmap

`status: planned`
`owner: decoder/reconstruction`
`scope: encoder-grade decode and reconstruction support, not playback`

## Scope

`splot` remains validator-first. Decoder work exists only where it helps future
encoder roundtrips and reconstruction correctness:

- parse accepted AV2 streams through the existing Annex B/IVF front door;
- return structured unsupported-feature diagnostics for streams outside the
  supported tier;
- define decoded frames, planes, hashes, limits, and reference-frame state that
  a future encoder can reuse for closed-loop encoding;
- eventually reconstruct a small, documented all-intra tier and prove it with
  self-contained fixtures.

It is not a production media player, not an optimized decoder, and not an
AVM/dav2d wrapper.

Current state: `splot decode` is an intentional unsupported entry point. It
renders the structured `decode/unsupported-feature` diagnostic owned by
`splot-decode` and does not reconstruct pixels, produce frame hashes, write Y4M
output, read input bytes, or touch the output path. Decode resource limits now
have a source-backed `splot-decode` policy API for configured thresholds and
pure checks, and the byte-stream planner applies the input-byte, OBU-count, IVF
frame-record, and selected-frame-candidate limits during traversal. The planned
`decode/resource-limit` diagnostic remains documented but not emitted by source.
The workspace now includes scaffolded `splot-recon` and `splot-decode` crates
for future reconstruction primitives and the future decode driver. `splot-decode`
also exposes `DecodeRuntimeConfig` and `DecodeContext`; each context owns one
`splot_parallel::WorkerPool` configured by the `--threads auto|N` runtime policy,
and now provides library-only stream planners over raw bytes and already parsed
`splot-core` stream structures. `DecodeContext::plan_bytes` walks raw Annex B or
IVF/DKIF bytes with bounded traversal before returning the same
`DecodeStreamPlan` as `DecodeContext::plan_stream`; both paths preserve raw
Annex B / IVF OBU order and offset metadata, select only the base minimal-tier
layer, treat `OBU_CLOSED_LOOP_KEY` as the only frame candidate, and reject
malformed sources or unsupported structures transactionally. These APIs do not
decode tile payloads, reconstruct pixels, produce hashes, write Y4M, or change
`splot decode` CLI behavior.
`splot-recon` now exposes immutable decoded output frame and plane model types
with constructor invariants plus a bounded immutable reference-slot container,
and canonical decoded-frame hash input serialization, but no reconstruction
algorithm, hash digest computation, Y4M output, or AV2 reference refresh
semantics exists yet. `splot-recon` remains scheduler-free:
future decoder code must partition and schedule parallel work from
`splot-decode`, then call deterministic reconstruction primitives.

Canonical decoder status lives in
[`DECODER-SUPPORT-MATRIX.toml`](./DECODER-SUPPORT-MATRIX.toml), rendered to
[`DECODER-SUPPORT-STATUS.md`](./DECODER-SUPPORT-STATUS.md). The global feature
ledger remains [`IMPLEMENTATION-MATRIX.toml`](./IMPLEMENTATION-MATRIX.toml).
Emitted `splot decode` diagnostic rule IDs are registered in
[`DECODER-DIAGNOSTICS.md`](./DECODER-DIAGNOSTICS.md), enforced by
`cargo xtask check-diagnostic-registry`.

## Supported Tier

The first supported decode tier is planned, not implemented. The repository
contract is:

```text
contract_id = "splot.decode.minimal_tier"
contract_version = 1
tier_id = "minimal-intra-8bit420-hash-v1"
feature_id = "DOC-MINIMAL-DECODE-TIER-CONTRACT"
```

This is a `splot` implementation-supported subset, not an Annex A
level-conformant decoder claim. Annex A decoder conformance is broader than the
encoder-MVP subset below.

The tier is deliberately small:

- input is Annex B length-delimited OBU data, either raw or IVF/DKIF-wrapped
  with Annex B frame payloads;
- one selected stream/layer only: non-global OBUs use `obu_xlayer_id == 0`,
  `obu_tlayer_id == 0`, and inferred `obu_mlayer_id == 0`;
- no external HLS, multistream composition, sub-bitstream extraction, MSDO, LCR,
  Atlas, or OPS selection path;
- sequence format uses `seq_profile_idc == 0` (`Main_420_10_IP0`),
  `chroma_format_idc == 0`, `bit_depth_idc == 1` (8-bit),
  `max_tlayer_id == 0`, `max_mlayer_id == 0`, `SeqMaxMlayerCnt == 1`, and
  `film_grain_params_present == 0`;
- frame dimensions, tile counts, decoded-frame bytes, reference-store bytes,
  hash bytes, and output bytes pass `DecodeLimits` using checked arithmetic
  before allocation or output;
- accepted frames are closed-loop key-frame output only, with parsed facts
  proving `obu_type == OBU_CLOSED_LOOP_KEY`, `FrameType = KEY_FRAME`, and
  `FrameIsIntra = 1`;
- inline frame headers only: `cur_mfh_id == 0`, `frame_size_override_flag == 0`,
  `immediate_output_frame == 1`, `implicit_output_frame == 0`, and no sequence
  cropping window;
- one tile and one first-and-only tile group;
- deterministic decoded-frame hashes are the first success artifact.

Y4M output remains unsupported until the `output-y4m` row is implemented and
tested against the same cropped visible output samples. The CLI parse contract
accepts future hash-output selection with `--output-format hash`; the
compatibility form `splot decode <input> -o <output>` remains the implicit Y4M
form, and `--output-format y4m -o <output>` is the explicit Y4M form. All valid
forms still emit the intentional unsupported diagnostic until runtime decode
support lands.

Everything outside the tier must fail explicitly with a structured diagnostic:
`decode/unsupported-feature` for unsupported tools or tier violations, and
`decode/resource-limit` for configured limit excess or overflow once that
diagnostic is emitted by source. Silent fallback to AVM, dav2d, ffmpeg, or any
other external decoder is forbidden.

## Stages

| Stage | Scope | Status |
|---|---|---|
| 0 | Roadmap, support matrix, generated status, drift gate | supported |
| 1 | Decode API contract, runtime context, limits, resource diagnostics, crate scaffolding, plan-only byte entry point | crate scaffolding, `DecodeContext` worker-pool runtime policy, limits runtime API, and bounded byte-stream planning supported; resource diagnostic emission planned |
| 2 | Shared decoded frame, plane, pixel format, and deterministic hash types | frame/plane model types and hash-input serialization supported; digest computation planned |
| 3 | CLI `splot decode` contract backed by library diagnostics | hash output parse contract wired; runtime unsupported |
| 4 | Container traversal, base-layer parsed/raw traversal, transactional decode planning | parsed and raw-byte stream planners supported; operating-point selection and CLI runtime planned |
| 5 | Self-contained decode fuzz target and fixture smoke | `decode_plan_bytes` fuzz target supported for the raw byte planner; decode fixtures planned |
| 6 | AV2 § 8 symbol/CDF decoder foundation | planned |
| 7 | Constrained intra tile syntax | planned |
| 8 | Scalar intra prediction, dequant/reconstruction, inverse transform, frame hashes | planned |
| 9 | Y4M output and reconstructed reference-frame store | reference-slot runtime store supported; Y4M and AV2 refresh semantics planned |
| 10 | Portable local-reference evidence manifests | metadata contract and offline checker wired; no entries yet |
| 11 | Encoder reconstruction API contract | planned |

## Runtime Concurrency Contract

Decoder and reconstruction work must follow the repository concurrency policy in
[`CONCURRENCY.md`](./CONCURRENCY.md), tracked by
`INFRA-PARALLEL-RUNTIME-POLICY`. This is project runtime policy, not an AV2
conformance rule, and it does not make the current unsupported decode entry
point byte-consuming.

The decoder/reconstruction ownership rule is:

- `splot-decode` owns runtime orchestration through `DecodeRuntimeConfig` and a
  `DecodeContext` with exactly one `splot_parallel::WorkerPool`;
- data-parallel decode work must reach Rayon traits only through
  `splot_parallel::prelude::*` and must run inside
  `ctx.pool().install(|| { ... })`;
- `splot-recon` stays pool-agnostic and must not construct worker pools, spawn
  codec worker threads, depend on Rayon/crossbeam directly, or own decode
  pipeline queues;
- bounded queues are allowed only through `splot_parallel::bounded_queue` at
  coarse producer/consumer boundaries, never for per-pixel, per-block, per-row,
  or other hot inner-loop signalling;
- future decoded-frame hashes, Y4M output, diagnostics, stats, progress events,
  and reference-state commits must be emitted in AV2 bitstream, presentation, or
  `splot` emission-index order, not worker completion order.

Future runtime decode rows may not be marked supported until self-contained tests
prove the supported behavior across all required thread-count forms:
`--threads 1`, `--threads auto`, and at least one fixed positive
`--threads N`. `--threads 0` remains a CLI/runtime alias for `auto`, resolved
once when the context-owned pool is created.

The current parsed stream planner is intentionally serial but runs through
`DecodeContext`, so future parallel planning or decode work already has the
context-owned `WorkerPool` boundary. Its tests prove identical plan metadata
across `ThreadCount::Auto`, one worker, and a fixed positive worker count. It
does not call direct Rayon/crossbeam APIs, does not construct queues, and does
not make `splot-recon` scheduler-aware.

## Spec Anchors

Decoder and reconstruction work must cite the committed AV2 v1.0.0 mirror:

- general decoding process: § 7.1,
  `docs/spec/av2/1.0.0/07-decoding-process.md#s-7-1`;
- Annex B length-delimited input: Annex B.2-Annex B.3,
  `docs/spec/av2/1.0.0/annex-b-length-delimited-bitstream-format.md#s-annex-b-2`;
- OBU syntax and OBU header semantics: § 5.2 and § 6.2,
  `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-2`;
- sequence format, layer counts, and frame-size semantics: § 6.4.1 and
  § 6.17.4.1,
  `docs/spec/av2/1.0.0/06-syntax-structures-semantics.md#s-6-4-1`;
- temporal units, coded extended layer units, and random access: § 7.3-§ 7.4,
  `docs/spec/av2/1.0.0/07-decoding-process.md#s-7-3`;
- tile group and tile payload syntax: § 5.19-§ 5.20,
  `docs/spec/av2/1.0.0/05-syntax-structures.md#s-5-19`;
- parsing process and symbol/CDF decoding: § 8.2-§ 8.3,
  `docs/spec/av2/1.0.0/08-parsing-process.md#s-8-2`;
- prediction, reconstruction, inverse transforms, filters, output, and reference
  updates: § 7.13-§ 7.23,
  `docs/spec/av2/1.0.0/07-decoding-process.md#s-7-13`.

Do not infer AV2 syntax from AV1 projects or copy source, constants, tables,
comments, or prose from AVM, dav2d, rav1e, SVT-AV1, or any other implementation.

## Decode Limits Contract

Future byte-consuming decode entry points must accept explicit resource limits
before they allocate from bitstream-derived values. The source-backed runtime
API shape is:

```text
DecodeOptions {
    limits: DecodeLimits
}
```

This is repository policy layered over AV2 syntax-derived values, not an AV2
conformance rule. The diagnostic must cite the AV2 section that supplied the
measured value, while the configured threshold comes from `DecodeLimits`.
`splot-decode` now provides typed limit names, units, thresholds, actual values,
and pure check helpers for this contract. Those helpers are not decoder
diagnostics and do not read bytes, traverse OBUs, allocate frames, write output,
or change the current unsupported `splot decode` behavior.

The first contract covers:

- `max_input_bytes`;
- `max_obus`;
- `max_ivf_frame_records`;
- `max_frames_to_decode`;
- `max_output_frames`;
- `max_frame_width`;
- `max_frame_height`;
- `max_luma_samples_per_frame`;
- `max_decoded_frame_bytes`;
- `max_reference_slots`;
- `max_reference_store_bytes`;
- `max_tile_count`;
- `max_tile_payload_bytes`;
- `max_output_bytes`.

The primary spec-derived surfaces are leb128 length fields (§ 4.11.6), Annex B
length-delimited input (Annex B.2-Annex B.3), OBU sizing (§ 5.2.1), sequence
maximum dimensions (§ 6.4.1), reference-frame count (§ 6.4.6), per-frame
dimensions (§ 6.17.4.1), tile grid counts (§ 6.17.7.2), tile group count and
semantics (§ 5.19 and § 6.18), tile payload traversal and semantics (§ 5.20.1
and § 6.19.1), the general decode input/output model (§ 7.1), decoded output
arrays (§ 7.21), and reference frame storage (§ 7.23). The byte-stream planner
checks `max_input_bytes` before traversing accepted input bytes, checks
`max_obus` before continuing OBU traversal or accumulating OBU state, and checks
`max_ivf_frame_records` before traversing each complete IVF frame record. Future
runtime stages must check the relevant derived resource limit before allocating
decoded frames, traversing tile payloads, storing reference frames, producing
frame hashes, or writing Y4M output.

Every derived `actual` resource value must be computed with checked arithmetic
before comparison or allocation. Overflow while deriving dimensions, strides,
tile products, plane sizes, reference-storage bytes, output bytes, or frame
counts is a `decode/resource-limit` failure, not a wraparound or panic.
Runtime emission of that diagnostic remains future work and belongs at the
byte-consuming decode boundary.
`DecodeLimits::zero()` and `DecodeLimits::unlimited()` are explicit constructors
for tests and callers; `DecodeLimits::default()` and `DecodeOptions::default()`
use finite repository policy thresholds for CI, fuzzing, and early decoder work.

`DecodeContext::plan_stream` applies only the limits it can derive from an
already parsed stream: `max_input_bytes` from the caller-supplied input length,
`max_obus` before adding each planned OBU, `max_ivf_frame_records` before
traversing each IVF frame record, and `max_frames_to_decode` before accepting
each closed-loop-key frame candidate. `DecodeContext::plan_bytes` is the first
raw byte-consuming planner: it performs bounded raw Annex B / IVF traversal and
then reuses the same selected-frame-candidate limit and unsupported-structure
classification as the parsed planner. It is still plan-only and does not parse
tile payloads or allocate decoded frames.

## Decoded Frame and Plane Model Contract

Decoded-frame data structures must preserve AV2 output semantics while
remaining reusable by future reconstruction, reference-frame storage, hashes,
Y4M, and encoder closed-loop tests. The source-backed `splot-recon` model now
provides:

```text
DecodedFrameInfo
DecodedFrame
FramePlanes<T>
Plane<T>
PlaneSize
PlaneRect
PixelFormat
BitDepth
OutputIndex
ReferenceSlot
ReferenceFrameStore<F>
ReferenceFrameEntry<'a, F>
ReferenceFrameEntries<'a, F>
ReconError
```

This is a committed Rust output-model API, not a byte-consuming decode API.
The model validates AV2-derived frame/plane geometry, sample storage, and
reference-slot container bounds, but does not reconstruct pixels, compute
hashes, write Y4M, or implement AV2 reference refresh semantics.

`PixelFormat` is derived from AV2 § 6.4.1 `chroma_format_idc`:

- `Monochrome` / 4:0:0: `SubsamplingX = 1`, `SubsamplingY = 1`,
  `NumPlanes = 1`;
- `Yuv420`: `SubsamplingX = 1`, `SubsamplingY = 1`, `NumPlanes = 3`;
- `Yuv422`: `SubsamplingX = 1`, `SubsamplingY = 0`, `NumPlanes = 3`;
- `Yuv444`: `SubsamplingX = 0`, `SubsamplingY = 0`, `NumPlanes = 3`.

`BitDepth` is derived from AV2 § 6.4.1 `bit_depth_idc`: AV2 v1.0.0 permits
10-bit samples for `bit_depth_idc = 0` and 8-bit samples for
`bit_depth_idc = 1`. Future decoded sample storage must reject values outside
`0..=(1 << bit_depth) - 1`.

The model must distinguish coded/reconstructed storage from cropped output:

- coded luma dimensions are `FrameWidth x FrameHeight` (§ 6.17.4.1);
- the visible output luma rectangle is `CropLeft`, `CropTop`, `CropWidth`, and
  `CropHeight`; `CropWidth` and `CropHeight` must be positive, and non-monochrome
  crop origins must be aligned to `SubsamplingX` / `SubsamplingY` (§ 6.17.4.4);
- decoded output frames are AV2 § 7.21 `OutY`/`OutU`/`OutV` arrays emitted by
  the AV2 output processes (§ 7.1, § 7.21.5, § 7.21.6);
- `splot` assigns a zero-based emission index in that output-process order after
  supported stream/layer selection; this index is repository-owned metadata, not
  an AV2 syntax element, and it is not decode order;
- output luma dimensions are `w x h` from § 7.21.2, and output chroma dimensions
  are `((w + subX) >> subX) x ((h + subY) >> subY)`;
- U and V planes are absent or ignored when `NumPlanes == 1`.

`Plane<T>` may include padding for efficient storage, and it carries explicit
storage `width`, storage `height`, `stride_samples`, and visible rectangle
metadata when storage and visible output differ. Invariants:

- `stride_samples >= storage_width`;
- `required_samples = stride_samples * storage_height` is computed with checked
  arithmetic;
- the backing buffer exposes exactly `required_samples` samples, and
  `allocation_bytes = required_samples * bytes_per_sample` is computed with
  checked arithmetic before reporting backing size;
- every product used for dimensions, strides, backing samples, byte sizes, hash
  lengths, Y4M output, or reference storage uses checked arithmetic;
- `splot-recon` constructors reject local arithmetic overflow with typed
  `ReconError` values and do not emit decoder diagnostics directly;
- future byte-consuming decode code must charge the full backing allocation,
  including padding, against `DecodeLimits` before allocation;
- future byte-consuming decode code reports allocation overflow or
  configured-limit excess as `decode/resource-limit`;
- padding and stride samples are not visible decoded output and must be excluded
  from frame hashes, Y4M output, and fixture expectations.

Reference-frame storage is related but not the same shape as output. AV2 § 7.23
stores loop-restored `LrFrame` into `FrameStore` over padded coded dimensions
(`MiCols * MI_SIZE` by `MiRows * MI_SIZE` for luma, shifted by subsampling for
chroma) and records reference metadata such as `RefFrameWidth`,
`RefFrameHeight`, `RefCropWidth`, `RefCropHeight`, `RefCropLeft`, `RefCropTop`,
`RefSubsamplingX`, `RefSubsamplingY`, `RefBitDepth`, `RefNumPlanes`,
`RefOutputOrder`, `RefOrderHint`, and `RefFilmGrainPresent`. Future APIs must
not treat cropped output-frame dimensions and reference-store backing dimensions
as interchangeable.

The source-backed `splot-recon` reference store is a safe runtime container for
future callers that have already derived AV2 reference update decisions:

- `ReferenceSlot::MAX_SLOTS == 16`, matching AV2 § 3 `NUM_REF_FRAMES`;
- `ReferenceSlot::new` validates indices in `0..16`;
- `ReferenceFrameStore<F>::with_capacity` validates a fixed capacity in
  `1..=16`;
- `put`, `get`, `take`, `clear`, `occupied`, and `entries` manage immutable
  caller-owned frame payloads without exposing mutable frame access;
- entries iterate occupied slots in ascending `ReferenceSlot` order.

The payload type is intentionally generic so future reference/reconstruction
payloads do not need to fabricate output-emission metadata just to live in the
store. This runtime store does not model active `NumRefFrames` or
`ActiveNumRefFrames` from § 5.4.6 / § 6.4.6, AV2 `RefValid`,
`refresh_frame_flags`, output scheduling, show-existing deduplication, motion
vectors, CDFs, grain params, segment IDs, global motion state, CCSO params, or
any other § 7.23 metadata. Future byte-consuming decode code must translate
parsed AV2 state into store operations and charge allocations against
`DecodeLimits` before storing frames.

Emitted output frames must remain immutable and valid after emission. Reference
slots may own or share reconstructed buffers, but overwriting a reference slot
must not mutate an already emitted output frame. Borrowed or shared views are
allowed only when the backing samples are immutable for the output view, when the
output owns an independent copy, or when copy-on-write or unique ownership is
proven before any reference-slot mutation.

## Hash Policy

Frame hashing is required before Y4M output is considered supported. The first
repository-owned contract is:

```text
contract_id = "splot.decoded_frame_hash"
contract_version = 1
algorithm_id = "splot-dfh-sha256-v1"
byte_stream_id = "av2-output-samples-v1"
```

The `splot-dfh-sha256-v1` digest is SHA-256 over canonical decoded output sample
bytes. The sample-byte stream follows AV2 § 6.16.13's decoded-frame-hash sample
serialization, but the digest is `splot`-owned fixture and roundtrip identity,
not the AV2 metadata MD5 value. AV2 `hash_type = 0` MD5 remains a separate
future verification path for `METADATA_TYPE_DECODED_FRAME_HASH` metadata.

The canonical byte stream is defined as follows:

- frame order uses the `splot` zero-based emission index assigned in AV2
  output-process order after supported stream/layer selection, including
  show-existing and flush output frames once those output paths are implemented;
- region is cropped visible output only: luma dimensions are `w x h`; chroma
  dimensions are `((w + subX) >> subX) x ((h + subY) >> subY)`;
- backing allocation padding and `Plane` stride bytes are excluded;
- non-monochrome plane order is Y, then U, then V; monochrome output hashes only
  Y;
- samples are traversed left-to-right, top-to-bottom within each plane;
- 8-bit samples are written as one byte;
- samples with bit depth greater than 8 are written as two bytes in little-endian
  order, least significant byte first, with no normalization;
- codec metadata, OBU bytes, container timestamps, HDR/ICC/timecode metadata,
  and signaled decoded-frame-hash metadata are excluded from the digest input
  and must be asserted separately when relevant.

`splot-recon` source-backs the byte-stream portion of this contract with
`DecodedFrameHashInput<'_, T>`. That API serializes a caller-supplied
`DecodedFrame<T>`'s modeled visible rows and exposes
`byte_stream_id = "av2-output-samples-v1"` plus
`variant_id = "raw_intermediate_output"`. It does not compute SHA-256, verify
AV2 metadata MD5, select output order, synthesize film grain, read bitstreams,
or invoke AVM/dav2d.

The default future hash variant is `raw_intermediate_output`, corresponding to
AV2 § 6.16.13 `has_grain = 0`: `OutY`/`OutU`/`OutV` from the § 7.21.2
intermediate output preparation process before § 7.21.7 film-grain synthesis.
A post-film-grain hash may be added later only as an explicit, separately named
variant after film-grain synthesis is implemented and tested.

Local AVM/dav2d MD5 output can be useful evidence, but committed `splot` tests
must not require those tools. Existing archived local reference evidence records
AVM/dav2d raw MD5 agreement for two tiny fixtures; it is non-executable
metadata only and does not prove that `splot` hash computation is implemented.
Future decoder local-reference evidence belongs in
[`LOCAL-REFERENCE-EVIDENCE.toml`](./LOCAL-REFERENCE-EVIDENCE.toml), which is
checked by `cargo xtask check-reference-evidence` and
`cargo xtask check-decoder-support`. The manifest stores portable metadata only:
repo-relative fixture identity, upstream reference-tool identity, sanitized
command summaries, digest metadata, and assertions.

## Unsupported Feature Contract

Decoder unsupported-feature output carries structured data. The current
`splot decode` entry point emits this diagnostic for all inputs until a
supported decode tier lands:

```json
{
  "rule_id": "decode/unsupported-feature",
  "severity": "Error",
  "spec_section": "7.1",
  "matrix_row": "cli-decode-entrypoint",
  "feature_id": "CLI-DECODE",
  "message": "`splot decode` is not implemented for AV2 bitstreams yet.",
  "remediation": "Use `splot validate` or `splot inspect` for bitstream analysis until CLI-DECODE is implemented."
}
```

The CLI renders the diagnostic as text by default and as JSON with
`splot decode --json`. Future library-facing decode diagnostics must preserve
stable field names for tests and encoder roundtrips. The emitted `rule_id` set
is registered in [`DECODER-DIAGNOSTICS.md`](./DECODER-DIAGNOSTICS.md).

Planned future resource-limit diagnostics use `decode/resource-limit`, but that
ID must stay outside the emitted decoder registry until source emits it. The
planned diagnostic extends the stable decoder fields with `limit_name`, `limit`,
`actual`, `unit`, `byte_offset`, and `bit_offset`.

## Local References

AVM and dav2d are local development aids only. They may be used to read source
code, generate tiny streams, compare decoded hashes, or record evidence in
agent logs, PR descriptions, and portable manifests.

They must not be added as:

- source, submodule, vendored tree, binary, object file, generated binding, or
  copied snippet;
- Cargo dependency, build dependency, `build.rs` probe, wrapper script, or
  runtime process execution;
- `xtask` command, CI job, Docker image, cache, default test path, or required
  developer setup.

Committed evidence must be portable: no local absolute paths and no assumption
that CI will rerun AVM or dav2d.

## Crate Split

Maintainer approval for the decoder/reconstruction dependency graph landed on
2026-06-13. The approved crate split is now scaffolded as:

```text
crates/splot-core      bitstream model + parsers
crates/splot-recon     decoded output model types; hash-input bytes; future reconstruction primitives, references
crates/splot-parallel  approved local worker-pool and bounded-queue runtime policy
crates/splot-decode    unsupported diagnostic API; runtime context; parsed stream planner; future driver using splot-recon
crates/splot-encode    future encoder, not yet depending on splot-recon
crates/splot-cli       thin CLI rendering splot-decode diagnostics
```

The scaffold is still an ownership boundary for decode. `splot-recon` exposes a
runtime decoded output frame/plane model, reference-slot container, and
deterministic hash-input byte serializer, but no reconstruction algorithm, AV2
reference refresh process, hash digest computation, or Y4M writer.
`splot-decode` owns the future decode scheduler boundary through
`DecodeRuntimeConfig` and `DecodeContext`, whose single `WorkerPool` is sized by
the CLI/runtime `--threads` policy. It now depends on `splot-core` for parsed
stream-planner input and the bounded raw-byte `plan_bytes` planner, while still
exposing no runtime tile decode, pixel reconstruction, hash digest, Y4M output,
or reference update semantics.
`splot-cli` only renders the current unsupported diagnostic path, and
`splot-encode` remains unchanged until a later encoder/reconstruction API
change explicitly adds reuse of `splot-recon`.
