## Context

`splot decode` currently reads bounded input bytes, builds a
`DecodeContext`, runs `DecodeContext::plan_bytes`, and always renders a
diagnostic. A planable minimal closed-loop-key stream exits with
`decode/unsupported-feature` rather than a success artifact. The full decoder
mission needs one real runtime success path so later tile/CDF/reconstruction
work can be measured against a stable CLI output contract.

The repository already has the prerequisites for a narrow first path:

- byte and parsed stream planners in `splot-decode`;
- crate-private minimal tile-payload boundary derivation in `splot-decode`;
- decoded frame/plane, workspace, intra primitive, and hash input APIs in
  `splot-recon`;
- a docs-only output-equivalence contract for `splot.decode.hash_report`.

The path must remain honest: a minimal fixture hash success is not full AV2
decoder conformance, not Annex A conformance, and not Y4M/raw output support.

## Verified Fixture And Tier Gates

The first candidate fixture is
`tests/conformance/vectors/valid/syn-key-intra-64x64.ivf`. Current evidence:

- `tests/conformance/manifest.toml` classifies it as a clean valid vector from
  project-owned synthetic input.
- `splot inspect --json` reports IVF/DKIF wrapping, AV02, 64x64, one declared
  frame, three OBUs, base-layer `OBU_SEQUENCE_HEADER`, and one base-layer
  `OBU_CLOSED_LOOP_KEY` frame candidate.
- The parsed sequence header reports `seq_profile_idc = 0`, `chroma_format_idc
  = 0`, `bit_depth = 8`, `max_tlayer_id = 0`, and `max_mlayer_id = 0`.
- The parsed frame header reports key/intra output, `cur_mfh_id = 0`,
  `show_existing_frame = false`, `immediate_output_frame = true`,
  `implicit_output_frame = false`, 64x64 frame size, one tile row, one tile
  column, one tile group, and `apply_grain = false`.
- Local AVM and dav2d development runs produced byte-identical 6144-byte raw
  8-bit 4:2:0 output. The raw MD5 matches the existing portable evidence row
  (`f2d45ae552bebe211f3156daf0a7fcf6`), and the corresponding repository
  SHA-256 sample hash is
  `052e33144ff95552a005db19589110651a814470540b0b708e30f7de4c99f496`.

That fixture is not automatically a trivial black-frame fixture: current
inspection also reports active frame-tool fields such as CDEF/CCSO-related
state. The implementation must either support every output-affecting path
required by the selected fixture, or replace the target with a narrower
source-backed fixture and commit the matching manifest/evidence metadata before
claiming support.

The smallest honest path for this fixture starts at the existing § 5.20.1 tile
framing boundary and still requires § 5.20.2 `decode_tile()` work: partition and
block syntax, the CDF selections actually exercised by the fixture, coefficient
and residual reconstruction, CDEF, U-plane CCSO if tile flags activate it, and
immediate output. Before hard-coding any branch subset, implementation must
obtain a source-backed tile/block/symbol trace or create a fixture whose active
paths are already fully understood.

### Source-backed trace result

A temporary local AVM `extract_proto` build produced a text trace for
`syn-key-intra-64x64.ivf`. The trace is local evidence only and is not committed
or required by the repository. It confirms the candidate fixture is too broad
for the first minimal runtime success PR:

- six coding units: four 32x16 blocks and two 32x32 blocks;
- active IntraBC in three coding units, including MV symbol reads;
- active CFL chroma prediction, CDEF, CCSO, FSC/MRL-related symbols, and
  transform/coefficient decoding;
- nonzero residual symbols and transform sizes spanning 32x16, 32x32, and
  64x32-shaped transform enums;
- 53 recorded symbol reads across partition, intra mode, skip transform,
  coefficient, CDEF/CCSO, CFL, CCTX, and IntraBC/MV families.

Therefore this fixture must not be the first runtime hash target unless the
change implements and tests those output-affecting paths. The implementation
should replace it with a narrower source-backed fixture whose trace excludes
IntraBC, CFL, CDEF, CCSO, and other broad decode paths, or deliberately expand
the change scope and matrix proof before claiming support.

### Replacement minimal fixture

The replacement runtime target is
`tests/conformance/vectors/valid/syn-flat-intra-64x64-minimal.ivf`. It was
generated locally from one project-owned flat 64x64 8-bit 4:2:0 I420 frame, with
all Y, U, and V samples set to 128. The sanitized AVM encoder recipe used AV2
IVF output, one input frame, `--lag-in-frames=0`, constant quality `--qp=255`,
64x64 dimensions, 8-bit I420 input, 64x64 partition bounds, 64x64 superblocks,
and disabled CDEF, CCSO, CFL intra, IntraBC, FSC, MRLS, MHCCP, CCTX, GDF,
deblocking, restoration, delta-q, TCQ, and transform partitioning.

Repository facts for the replacement fixture:

- IVF byte length: 66 bytes.
- Fixture SHA-256:
  `82ed37f2586ad9262185304911718322d74633698850a70e75033a67d734f5e0`.
- `splot validate --json` reports an empty diagnostics array.
- `splot inspect --json` reports one declared IVF frame, three OBUs, one base
  sequence header, one closed-loop-key output frame, one tile group, one tile,
  and a two-byte tile payload.
- Frame header facts stay inside the intended tier: profile 0, 8-bit 4:2:0,
  base layer only, inline frame header, 64x64 output, no film grain, no
  deblocking, no GDF, no CDEF, no LR, no qmatrix, no delta-q, no segmentation,
  and no crop.

Local reference decoders produced raw 6144-byte output identical to the input
I420 frame. The raw output MD5 is `9604569c8e5fcd812a940b82ef39b552`; the raw
output SHA-256, matching the future `raw_intermediate_output` sample stream for
this fixture, is
`cb11e05cb5da949c0e0f5b5a7cb310df35a96a22c45d1ada70d950859fe697d1`.

The replacement fixture's temporary AVM `extract_proto` trace contains one
64x64 superblock, one shared 64x64 coding unit, one chroma coding unit, skipped
luma/chroma transforms, and six symbol families: partition `do_split`, intra
luma mode set/index, all-zero luma-or-U transform flags, UV mode index, and the
all-zero V transform flag. The trace contains no IntraBC, CFL alpha, CDEF, CCSO,
FSC, MRL, MHCCP, CCTX, coefficient, or MV-read symbols. This is the smallest
source-backed path for the first runtime hash implementation.

The implementation verifies this fixture through the existing §8.2
`SymbolDecoder` and generated §9.3 default CDF rows before constructing output.
The fixed trace consumes `Default_Do_Split_Cdf[0][0]`,
`Default_Y_Mode_Set_Cdf`, `Default_Y_Mode_Index_Cdf[0]`,
`Default_Txb_Skip_Cdf[2][0][0][0]`,
`Default_Uv_Mode_Cfl_Not_Allowed_Cdf[0]`, and
`Default_V_Txb_Skip_Cdf[1][3]`, then requires §8.2.4 `exit_symbol()` to end at
the two-byte tile boundary. This is still a fixture-trace path, not generic
recursive `decode_tile()` / `read_partition()` support.

## Goals / Non-Goals

**Goals:**

- Make one committed minimal-tier intra IVF fixture verify its traced tile
  symbol stream and emit `splot decode --output-format hash --json` with exit
  code 0.
- Emit `contract_id = "splot.decode.hash_report"` and
  `contract_version = 1` success JSON for the supported path.
- Compute `splot-dfh-sha256-v1` over the decoded
  `raw_intermediate_output` visible sample byte stream.
- Keep hash mode output-path no-touch semantics: no create, truncate, write,
  rename, or cleanup action against the optional `-o` path.
- Prove deterministic frame hashes and output order across `--threads 1`,
  `--threads auto`, and selected fixed `--threads N`.
- Preserve structured non-success diagnostics for malformed sources, local
  resource-limit failures, and valid but out-of-tier streams.
- Record local AVM/dav2d evidence only as portable metadata checked offline.
- Update the implementation matrix, decoder support matrix, generated status,
  and docs without moving broad partial rows to supported.

**Non-Goals:**

- Full AV2 decoder conformance or Annex A level/tier conformance.
- Runtime Y4M or raw file output.
- Film-grain/post-film-grain output, decoded-frame-hash metadata verification,
  or metadata MD5 interop.
- Full tile syntax traversal, full CDF lifecycle, broad intra/inter prediction, MFH,
  operating points, external HLS, multistream, multi-layer, random access,
  reference refresh completeness, loop filtering, or decoder-model checks.
- Any checked-in AVM/dav2d source, binary, wrapper, setup script, CI job,
  cache, runtime subprocess invocation, or required local setup.

## Decisions

1. Add a narrow hash runtime entry point in `splot-decode`.

   `DecodeContext` should expose a runtime method such as
   `decode_hash_report_bytes(bytes, options)`. The method first reuses
   `plan_bytes` so current malformed-source, resource-limit, layer-selection,
   and unsupported-structure behavior stays transactional and deterministic.
   The CLI should dispatch to this API and only render success JSON or the
   existing diagnostic report shapes; it must not parse tile payloads or build
   frames itself.

   Alternative considered: implement the success path directly in
   `splot-cli`. That would violate the library-first boundary and make future
   runtime decode unusable from Rust APIs.

2. Keep the accepted tier explicit and fail closed.

   The runtime path should accept only the `minimal-intra-8bit420-hash-v1`
   fixture-trace tier once source facts prove it: base-layer selection, one
   sequence header, one closed-loop-key frame candidate, inline frame header,
   8-bit 4:2:0 Main profile, no crop, no film grain, one tile group, one tile,
   the traced six-symbol §8.2 stream, and no external HLS/MFH/multistream/OPS
   dependency. Anything outside that tier returns
   `decode/unsupported-feature` with stable matrix/feature metadata rather than
   guessing a decode.

   Alternative considered: synthesize a frame for any planable closed-loop-key
   OBU. That would be a false conformance claim because the bytes would not be
   derived from the AV2 tile and reconstruction process.

3. Use real decoded frame/hash primitives for the success artifact.

   The implementation should build a `splot-recon::DecodedFrame`, compute
   `DecodedFrameHashInput::compute_hash()`, and serialize the report described
   by `docs/DECODER-FULL-CONFORMANCE.md` after the traced no-residual/no-filter
   tile symbol stream is verified. The all-flat frame construction is a narrow
   reconstruction handoff for this fixture only, not a generic intra prediction
   or transform path. This likely requires adding the approved future internal
   dependency `splot-decode -> splot-recon` when the implementation starts. That
   dependency edge is already described as the approved future direction for
   runtime decode, but the implementation must still update dependency-direction
   evidence and call out the graph change in review/PR notes.

   Alternative considered: duplicate hash serialization inside `splot-decode`.
   That would drift from the existing `splot-recon` hash contract.

4. Hash mode remains stdout-first and no-touch for `-o`.

   This change should support success JSON on stdout for `--output-format hash`.
   Even if `-o` is supplied with hash mode, the CLI must not open or publish the
   path in this phase. Atomic file publication remains required before future
   successful file-output modes, but it is not necessary for stdout-only hash
   success.

   Alternative considered: write hash JSON to `-o`. That would introduce file
   output semantics before atomic publication is implemented and would
   complicate the first runtime success path.

5. Determinism is measured on frame/output data, not byte-identical JSON across
   different thread policies.

   The hash report records the selected thread policy. Therefore JSON documents
   may differ in that metadata field across `--threads 1`, `auto`, and fixed
   `N`; the required invariant is identical output indices, frame geometry,
   variants, byte-stream identifiers, and digest values.

## Risks / Trade-offs

- [Risk] Minimal fixture bytes do not match the documented
  `minimal-intra-8bit420-hash-v1` tier. -> Verify the fixture before claiming
  support; if needed, commit a true tiny intra IVF fixture with manifest and
  evidence metadata.
- [Risk] The selected fixture needs output-affecting CDEF/CCSO or residual
  paths that are larger than the intended first PR. -> Capture a source-backed
  trace first, or switch to a narrower fixture before claiming runtime success.
- [Risk] The first tile/reconstruction subset accidentally overclaims broad
  tile/CDF/intra support. -> Add a narrow matrix row and leave broad rows
  partial unless their full row scope is implemented and tested.
- [Risk] Runtime allocations bypass `DecodeLimits`. -> Check frame dimensions,
  luma samples, decoded-frame bytes, output frame count, output byte count, tile
  count, and tile payload bytes with checked arithmetic before allocation,
  indexing, or hash report construction.
- [Risk] Existing diagnostics regress when success support is added. -> Keep
  negative CLI and library tests for malformed, unsupported, and resource-limit
  paths, and assert failures emit diagnostic JSON rather than partial hash
  reports.
- [Risk] Local reference metadata is mistaken for executable integration. ->
  Keep evidence in `docs/LOCAL-REFERENCE-EVIDENCE.toml` only and rely on the
  existing offline checker; do not add wrappers or commands.

## Fuzz Target Decision

This change does not add a new runtime fuzz target. The only broadly
byte-consuming traversal remains `DecodeContext::plan_bytes`, which is already
covered by the `decode_plan_bytes` fuzz target. The new runtime hash path first
reuses that planner, then accepts only the committed `minimal-intra-8bit420-
hash-v1` IVF shape and fails closed on any shape mismatch before allocation or
hash report construction. Its additional byte reads reuse existing `splot-core`
sequence/frame/tile-group parsers that are covered by parser tests and fuzz
targets; a runtime fuzz target for this PR would mostly exercise the same
closed rejection gates rather than a broader decode surface. Add a dedicated
runtime fuzz target when the next slice introduces data-dependent tile syntax
traversal beyond the exact traced payload gate.
