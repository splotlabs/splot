# Decoder Full Conformance Contract

`status: contract`
`owner: decoder`
`feature_id: DOC-DECODER-FULL-CONFORMANCE-CONTRACT`

This document defines what `splot` means by a future "full AV2 v1.0.0 decoder
conformance" claim. It is a contract and status boundary, not a runtime decoder
implementation.

## Current Status

`splot decode` is not currently a full AV2 decoder. The public command has one
narrow runtime success path: `--output-format hash --json` on the committed
`minimal-intra-8bit420-hash-v1` flat 64x64 intra IVF fixture verifies the
traced §8.2 tile-symbol stream and emits a `splot.decode.hash_report` v1
artifact. Other inputs are still planner-only or fail closed at the runtime tier
gate: the command reads bounded input bytes, constructs a `DecodeContext`,
reuses `DecodeContext::plan_bytes`, and renders structured diagnostics for
malformed sources, resource limits, unsupported planner structures, or
out-of-tier runtime features.

Current `splot decode` does not:

- decode broad tile payload syntax to completion;
- reconstruct output frames beyond the single traced all-flat minimal fixture;
- compute runtime decoded-frame hashes beyond the minimal raw-intermediate hash
  tier;
- write runtime raw output or Y4M output beyond the committed minimal IVF tier;
- perform AV2 reference refresh or output scheduling;
- synthesize film grain;
- invoke AVM, dav2d, ffmpeg, or any external decoder.

Current generated status lives in:

- [`DECODER-SUPPORT-STATUS.md`](./DECODER-SUPPORT-STATUS.md)
- [`DECODER-SPEC-COVERAGE.md`](./DECODER-SPEC-COVERAGE.md)
- [`DECODER-FULL-CONFORMANCE-GAP-AUDIT.md`](./DECODER-FULL-CONFORMANCE-GAP-AUDIT.md)

## Full Conformance Claim

A full decoder conformance claim is allowed only when all normative AV2 v1.0.0
decode-relevant coverage rows in `docs/DECODER-SPEC-COVERAGE.md` are supported
with self-contained proof, and `splot decode` accepts every conforming AV2
Annex B bitstream within configured resource limits.

At that point, conforming streams must not fail with temporary
`decode/unsupported-feature`. A conforming stream that fails after full
completion is a bug or a resource-policy rejection, not an unsupported feature.

The claim covers:

- Section 4 descriptor and primitive parsing used by decoding;
- Section 5 syntax structures and matching Section 6 semantics that affect
  decoder state, tile payloads, reconstruction, output, layers, metadata, and
  model constraints;
- Section 7 decoding process, including ordering, random access, CDF lifecycle,
  prediction, transforms, filtering, output, film grain, motion-field storage,
  and reference-frame update;
- Section 8 symbol and CDF parsing;
- Section 9 normative lookup/default tables consumed by decoding;
- Annex A profiles, levels, tiers, and decoder conformance;
- Annex B length-delimited bitstream format;
- Annex E decoder model constraints when activated or signaled.

Informative annexes remain out of scope unless a normative section explicitly
depends on them.

## Output Variants

Full conformance requires named output variants:

- `raw_intermediate_output`: the AV2 § 7.21.2 intermediate output sample set
  before film-grain synthesis;
- `post_film_grain_output`: the output sample set after the AV2 § 7.21.7
  film-grain synthesis process when that process applies.

The stable hash contract keeps `splot-dfh-sha256-v1` over
`av2-output-samples-v1` for `raw_intermediate_output`. A future
post-film-grain hash may use the same digest algorithm and sample-byte stream,
but it must carry `post_film_grain_output` as the variant identifier and cannot
be claimed supported until film-grain synthesis is implemented and tested.
Streams with no applied film grain may produce identical bytes for both
variants; the variant identifier still remains part of artifact identity.

Runtime output order is AV2 output-process order after supported operating-point
or layer selection. Each emitted output event receives a zero-based
`output_index`, including show-existing events that reuse stored frame samples.
Implicit output and flush output are appended in the order required by § 7.21.4
and § 7.21.5. Output order must not depend on decode order, OBU order,
reference-slot index, hash completion order, file-write completion order, or
worker completion order. For § 7.21.4 implicit-output eligibility, a
show-existing output reached through `output_process(-1)` with
`ShowExistingFrame == 1` does not mark the referenced frame as already output;
only an explicit output of that reference index consumes that implicit/flush
eligibility.

Canonical output sample bytes use the visible output rectangle from the AV2
output process. Luma is `w` by `h`; non-monochrome chroma planes are
`((w + subX) >> subX)` by `((h + subY) >> subY)`; monochrome output omits U and
V. Plane order is Y, then U, then V, in raster order within each present plane.
8-bit samples serialize as one byte, and greater-than-8-bit samples serialize
as two little-endian bytes. Stride padding, backing allocation padding,
reference-store metadata, OBU bytes, container bytes, and decoded-frame-hash
metadata are excluded from sample-byte hashes and raw output payloads.

Successful `splot decode --output-format hash --json` output is a success
artifact, not a diagnostic JSON object. Current runtime support is limited to
`minimal-intra-8bit420-hash-v1`; the full schema contract is:

```text
contract_id = "splot.decode.hash_report"
contract_version = 1
selected_output_variants = [ ... ]
selected_thread_policy
frames = [
  {
    output_index,
    visible_luma_left,
    visible_luma_top,
    visible_luma_width,
    visible_luma_height,
    chroma_left,
    chroma_top,
    chroma_width,
    chroma_height,
    bit_depth,
    pixel_format,
    hashes = [
      {
        variant,
        algorithm_id,
        byte_stream_id,
        digest_hex
      }
    ]
  }
]
```

`selected_output_variants` records the report-level requested variants even for
empty output. `selected_thread_policy` records the resolved CLI/runtime thread
policy used for the decode run. `frames` are sorted by `output_index`.
`visible_luma_left` and `visible_luma_top` record the visible crop origin used
to derive the output sample arrays. For monochrome output, `chroma_left`,
`chroma_top`, `chroma_width`, and `chroma_height` are omitted; otherwise
`chroma_left = visible_luma_left >> subX` and
`chroma_top = visible_luma_top >> subY`. `digest_hex` is exactly 64 lowercase
hexadecimal characters for `splot-dfh-sha256-v1`.

Future raw output is concatenated canonical sample bytes for each output event
in output-index order for the selected variant, with no header or metadata
bytes. This contract does not add a current `--output-format raw` CLI mode.
Y4M output represents the chosen AV2 output-frame sample set using the
repository-owned Y4M container policy; Y4M container bytes are not AV2 syntax.
Current runtime Y4M support is limited to the committed minimal IVF tier tracked
by `DECODE-Y4M-RUNTIME-OUTPUT`.

## Diagnostics

Decoder findings are structured data, not logs. Emitted decoder diagnostics are
registered in [`DECODER-DIAGNOSTICS.md`](./DECODER-DIAGNOSTICS.md).

The current emitted decoder rules are:

- `decode/malformed-source`
- `decode/output-error`
- `decode/resource-limit`
- `decode/unsupported-feature`

Future full decoder work may add more rules such as
`decode/conformance-error` and `decode/internal-invariant`, but every emitted
rule must be registered, tested, and linked to decoder support or coverage rows.

Successful runtime file-output modes must register and emit
`decode/output-error` for output path creation, temporary-file write, flush,
sync, rename, parent-directory sync, cleanup, or serialization failures that are
not AV2 bitstream conformance failures. These failures use diagnostic JSON on
`--json` paths, not partial success artifacts.

`decode/resource-limit` is local `splot` resource policy over measured values.
It is not an AV2 conformance failure by itself.

## Resource And Output Safety

Every bitstream-derived allocation must be guarded by checked arithmetic and
`DecodeLimits` before allocation or traversal. Required surfaces include input
bytes, OBU count, frame candidates, frame dimensions, decoded-frame bytes,
reference-store bytes, tile counts, tile payload bytes, CDF/table sizes,
transform buffers, output bytes, and worker queue sizes.

Runtime file output must be atomic before any successful `-o` mode is claimed:
write a same-directory temp file, complete serialization, flush user-space
buffers, sync the temp file's contents and metadata, rename only after those
steps succeed, then sync the parent directory so the final name is durably
published. If decode, reconstruction, hash serialization, raw/Y4M
serialization, validation, temp-file write, flush, temp-file sync, rename, or
any other pre-rename publication step fails, an absent final path remains absent
and an existing final path remains byte-for-byte unchanged. If rename succeeds
but the following parent-directory sync fails, the command must report
`decode/output-error` as a durability failure; the final path may already contain
the complete serialized output, but it must never contain a partially serialized
payload. Any such failure is reported as a registered decoder diagnostic, not as
a partial success artifact. Output-derived frame counts, dimensions, row byte
counts, total output bytes, JSON frame arrays, raw payload bytes, and Y4M
payload bytes must use checked arithmetic and `DecodeLimits` before allocation,
indexing, or output publication.

## Evidence Boundary

AVM v1.0.0 and dav2d may be used only as local development evidence. The
repository may commit portable metadata such as tool revisions, command
summaries, fixture hashes, output hashes, and agreement notes.

The repository must not add AVM/dav2d source, binaries, submodules, dependencies,
wrappers, setup scripts, Docker images, caches, CI jobs, runtime process
execution, or `xtask` commands that locate, build, invoke, or require AVM or
dav2d.

Local reference evidence is supplemental. It cannot by itself make a runtime
decoder row `supported`; supported runtime decoder claims require self-contained
implementation tests, fixtures, fuzz/property coverage for byte-consuming
boundaries, and output/thread determinism where output is involved.

## Final Definition Of Done

The full decoder mission is complete only when:

1. `docs/DECODER-SPEC-COVERAGE.md` has zero unsupported normative rows, zero
   unexplained partial normative rows, and zero false runtime support claims.
2. `docs/DECODER-SUPPORT-MATRIX.toml` and generated
   `docs/DECODER-SUPPORT-STATUS.md` show no unimplemented normative decoder row.
3. `splot decode` decodes every committed valid decoder fixture and produces the
   expected hash, Y4M, or raw output where requested.
4. Every committed invalid decoder fixture fails with deterministic structured
   diagnostics.
5. Fuzz smoke covers every byte-consuming decoder entry point in CI.
6. Output bytes and diagnostics are deterministic across supported thread
   policies.
7. Local AVM evidence exists for every supported normative feature group, and
   dav2d evidence exists where dav2d supports the same AV2 version and feature.
8. Differential disagreements are resolved or documented as upstream/spec
   blockers without a false conformance claim.
9. `cargo xtask check-decoder-conformance-coverage` is green.
10. `cargo xtask ci` is green.
11. `openspec validate --all --no-interactive` is green.
12. No AVM/dav2d integration boundary is violated.
