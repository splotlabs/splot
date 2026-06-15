# Decoder Full Conformance Contract

`status: contract`
`owner: decoder`
`feature_id: DOC-DECODER-FULL-CONFORMANCE-CONTRACT`

This document defines what `splot` means by a future "full AV2 v1.0.0 decoder
conformance" claim. It is a contract and status boundary, not a runtime decoder
implementation.

## Current Status

`splot decode` is not currently a full AV2 decoder. The public command is a
plan-only runtime entry point: it reads bounded input bytes, constructs a
`DecodeContext`, calls `DecodeContext::plan_bytes`, renders a structured
diagnostic, and exits without reconstructing pixels.

Current `splot decode` does not:

- decode tile payload syntax to completion;
- reconstruct output frames;
- compute runtime decoded-frame hashes;
- write runtime Y4M or raw output;
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

- `raw_intermediate_output`: decoded output frames before film-grain synthesis;
- `post_film_grain_output`: output frames after normative film-grain synthesis.

The stable hash contract keeps `splot-dfh-sha256-v1` for raw intermediate output.
A future output-equivalence change must define the exact post-film-grain hash
variant, raw output bytes, Y4M behavior, visible crop handling, chroma plane
sizes, bit-depth serialization, output order, show-existing behavior, and flush
behavior before those modes can be claimed as runtime-supported.

## Diagnostics

Decoder findings are structured data, not logs. Emitted decoder diagnostics are
registered in [`DECODER-DIAGNOSTICS.md`](./DECODER-DIAGNOSTICS.md).

The current emitted decoder rules are:

- `decode/malformed-source`
- `decode/resource-limit`
- `decode/unsupported-feature`

Future full decoder work may add more rules such as
`decode/conformance-error`, `decode/internal-invariant`, and
`decode/output-error`, but every emitted rule must be registered, tested, and
linked to decoder support or coverage rows.

`decode/resource-limit` is local `splot` resource policy over measured values.
It is not an AV2 conformance failure by itself.

## Resource And Output Safety

Every bitstream-derived allocation must be guarded by checked arithmetic and
`DecodeLimits` before allocation or traversal. Required surfaces include input
bytes, OBU count, frame candidates, frame dimensions, decoded-frame bytes,
reference-store bytes, tile counts, tile payload bytes, CDF/table sizes,
transform buffers, output bytes, and worker queue sizes.

Runtime file output must be atomic before any successful `-o` mode is claimed:
write a temp file, complete serialization, flush according to the chosen policy,
rename only after success, and leave no partial final-path output on failure.

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
