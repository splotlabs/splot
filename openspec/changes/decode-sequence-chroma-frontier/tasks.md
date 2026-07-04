# Tasks

## Runtime Gate

- [x] 1.1 Move the sequence-level CfL/MHCCP rejection out of `validate_sequence`.
- [x] 1.2 Parse the leading key-frame header before the chroma-tool, bit-depth
      storage, and extra-leading-OBU gates so parse-only frame-header frontiers can
      surface before output.
- [x] 1.3 Preserve fail-closed CfL/MHCCP rejection before any tile mode-info symbol
      decode or reconstructed-frame allocation.

## Tests And Tracking

- [x] 2.1 Update the local `local decoder mission` CLI regression to the new key-frame header gate.
- [x] 2.2 Add focused runtime regressions proving CfL/MHCCP still reject at the
      pre-tile boundary and track the new feature row.
- [x] 2.3 Add matrix, decoder-support, and OpenSpec tracking for
      `DECODE-SEQUENCE-CHROMA-FRONTIER`.
- [x] 2.4 Regenerate generated docs and run the required checks.
