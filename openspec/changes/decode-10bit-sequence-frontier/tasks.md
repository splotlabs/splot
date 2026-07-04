# Tasks

## Runtime Gate

- [x] 1.1 Move the 8-bit-only runtime storage check out of sequence validation and into the pre-decode runtime boundary.
- [x] 1.2 Preserve fail-closed rejection for otherwise reachable 10-bit streams before any `DecodedFrame<u8>` reconstruction or output.
- [x] 1.3 Keep existing profile, chroma, layer, crop, and unsupported sequence-tool validation unchanged.

## Tests And Tracking

- [x] 2.1 Update the local `local decoder mission` CLI regression to the new next gate.
- [x] 2.2 Add or update 10-bit sequence regressions proving the next parsed gate and the 8-bit runtime storage guard.
- [x] 2.3 Add matrix and OpenSpec tracking for `DECODE-10BIT-SEQUENCE-FRONTIER`.
- [x] 2.4 Regenerate generated docs and run the required checks.
