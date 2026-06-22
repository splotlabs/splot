## Why

The inter decoder decodes multi-block, multi-superblock, and 2-D-grid inter frames
whose later blocks predict a neighbour's motion vector through the AV2 § 7.12.2
`find_mv_stack` spatial scan. But EVERY committed inter fixture
(`syn-2frame-inter-mvstack-64x64`, `syn-2sb-inter-128x64-q80`,
`syn-grid-inter-128x128-q80`) propagates ONE identical motion vector (block 0 /
SB0 NEWMV col 48; every later block NEARMV reusing col 48). Because all candidate
MVs are equal, the § 7.12.2 stack collapses to a single entry, so the per-neighbour
scan-point ORDERING (which neighbour wins — left vs above precedence) and the
§ 5.20.7.8 DRL slot selection are EXERCISED-BUT-NOT-DISCRIMINATED: a wrong order
among identical-value candidates decodes identically, and § 8.2.4 `exit_symbol()`
is bit-count-only, so it cannot catch the mismatch either. This is a verified-subset
honesty gap recorded against `DECODE-INTER-MVSTACK-SPATIAL` /
`DECODE-INTER-GRID-SPATIAL`.

The smallest bit-exact-verifiable closure is a two-frame 64x64 stream whose inter
superblock is § 5.20.3 SPLIT into four 32x32 leaves, each carrying a DISTINCT
motion vector, with an interior NEARMV block whose left and above neighbours hold
DIFFERENT MVs and whose decoded RefMvIdx selects a SPECIFIC slot — so the
reconstructed MV reveals which neighbour/slot the stack chose. Both oracles agree
byte-for-byte.

Investigation result: `find_mv_stack`'s § 7.12.2.6 ordered spatial scan and the
§ 7.12.2.12 search-stack dedupe were ALREADY CORRECT — the distinct-MV oracle
match succeeds with no decoder change. This brick is a fixture + tests + honesty
brick that PINS the ordering. Falsifiability was verified locally by temporarily
swapping the scan steps, which flips the stack slots and changes the decoded
hash (failing both the unit test and the bit-exact decode test).

Because every leaf is exactly 32x32 (`Block_Width` / `Block_Height` == 32, NOT
> 32), the § 7.12.2.20 large-block (> 32x32) extra MVP combinations are correctly
INAPPLICABLE, so this brick pins the spatial ordering WITHOUT implementing
§ 7.12.2.20 (the preferred, smaller brick).

## What Changes

- Add Feature ID `DECODE-INTER-MVORDER-SPATIAL`.
- NO decoder code change: the existing `find_mv_stack` § 7.12.2.6 spatial scan
  order (step 7 left = `scan_point(bh4 - 1, -1)`, step 8 above =
  `scan_point(-1, bw4 - 1)`, …) and § 7.12.2.12 search-stack dedupe already match
  the spec and both oracles. The brick adds a committed distinct-MV fixture plus
  tests and updates the now-inaccurate deferral comments in `find_mv_stack.rs`.
- Add the project-owned `syn-2frame-inter-mvorder-64x64.ivf` fixture (frame 0 =
  four flat 32x32 DC_PRED intra quadrants 100/150/60/200; frame 1 = the 64x64
  superblock SPLIT into four 32x32 single-reference inter blocks, all skip=1, with
  DISTINCT MVs: block 0 NEWMV col 64, block 1 NEWMV col -32, block 2 NEWMV col 32,
  and the interior block 3 NEARMV RefMvIdx 1 reconstructing col -32 — the ABOVE
  neighbour — over a stack whose slot 0 is the LEFT neighbour col 32). Prove
  avmdec `--rawvideo --i420` and dav2d `--demuxer ivf` agree byte-for-byte (md5
  `284e1450b42180f02de7415ab0367bfe`, 12288 bytes).
- Register the fixture in the conformance manifest (`expect = "clean"`) and add
  the reciprocal LOCAL-REFERENCE-EVIDENCE entry.
- Add decode tests pinning the bit-exact output (per-frame hash + the CLI raw
  output round-trip) and a `find_mv_stack` unit test that asserts the
  distinct-MV stack ORDER (slot 0 = the left neighbour, slot 1 = the above
  neighbour) — the in-repo falsifiability proof.
- Update the `find_mv_stack.rs` module / call-site comments: the per-neighbour
  ordering is now PROVEN by a distinct-MV fixture; the § 7.12.2.20 large-block step
  is correctly INAPPLICABLE to the 32x32 leaves the fixture uses (so the ordering
  is pinned without it) and remains deferred for the > 32x32 leaves it does not
  yet model.

## Capabilities

### New Capabilities
- `decode-inter-mvorder-spatial`: A distinct-neighbour-MV inter frame decodes
  bit-exact, so the § 7.12.2 spatial scan-point ORDERING (left-before-above
  precedence) and the § 5.20.7.8 DRL slot selection are falsifiably pinned: an
  interior NEARMV block whose left and above neighbours hold DIFFERENT motion
  vectors reconstructs the slot-1 (above) candidate, which a reversed order would
  get wrong.

### Modified Capabilities
- `decoder-support`: Track the new partial decoder-support row.

## Impact

- Adds `tests/conformance/vectors/valid/syn-2frame-inter-mvorder-64x64.ivf` and
  decode tests in `crates/splot-decode/src/runtime_minimal/inter/tests.rs`,
  `crates/splot-decode/src/runtime_minimal/inter/find_mv_stack/tests.rs`, and
  `crates/splot-cli/tests/decode_cli.rs`.
- Changes only the comments in
  `crates/splot-decode/src/runtime_minimal/inter/find_mv_stack.rs` (the ordering
  is now proven; § 7.12.2.20 is inapplicable to the 32x32 leaves). No runtime
  decode logic changes.
- Updates `docs/IMPLEMENTATION-MATRIX.toml`, `docs/DECODER-SUPPORT-MATRIX.toml`,
  `docs/LOCAL-REFERENCE-EVIDENCE.toml`, `tests/conformance/manifest.toml`, and the
  generated status/coverage docs.
- No public API, dependency graph, encoder, or validator changes. The § 7.12.2.20
  large-block (> 32x32) MVP combinations, the temporal / compound / warp /
  ref-MV-bank / derived-SMVP / DRL-reorder / scan-col candidates, and a
  multi-superblock skip == 0 residual remain out of scope (rejected before output).
- Scope of what the fixture PROVES (honest): it pins the per-neighbour spatial
  scan ORDERING (left-before-above) and the § 5.20.7.8 DRL slot-1 selection for
  32x32 leaves. It does NOT exercise § 7.12.2.20 (the leaves are exactly 32x32,
  not > 32x32, so that step is inapplicable) and it does not lift any frame-size,
  residual, or tool gate.
