## 1. Tracking

- [x] 1.1 Add `DECODE-INACTIVE-LR-UNITS-FRONTIER` to the implementation matrix.
- [x] 1.2 Add decoder support/status coverage for `inactive-lr-units-frontier`.
- [x] 1.3 Update roadmap/support notes that name the current local decoder mission diagnostic.

## 2. Implementation

- [x] 2.1 Extend `TileLoopRestorationRootFrontier` to report consumed and active Wiener NS LR units.
- [x] 2.2 Preserve transactional CDF and resource-limit behavior while collecting LR activity.
- [x] 2.3 Update the minimal runtime so inactive LR units advance and active LR units remain unsupported.

## 3. Verification

- [x] 3.1 Add focused inactive-unit, active-unit, and rejection-path tests for the LR frontier summary.
- [x] 3.2 Update the local ignored local decoder mission CLI regression to the new current diagnostic.
- [x] 3.3 Run `openspec validate decode-inactive-lr-units-frontier --no-interactive`, feature/support checks, conformance, and `cargo xtask ci`.
