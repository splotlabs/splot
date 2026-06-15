## ADDED Requirements

### Requirement: Decoder output equivalence contract

The decoder support model SHALL document a decoder output equivalence contract
tracked by Feature ID `DOC-DECODER-OUTPUT-EQUIVALENCE-CONTRACT`. The contract
SHALL define future runtime output identity for `splot decode` without claiming
runtime decode support. It SHALL cite AV2 v1.0.0 § 5.17.12 and § 6.16.13 for
decoded-frame-hash metadata, § 6.4.1, § 6.17.4.1, and § 6.17.4.4 for output
format and crop-derived geometry, § 7.21.1 through § 7.21.7 for output events,
intermediate output, implicit output, flush output, output frame buffers, and
film grain, and § 7.22-§ 7.23 for the distinction between output events and
reference-frame state. The contract SHALL keep runtime decode, runtime hash
output, raw output, Y4M output, film-grain synthesis, metadata-hash
verification, and external reference-tool integration unsupported until later
source-backed changes provide implementation and tests.

#### Scenario: Output variants are named

- **WHEN** a reader checks the decoder output equivalence contract
- **THEN** it defines `raw_intermediate_output` as the § 7.21.2 intermediate
  output sample set before film grain synthesis
- **AND** it defines `post_film_grain_output` as the sample set after the
  § 7.21.7 film-grain synthesis process when that process applies
- **AND** it states that no-grain streams may produce identical sample bytes for
  both variants while the variant identifier remains part of artifact identity

#### Scenario: Raw intermediate hash contract remains stable

- **WHEN** a reader checks the hash identity for `raw_intermediate_output`
- **THEN** the contract keeps `splot-dfh-sha256-v1` over the
  `av2-output-samples-v1` byte stream as the stable raw intermediate hash
- **AND** it requires hash results to name the output variant, algorithm
  identifier, and byte-stream identifier
- **AND** it states that any future post-film-grain hash result MUST carry the
  `post_film_grain_output` variant identifier and cannot be supported before
  film-grain synthesis is implemented and tested

#### Scenario: Output event order is pinned

- **WHEN** future runtime decode emits output frames
- **THEN** output indices are assigned in AV2 output-process order after the
  selected operating point or layer is applied
- **AND** show-existing output events receive distinct output indices even when
  they reuse stored frame samples
- **AND** a show-existing output reached through `output_process(-1)` with
  `ShowExistingFrame == 1` does not mark the referenced frame as already output
  for later implicit-output or flush eligibility
- **AND** implicit output and flush events are appended according to § 7.21.4
  and § 7.21.5
- **AND** output ordering MUST NOT depend on decode order, OBU order, reference
  slot index, hash completion order, file-write completion order, or worker
  completion order

#### Scenario: Visible sample bytes are canonical

- **WHEN** future runtime output serializes hash, raw, or Y4M sample payloads
- **THEN** luma output uses the visible `w` by `h` sample rectangle produced by
  the AV2 output process
- **AND** non-monochrome chroma output uses
  `((w + subX) >> subX)` by `((h + subY) >> subY)` samples for U and V
- **AND** monochrome output omits U and V planes
- **AND** sample traversal is Y, then U, then V, in raster order within each
  present plane
- **AND** 8-bit output samples serialize as one byte and greater-than-8-bit
  output samples serialize as two little-endian bytes
- **AND** stride padding, backing allocation padding, reference-store metadata,
  OBU bytes, container bytes, and decoded-frame-hash metadata are excluded from
  the sample byte stream

#### Scenario: Hash JSON success schema is separate from diagnostics

- **WHEN** future `splot decode --output-format hash --json` succeeds
- **THEN** stdout is a success artifact with
  `contract_id = "splot.decode.hash_report"` and `contract_version = 1`
- **AND** it contains the selected output variant or variants, the selected
  thread policy, and an array of frames sorted by output index
- **AND** each frame entry records output index, visible luma crop origin,
  visible luma dimensions, chroma crop origin and dimensions when present, bit
  depth, pixel format, and one or more hash entries with `variant`,
  `algorithm_id`, `byte_stream_id`, and 64-character lowercase hexadecimal
  `digest_hex`
- **AND** monochrome frame entries omit chroma origin and dimension fields
- **AND** failure paths continue to emit decoder diagnostic JSON instead of a
  partial hash report

#### Scenario: Raw and Y4M output contracts are distinct

- **WHEN** future runtime raw output is implemented
- **THEN** raw output is defined as concatenated canonical sample bytes for each
  output event in output-index order for the selected variant, with no header or
  metadata bytes
- **AND** this contract does not add a current `--output-format raw` CLI mode
- **WHEN** future runtime Y4M output is implemented
- **THEN** Y4M output represents the AV2 output-frame sample set for the chosen
  variant, using repository-owned Y4M container policy
- **AND** Y4M container bytes remain repository output policy rather than AV2
  syntax

#### Scenario: Output-file publication is atomic

- **WHEN** a future successful `splot decode -o <path>` mode writes hash, raw,
  or Y4M output
- **THEN** it writes to a temporary file in the final path's directory,
  completes serialization, flushes user-space buffers, syncs the temporary
  file's contents and metadata, renames the temporary file as the final publish
  step, and attempts best-effort parent-directory sync after rename where the
  platform supports it
- **AND** if decode, reconstruction, hash serialization, raw/Y4M serialization,
  validation, temporary-file write, flush, temporary-file sync, rename, or any
  other pre-rename publication step fails, an absent final path remains absent
  and an existing final path remains byte-for-byte unchanged
- **AND** if rename succeeds, unsupported or failed parent-directory sync does
  not convert the completed publication into a failed decode, and the final path
  MUST NOT contain a partially serialized payload
- **AND** output path creation, temporary-file write, flush, sync, rename,
  cleanup, or serialization failures before the completed rename are emitted as
  a registered `decode/output-error` diagnostic rather than as partial success
  artifacts
- **AND** output-derived counts and byte sizes are computed with checked
  arithmetic and checked against `DecodeLimits` before allocation, indexing, or
  output publication

#### Scenario: Metadata hashes remain separate

- **WHEN** decoded-frame-hash metadata is present in a future supported stream
- **THEN** metadata verification uses the AV2 § 5.17.12 and § 6.16.13 metadata
  contract for conformance checking
- **AND** metadata verification is reported separately from repository
  `splot-dfh-sha256-v1` success artifacts
- **AND** the decoder support matrix does not treat AVM/dav2d raw MD5 metadata
  or decoded-frame-hash metadata verification as proof of repository SHA-256
  runtime output support

#### Scenario: Reference tools remain metadata only

- **WHEN** local AVM or dav2d evidence is recorded for output equivalence
- **THEN** committed evidence is portable metadata such as tool name, revision,
  sanitized command summary, input hash, output hash, date, and agreement notes
- **AND** the repository does not add AVM/dav2d source, binaries, submodules,
  dependencies, wrappers, setup scripts, Docker images, caches, CI jobs, runtime
  process execution, or `xtask` commands that locate, build, invoke, or require
  AVM or dav2d
