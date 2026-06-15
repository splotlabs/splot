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

Current state: `splot decode` has a narrow byte-consuming runtime success path
for the committed `minimal-intra-8bit420-hash-v1` IVF tier. It reads input
bytes, constructs `DecodeContext`, calls `DecodeContext::plan_bytes`, emits
`splot.decode.hash_report` JSON for `--output-format hash`, and atomically
publishes Y4M for `-o` / `--output-format y4m -o` on that minimal fixture.
Diagnostics remain structured data owned by `splot-decode`:
`decode/malformed-source` for malformed source/container bytes,
`decode/resource-limit` for byte-planner or runtime limit failures,
`decode/unsupported-feature` for planner unsupported structures or out-of-tier
runtime requests, and `decode/output-error` for Y4M serialization/publication
failures. It does not perform broad tile decode, broad pixel reconstruction,
raw output, film grain, reference refresh, or external decoder invocation.
Decode resource limits now have a source-backed `splot-decode` policy API for
configured thresholds and pure checks, and the byte-stream planner applies the
input-byte, OBU-count, IVF frame-record, and selected-frame-candidate limits
during traversal.
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
decode tile payloads, reconstruct pixels, produce hashes, write Y4M, or provide
runtime decode output.
`splot-recon` now exposes immutable decoded output frame and plane model types
with constructor invariants plus a bounded immutable reference-slot container,
canonical decoded-frame hash input serialization, source-backed
`splot-dfh-sha256-v1` digest computation, and a source-backed Y4M writer for
caller-supplied decoded frames. It also exposes scheduler-free scalar
prediction primitives for square and rectangular § 7.13.2.10 DC intra
prediction over caller-provided left/above edge samples, plus § 7.13.2.2
basic/PAETH prediction over prepared left/above/top-left edge samples, and
§ 7.13.2.13 smooth prediction over prepared left/above sentinel edge samples;
rectangular both-edge DC prediction uses the § 7.13.3.22 approximate divisor
path. Directional prediction, data-driven prediction, subsampled DC, IBP, CfL,
full `predict_intra()` dispatch, dequantization,
inverse transforms, residual addition, runtime decode output, output
scheduling, and AV2 reference refresh semantics remain unimplemented.
`splot-recon` remains scheduler-free:
future decoder code must partition and schedule parallel work from
`splot-decode`, then call deterministic reconstruction primitives.
`splot-core` now also exposes a bounded AV2 § 8.2 `SymbolDecoder` foundation
for caller-provided tile payload slices: initialization, pseudo-raw bool/literal
reads, caller-supplied-CDF symbol reads with optional CDF updates, and
`exit_symbol()` padding validation. This does not make runtime tile decode
supported; broad § 8.3 CDF selection, full tile CDF banks, `decode_tile()`,
broad reconstruction, broad hash output, and broad Y4M output remain future
rows beyond the committed minimal fixture tier.
`splot-decode` now also has crate-private tile-payload planning for the minimal
one-tile closed-loop-key tier. The boundary consumes § 5.20.1
`TileGroupFraming`, checks tile payload/count limits, derives one deterministic
tile work unit with exact source/layer/tile/MI-range/byte-span provenance,
initializes § 8.2 symbol state for the bounded tile slice, attaches a
crate-private first partition CDF subset (`TileDoSplitCdf` and
`TileDoSquareSplitCdf`) copied from generated § 9.3 defaults with typed § 8.3
row selection and § 8.2 copy/average policy metadata, and then stops at
structured `decode/unsupported-feature` metadata for the unimplemented
`decode_tile()` block syntax. A crate-private source-backed derivation bridge now
validates a selected `DecodePlannedObu` against a borrowed `splot-core`
`ObuEnvelope`, slices only the complete § 5.19-derived § 5.20 payload region,
uses parser-derived tile grid, quantizer, CDF, and `disable_cdf_update` facts,
and runs the resulting boundary inside the context-owned
`splot_parallel::WorkerPool`, preserving the PR #101 concurrency model without
exposing public tile-payload APIs. It is not wired to a runtime decode success
path and does not support multiple tiles or tile groups, bridge/BRU paths,
`exit_symbol()` after real syntax, Saved CDF mutation, reconstruction, hashes,
runtime Y4M, reference refresh, or external decoders.

Canonical decoder status lives in
[`DECODER-SUPPORT-MATRIX.toml`](./DECODER-SUPPORT-MATRIX.toml), rendered to
[`DECODER-SUPPORT-STATUS.md`](./DECODER-SUPPORT-STATUS.md). The global feature
ledger remains [`IMPLEMENTATION-MATRIX.toml`](./IMPLEMENTATION-MATRIX.toml).
The future full-decoder conformance claim is defined in
[`DECODER-FULL-CONFORMANCE.md`](./DECODER-FULL-CONFORMANCE.md), and the
decode-relevant AV2 section-family ownership map is generated in
[`DECODER-SPEC-COVERAGE.md`](./DECODER-SPEC-COVERAGE.md). These documents expose
current unsupported and partial runtime decoder gaps; they do not make the
narrow minimal hash/Y4M paths a full supported decoder.
The output-equivalence contract tracked by
`DOC-DECODER-OUTPUT-EQUIVALENCE-CONTRACT` defines the future runtime output
identity target: `raw_intermediate_output` and `post_film_grain_output`
variants, `splot-dfh-sha256-v1` raw-intermediate hash reporting, visible sample
bytes, show-existing and flush output order, raw/Y4M output policy, metadata
hash separation, and atomic file publication. The
`minimal-intra-8bit420-hash-v1` rows now support the first raw-intermediate hash
success artifact and the first atomically published Y4M file for the committed
minimal IVF fixture; raw output, film grain, broad output ordering, and full
decoder conformance remain unsupported.
Emitted `splot decode` diagnostic rule IDs are registered in
[`DECODER-DIAGNOSTICS.md`](./DECODER-DIAGNOSTICS.md), enforced by
`cargo xtask check-diagnostic-registry`.

## Supported Tier

The first supported decode tier is implemented for hash JSON output and
atomically published Y4M file output on the committed minimal fixture. The
repository contract is:

```text
contract_id = "splot.decode.minimal_tier"
contract_version = 1
tier_id = "minimal-intra-8bit420-hash-v1"
feature_id = "DOC-MINIMAL-DECODE-TIER-CONTRACT"
runtime_feature_id = "DECODE-MINIMAL-TIER-RUNTIME-SUCCESS"
y4m_runtime_feature_id = "DECODE-Y4M-RUNTIME-OUTPUT"
```

This is a `splot` implementation-supported subset, not an Annex A
level-conformant decoder claim. Annex A decoder conformance is broader than the
encoder-MVP subset below.

The tier is deliberately small:

- input is one committed IVF/DKIF-wrapped AV02 frame whose payload uses the
  Annex B length-delimited OBU framing; raw Annex B planning remains supported
  by the byte planner but is outside this runtime hash success tier;
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
- deterministic decoded-frame hashes and Y4M files are the first success
  artifacts; current runtime support is limited to the committed flat 64x64
  fixture, its traced six-symbol §8.2 tile stream, and its all-flat output
  model.

Runtime `splot decode` Y4M output is supported only for the committed minimal
IVF tier through `DECODE-Y4M-RUNTIME-OUTPUT`. The compatibility form
`splot decode <input> -o <output>` remains the implicit Y4M form, and
`--output-format y4m -o <output>` is the explicit Y4M form. The CLI publishes
that output atomically through a same-directory temporary file and reports
`decode/output-error` for publication failures. Hash success JSON uses the
separate `splot.decode.hash_report` schema rather than the diagnostic JSON
shape. All Y4M requests outside the minimal tier still emit a structured
unsupported/resource/malformed diagnostic without touching the requested output
path.

Everything outside the tier must fail explicitly with a structured diagnostic:
`decode/unsupported-feature` for unsupported tools or tier violations, and
`decode/resource-limit` for configured limit excess or overflow once that
diagnostic is emitted by source. Silent fallback to AVM, dav2d, ffmpeg, or any
other external decoder is forbidden.

## Stages

| Stage | Scope | Status |
|---|---|---|
| 0 | Roadmap, support matrix, generated status, drift gate | supported |
| 1 | Decode API contract, runtime context, limits, resource diagnostics, crate scaffolding, byte entry point | crate scaffolding, `DecodeContext` worker-pool runtime policy, limits runtime API, bounded byte-stream planning, and resource diagnostic emission supported |
| 2 | Shared decoded frame, plane, pixel format, workspace, and deterministic hash types | frame/plane model types, current-frame workspace, hash-input serialization, and `splot-dfh-sha256-v1` digest computation supported |
| 3 | CLI `splot decode` contract backed by library diagnostics | minimal hash JSON and minimal Y4M output supported; raw output unsupported |
| 4 | Container traversal, base-layer parsed/raw traversal, transactional decode planning | parsed and raw-byte stream planners supported; operating-point selection and broad CLI runtime unsupported |
| 5 | Self-contained decode fuzz target and fixture smoke | `decode_plan_bytes` fuzz target supported for the raw byte planner; minimal runtime fixture smoke supported |
| 6 | AV2 § 8 symbol/CDF decoder foundation | § 8.2 generic primitive partial; first crate-private partition CDF subset boundary partial; broad § 8.3 and tile decode planned |
| 7 | Constrained intra tile syntax | tile payload and tile CDF boundaries partial; `decode_tile()` syntax planned |
| 8 | Scalar intra prediction, dequant/reconstruction, inverse transform, frame hashes | current-frame workspace plus square DC, rectangular DC, basic/PAETH, and smooth prediction primitives supported; directional/DIP/subsampled DC/IBP/CfL modes, dequant/reconstruction, inverse transforms, runtime hashes planned |
| 9 | Y4M output and reconstructed reference-frame store | reference-slot runtime store, source-backed Y4M writer, and minimal runtime Y4M output supported; broad runtime Y4M output and AV2 refresh semantics planned |
| 10 | Portable local-reference evidence manifests | metadata contract and offline checker wired; two AVM/dav2d raw MD5 agreement entries recorded as non-executable metadata |
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

Frame hashing was the first supported runtime output proof and remains the
canonical deterministic sample identity check. The first repository-owned
contract is:

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

`splot-recon` source-backs this contract with
`DecodedFrameHashInput<'_, T>` and `DecodedFrameHash`. The input API serializes
a caller-supplied `DecodedFrame<T>`'s modeled visible rows and exposes
`byte_stream_id = "av2-output-samples-v1"` plus
`variant_id = "raw_intermediate_output"`. The digest API computes
`algorithm_id = "splot-dfh-sha256-v1"` over that same byte stream and exposes
raw 32-byte digest access plus lowercase hex formatting. These APIs do not
verify AV2 metadata MD5, select output order, synthesize film grain, read
bitstreams, write Y4M, reconstruct pixels, or invoke AVM/dav2d.

The default hash variant is `raw_intermediate_output`, corresponding to
AV2 § 6.16.13 `has_grain = 0`: `OutY`/`OutU`/`OutV` from the § 7.21.2
intermediate output preparation process before § 7.21.7 film-grain synthesis.
A post-film-grain hash may be added later only as an explicit, separately named
variant after film-grain synthesis is implemented and tested.

Local AVM/dav2d output can be useful evidence, but committed `splot` tests
must not require those tools. The checked local-reference evidence manifest
records AVM/dav2d raw MD5 agreement for two background fixtures and raw SHA-256
agreement for the committed minimal runtime hash fixture; it is non-executable
metadata only and does not add an external decoder dependency. Future decoder
local-reference evidence also belongs in
[`LOCAL-REFERENCE-EVIDENCE.toml`](./LOCAL-REFERENCE-EVIDENCE.toml), which is
checked by `cargo xtask check-reference-evidence` and
`cargo xtask check-decoder-support`. The manifest stores portable metadata only:
repo-relative fixture identity, upstream reference-tool identity, sanitized
command summaries, digest metadata, and assertions.

## Unsupported Feature Contract

Decoder unsupported-feature output carries structured data. Y4M requests and
planable inputs outside the minimal tier still emit this diagnostic after byte
planning succeeds:

```json
{
  "rule_id": "decode/unsupported-feature",
  "severity": "Error",
  "spec_section": "7.1",
  "matrix_row": "cli-decode-entrypoint",
  "feature_id": "CLI-DECODE",
  "message": "Byte stream planning succeeded, but `splot decode` runtime output is not implemented yet.",
  "remediation": "Use `splot validate` or `splot inspect` for bitstream analysis until CLI-DECODE implements output.",
  "detail_kind": "runtime_unsupported"
}
```

Planner-level unsupported structures also use `decode/unsupported-feature`, but
with `decode-stream-state` / `DECODE-STREAM-STATE-PLANNER` metadata and details
such as `unsupported_reason`, `obu_type`, and `byte_offset`.
Runtime hash tier rejections use `minimal-decode-tier-contract` /
`DECODE-MINIMAL-TIER-RUNTIME-SUCCESS` metadata plus `unsupported_reason`,
`tier_id`, and an optional byte offset.

The CLI renders diagnostics as text by default and as JSON with
`splot decode --json`. Library-facing decode diagnostics must preserve stable
field names for tests and encoder roundtrips. The emitted `rule_id` set is
registered in [`DECODER-DIAGNOSTICS.md`](./DECODER-DIAGNOSTICS.md).

Resource-limit diagnostics now use `decode/resource-limit` for byte-planner
limit failures. The diagnostic extends the stable decoder fields with
`limit_name`, `limit`, `actual`, `unit`, `byte_offset`, and `bit_offset`.

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
crates/splot-recon     decoded output model types; hash-input bytes, frame hashes, Y4M writer; future reconstruction primitives, references
crates/splot-parallel  approved local worker-pool and bounded-queue runtime policy
crates/splot-decode    diagnostic API; runtime context; stream planners; minimal hash runtime using splot-recon
crates/splot-encode    future encoder, not yet depending on splot-recon
crates/splot-cli       thin CLI rendering splot-decode diagnostics
```

The scaffold is still an ownership boundary for decode. `splot-recon` exposes a
runtime decoded output frame/plane model, reference-slot container,
deterministic hash-input byte serializer, `splot-dfh-sha256-v1` digest API, and
Y4M writer for caller-supplied decoded frames, but no reconstruction algorithm,
output scheduling, or AV2 reference refresh process.
`splot-decode` owns the decode scheduler boundary through
`DecodeRuntimeConfig` and `DecodeContext`, whose single `WorkerPool` is sized by
the CLI/runtime `--threads` policy. It now depends on `splot-core` for parsed
stream-planner input and the bounded raw-byte `plan_bytes` planner,
`splot-parallel` for worker-pool execution, and `splot-recon` for the narrow
minimal runtime decoded frame/hash/Y4M handoff. Broad tile decode, pixel
reconstruction, broad Y4M output, and reference update semantics remain
unsupported.
`splot-cli` reads input bytes for `splot decode`, calls the plan-only
or minimal runtime `splot-decode` handoff, renders structured diagnostics,
emits hash JSON for the supported minimal tier, and atomically publishes Y4M for
the same minimal tier. `splot-encode` remains unchanged until a later
encoder/reconstruction API change explicitly adds reuse of `splot-recon`.
