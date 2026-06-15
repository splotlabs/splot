## Context

The merged full-conformance contract makes output identity a prerequisite for
future runtime decoder support. Today `splot decode` is still a plan-only entry
point: it accepts future `hash` and `y4m` selections, reads and plans bounded
input, emits diagnostics, and deliberately writes no output. `splot-recon`
already provides source-backed visible-sample serialization and
`splot-dfh-sha256-v1` digest computation for caller-supplied
`raw_intermediate_output` frames, plus a source-backed Y4M writer for
caller-supplied frames. Those primitives do not define the CLI success artifact,
output ordering, post-film-grain variant, or atomic file-output policy.

The AV2 v1.0.0 anchors for this contract are:

- § 5.17.12 and § 6.16.13 for decoded-frame-hash metadata and byte conversion;
- § 6.4.1, § 6.17.4.1, and § 6.17.4.4 for profile, bit depth, dimensions, and
  visible crop facts;
- § 7.21.1-§ 7.21.7 for output events, intermediate output, implicit/flush
  output, output frame buffers, and film grain;
- § 7.22 and § 7.23 for the distinction between output frames, motion-field
  storage, and reference-frame update state.

This change is tracked by `DOC-DECODER-OUTPUT-EQUIVALENCE-CONTRACT` and is
documentation/status only. It does not make runtime decode, hashes, raw output,
Y4M output, post-film-grain output, metadata-hash verification, or external
reference evidence supported.

## Goals / Non-Goals

**Goals:**

- Define the two named decoder output variants:
  `raw_intermediate_output` and `post_film_grain_output`.
- Keep `splot-dfh-sha256-v1` tied to `raw_intermediate_output` and define how a
  future post-film-grain hash result must be labeled.
- Pin future output event ordering, including show-existing frames and implicit
  flush output.
- Pin visible crop, chroma plane dimensions, plane order, sample traversal, and
  8-bit/greater-than-8-bit sample-byte serialization.
- Define a future success JSON schema for
  `splot decode --output-format hash --json` that is distinct from current
  diagnostic JSON.
- Define raw/Y4M output contracts and atomic file-output safety requirements.
- Preserve the AVM/dav2d local-reference evidence boundary as metadata only.

**Non-Goals:**

- No runtime decode success path.
- No new `--output-format` value, variant-selection CLI flag, or change to
  current diagnostic JSON.
- No film-grain synthesis implementation.
- No raw/Y4M/hash file emission implementation.
- No `splot-decode -> splot-recon` dependency change.
- No AVM, dav2d, ffmpeg, wrapper, script, CI, or `xtask` integration.

## Decisions

1. **Contract row is supported; runtime rows remain partial or unsupported.**
   The new matrix row records that the output-equivalence contract exists and is
   validated. Existing rows such as `deterministic-frame-hash`,
   `cli-decode-hash-output-contract`, `output-y4m`, and the generated
   `output-film-grain-and-reference-update` coverage row remain honest about
   runtime gaps.

2. **Output variants are labels over AV2 output sample sets, not CLI modes yet.**
   `raw_intermediate_output` names the § 7.21.2 sample set before film grain.
   `post_film_grain_output` names the sample set after § 7.21.7 when film grain
   applies. A no-grain stream may produce identical bytes for both variants, but
   the variant label remains part of the artifact identity.

3. **Hash JSON success output is a separate schema from diagnostic JSON.**
   Current `--json` failures are diagnostic objects. Future hash success output
   uses `contract_id = "splot.decode.hash_report"` and
   `contract_version = 1`, with frames in output-event order and per-frame hash
   entries that name `variant`, `algorithm_id`, `byte_stream_id`, and
   `digest_hex`.

4. **Atomic output belongs at the CLI/file-output boundary.**
   Existing `splot-recon` serializers write to caller-owned writers and may
   leave those writers partially written on I/O failure. Future successful
   `splot decode -o` modes must wrap them in same-directory temporary-file
   staging, complete serialization, flush user-space buffers, sync the temp
   file's contents and metadata, rename only after those steps succeed, and
   attempt best-effort parent-directory sync after rename where supported. Any
   output path creation, temp write, flush, sync, rename, cleanup, or
   serialization failure before the completed rename must become a registered
   `decode/output-error` diagnostic rather than a partial success artifact.
   Pre-rename failures preserve the final path unchanged; unsupported or failed
   parent-directory sync after a successful rename does not convert a complete
   publication into a failed decode.

5. **Reference tools remain evidence only.**
   AVM/dav2d command summaries, tool revisions, fixture hashes, and output
   digests may be recorded as portable metadata. They cannot become build,
   test, CI, `xtask`, runtime, or support-status dependencies.

## Risks / Trade-offs

- Show-existing output can reuse stored samples. Deduplicating by reference slot
  or frame identity would lose a distinct output event. The contract requires a
  fresh output index per emitted event.
- Post-film-grain output can be byte-identical to raw output when grain does not
  apply. The contract still requires distinct variant identity so later
  film-grain support does not silently change artifact meaning.
- Y4M frame-rate and stream metadata are repository output policy, not AV2
  syntax. Runtime Y4M remains unsupported until the future output row pins and
  tests that policy.
- Cross-platform atomic replace behavior can be subtle. This contract requires
  temp-file sync before rename and best-effort parent-directory sync after
  rename for any future successful `-o` claim; the future runtime output change
  owns the helper implementation details and platform support tests. Because
  parent-directory sync happens after rename, unsupported or failed directory
  sync must not turn a completed publication into a failed decode.
- A success hash report could be confused with current diagnostic JSON. The
  schema uses explicit `contract_id`, `contract_version`, and success-only
  fields to keep those surfaces separate.
