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
emits the structured `decode/unsupported-feature` diagnostic and does not
reconstruct pixels, produce frame hashes, write Y4M output, read input bytes, or
touch the output path. Decode resource limits are a contract-only planning item:
`decode/resource-limit` is documented as a planned diagnostic, but is not emitted
by source yet.

Canonical decoder status lives in
[`DECODER-SUPPORT-MATRIX.toml`](./DECODER-SUPPORT-MATRIX.toml), rendered to
[`DECODER-SUPPORT-STATUS.md`](./DECODER-SUPPORT-STATUS.md). The global feature
ledger remains [`IMPLEMENTATION-MATRIX.toml`](./IMPLEMENTATION-MATRIX.toml).
Emitted `splot decode` diagnostic rule IDs are registered in
[`DECODER-DIAGNOSTICS.md`](./DECODER-DIAGNOSTICS.md), enforced by
`cargo xtask check-diagnostic-registry`.

## Supported Tier

The first supported decode tier is planned, not implemented. The intended tier
is deliberately small:

- raw Annex B and IVF-wrapped Annex B inputs through the existing stream parser;
- one operating point and one output layer;
- 8-bit 4:2:0;
- small dimensions under explicit decode limits;
- key/all-intra frames only;
- no film-grain application, multistream composition, inter prediction, loop
  restoration, or external-HLS-dependent output effects unless a row marks them
  supported with tests;
- deterministic decoded-frame hashes before Y4M output is treated as a success
  criterion.

Every stream outside the supported tier must fail explicitly with a structured
unsupported-feature diagnostic. Silent fallback to AVM, dav2d, ffmpeg, or any
other external decoder is forbidden.

## Stages

| Stage | Scope | Status |
|---|---|---|
| 0 | Roadmap, support matrix, generated status, drift gate | supported |
| 1 | Decode API contract, limits, resource diagnostics, plan-only byte entry point | partial contract documented |
| 2 | Shared decoded frame, plane, pixel format, and deterministic hash types | hash contract documented; types planned |
| 3 | CLI `splot decode` contract backed by library diagnostics | planned |
| 4 | Container traversal, layer/operating-point selection, transactional decode planning | planned |
| 5 | Self-contained decode fuzz target and fixture smoke | planned |
| 6 | AV2 § 8 symbol/CDF decoder foundation | planned |
| 7 | Constrained intra tile syntax | planned |
| 8 | Scalar intra prediction, dequant/reconstruction, inverse transform, frame hashes | planned |
| 9 | Y4M output and reconstructed reference-frame store | planned |
| 10 | Portable local-reference evidence manifests | planned |
| 11 | Encoder reconstruction API contract | planned |

## Spec Anchors

Decoder and reconstruction work must cite the committed AV2 v1.0.0 mirror:

- general decoding process: § 7.1,
  `docs/spec/av2/1.0.0/07-decoding-process.md#s-7-1`;
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
before they allocate from bitstream-derived values. The conceptual API shape is:

```text
DecodeOptions {
    limits: DecodeLimits
}
```

This is repository policy layered over AV2 syntax-derived values, not an AV2
conformance rule. The diagnostic must cite the AV2 section that supplied the
measured value, while the configured threshold comes from `DecodeLimits`.

The first contract covers:

- `max_input_bytes`;
- `max_obus`;
- `max_frames_to_decode`;
- `max_output_frames`;
- `max_frame_width`;
- `max_frame_height`;
- `max_luma_samples_per_frame`;
- `max_decoded_frame_bytes`;
- `max_reference_frames`;
- `max_tile_count`;
- `max_tile_bytes`;
- `max_output_bytes`.

The primary spec-derived surfaces are sequence maximum dimensions (§ 6.4.1),
reference-frame count (§ 6.4.6), per-frame dimensions (§ 6.17.4.1), tile grid
counts (§ 6.17.7.2), tile group count derivation (§ 5.19), tile payload
traversal (§ 5.20), the general decode input/output model (§ 7.1), decoded
output arrays (§ 7.21), and reference frame storage (§ 7.23). A future planner
must check `max_input_bytes` before buffering or accepting input bytes, check
`max_obus` before continuing OBU traversal or accumulating OBU state, and check
the relevant derived resource limit before allocating decoded frames, traversing
tile payloads, storing reference frames, producing frame hashes, or writing Y4M
output.

Every derived `actual` resource value must be computed with checked arithmetic
before comparison or allocation. Overflow while deriving dimensions, strides,
tile products, plane sizes, reference-storage bytes, output bytes, or frame
counts is a `decode/resource-limit` failure, not a wraparound or panic.

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

- frame order is zero-based AV2 § 7.21 output order after supported
  stream/layer selection, including show-existing and flush output frames once
  those output paths are implemented;
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

The default future hash variant is `raw_intermediate_output`, corresponding to
AV2 § 6.16.13 `has_grain = 0`: `OutY`/`OutU`/`OutV` from the § 7.21.2
intermediate output preparation process before § 7.21.7 film-grain synthesis.
A post-film-grain hash may be added later only as an explicit, separately named
variant after film-grain synthesis is implemented and tested.

Local AVM/dav2d MD5 output can be useful evidence, but committed `splot` tests
must not require those tools. Existing archived local reference evidence records
AVM/dav2d raw MD5 agreement for two tiny fixtures; it is non-executable
metadata only and does not prove that `splot` hash computation is implemented.

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

## Next Decision

The next implementation item must ask for explicit maintainer approval before
changing the dependency graph. The likely crate split is:

```text
crates/splot-core      bitstream model + parsers
crates/splot-recon     pixel buffers, hashes, reconstruction primitives, references
crates/splot-decode    decode driver using splot-core + splot-recon
crates/splot-encode    future encoder, reusing splot-recon
crates/splot-cli       thin CLI only
```

Until that approval lands, decoder planning must stay in docs, OpenSpec, and
automation.
