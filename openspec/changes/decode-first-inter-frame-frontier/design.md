## Context

The decoder has no inter support. The first verifiable inter step must be the
smallest stream whose inter frame can be checked bit-exact against both oracles
(avmdec and dav2d) and whose decode reduces to a well-understood spec primitive.

## Decisions

### The verified target: a zero-MV skip inter frame over identical content

The fixture is encoded from a project-owned flat synthetic Y4M whose two frames
are byte-identical (luma 100, U 120, V 130). With broad decode tools disabled and
`--enable-global-motion=1 --qp=80 --sb-size=64 --min/max-partition-size=64`, the
encoder codes:

- Frame 0: a single flat 64x64 intra DC key block (POC 0, KEY).
- Frame 1: a single 64x64 inter block (POC 1, INTER), single reference, zero MV,
  skip=1, no residual.

Because the MV is zero, AV2 § 7.13.3.18 motion compensation reduces to a straight
sample copy of the co-located reference block ("if the fractional part is zero,
the filtering is equivalent to a straight sample copy"). Frame 1 therefore
decodes bit-exact to a copy of frame 0. Both oracles agree byte-for-byte
(decoded-output md5 `4e1bd39f0b541ef1f479cff049e6985c`, 12288 bytes), confirmed
by regenerating the fixture and decoding with both tools.

### Implicit reference map (PRIMARY_REF_CHOOSE), not the explicit map

The fixture must be decodable by BOTH avmdec and dav2d. Encoding with
`--explicit-ref-frame-map=1` produces a stream where dav2d's raw output diverges
from avmdec's, so it is unusable as a dual-oracle target. The dav2d-compatible
encode uses `signal_primary_ref_frame = 0` (`primary_ref_frame =
PRIMARY_REF_CHOOSE`) and the IMPLICIT reference map. The splot-core § 5.18.2 inter
frame-header parser models the control region but stops at
`InterStop::UnmodeledDerivation` exactly at the implicit-map branch, because
`get_ref_frames()` (§ 7.7) is not yet modeled. This is the empirically-confirmed
next blocker for the inter decode slice (verified by parsing the real fixture
bytes through `parse_frame_header_core`: it stops after 18 bits with
`primary_ref_frame = Some(8)` / PRIMARY_REF_CHOOSE).

### Land the verified target now; defer the decode slice

This change commits the verified fixture and pins the honest rejection so the
target survives and the next session starts from a known-good artifact with a
precise blocker. It does NOT relax the planner to a half-supported state (which
would make `splot decode` emit frame 0 then error on frame 1) and it does NOT
fabricate an inter decode. The full inter decode slice is enumerated in
`tasks.md` § 4 and gated behind modeling `get_ref_frames()`.

## Honesty

- The only behaviour pinned in code is the existing, correct rejection of the
  inter OBU at the stream planner.
- The decoded-output md5 and the avm/dav2d agreement are recorded in
  `docs/LOCAL-REFERENCE-EVIDENCE.toml` as locally-verified reference evidence
  (avmdec == dav2d), not as a splot decode result.
- No deferred reject is claimed as fixture-verified beyond the planner rejection
  this change actually tests.

## Out of scope

The full inter decode slice (multi-frame planner + runtime loop, § 7.7
`get_ref_frames()`, the § 5.18.2 inter frame-header shared tail, § 7.23 reference
retention, § 5.20 inter mode_info, § 7.11 zero-MV derivation, § 7.13.3.18 copy,
frame-1 output), compound / nonzero-MV / non-skip / sub-pel inter, in-loop
filters, and live in-CI AVM/dav2d invocation.
