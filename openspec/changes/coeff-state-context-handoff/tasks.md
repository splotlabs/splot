## 1. Decoder Implementation

- [x] 1.1 Add a crate-private state-backed ordinary nonzero pass config that uses one plane/geometry source for sign DC context reads and final context commits.
- [x] 1.2 Add a wrapper that reads `AboveDcContext` and `LeftDcContext` from `TileCoeffContextState` before calling the existing derived-base/context-commit path.
- [x] 1.3 Preserve existing explicit-slice ordinary pass APIs for staged tests and later dequant/reconstruction handoff.

## 2. Tests

- [x] 2.1 Add successful read-before-write coverage proving seeded DC contexts affect derived sign reads before final context lines are updated.
- [x] 2.2 Add ordinary-pass failure coverage proving tile context lines are unchanged.
- [x] 2.3 Add invalid context-update geometry coverage proving no partial context-line mutation after the pass succeeds.

## 3. Tracking And Gates

- [x] 3.1 Add `DECODE-COEFF-STATE-CONTEXT-HANDOFF` rows to implementation, decoder-support, conformance coverage, and roadmap docs.
- [x] 3.2 Regenerate feature/spec/support/decoder-coverage status docs.
- [x] 3.3 Run focused tests, OpenSpec validation, feature/support checks, decoder conformance coverage checks, and `cargo xtask ci`.
