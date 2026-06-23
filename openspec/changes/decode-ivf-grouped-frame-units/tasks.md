## 1. Runtime Shape

- [x] 1.1 Stop requiring IVF record count to equal decoded frame-candidate count.
- [x] 1.2 Resolve following inter candidates by planned OBU offset inside parsed
  IVF payloads.
- [x] 1.3 Keep the verified subset limited to a temporal delimiter immediately
  preceding each following `OBU_REGULAR_TILE_GROUP` candidate.

## 2. Tests And Tracking

- [x] 2.1 Add a repacked-IVF regression test using the committed multiref fixture
  bytes.
- [x] 2.2 Update implementation / decoder-support tracking to state that IVF
  records are container groups and may hold multiple verified frame units.
- [x] 2.3 Add and validate an OpenSpec requirement delta for grouped IVF frame
  units in the verified multiref runtime subset.
- [x] 2.4 Run focused tests and the required repository gates.
