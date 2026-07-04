## Why

The `local-decoder-mission.ivf` mission target stores more than one AV2 coded frame unit inside
some IVF frame records. The minimal multi-frame runtime currently assumes that
IVF record index and decoded frame-candidate index are the same thing, so it can
only handle the committed fixture shape where every following inter frame lives
in its own IVF record.

IVF is a non-normative byte envelope; AV2 frame-unit order is defined by the OBU
stream. The runtime should therefore resolve verified inter candidates by their
planned OBU offsets, while keeping the same strict verified decode subset.

## What Changes

- Keep the existing three-frame verified runtime cap.
- Keep the existing `[TD, SEQ, CLK] + [TD, OBU_REGULAR_TILE_GROUP]...` OBU-order
  subset.
- Relax only the IVF record grouping assumption: an IVF record may contain more
  than one `[TD, OBU_REGULAR_TILE_GROUP]` pair.
- Add a regression test that repacks the already oracle-proven
  `syn-3frame-multiref-64x64.ivf` OBU payloads into two IVF records while keeping
  the Annex B OBU bytes unchanged, then proves the decoded frames match the
  committed fixture.

## Impact

- Touches the minimal decode runtime and its tests.
- No new AV2 syntax support, no new prediction mode, no frame-count cap increase,
  no dependency graph change, and no claim of broad `local decoder mission` decode support.
