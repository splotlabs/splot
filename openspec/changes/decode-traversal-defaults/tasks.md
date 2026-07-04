## 1. Resource Policy

- [x] 1.1 Raise default OBU traversal and frame/output-frame thresholds enough
  for the current `local-decoder-mission.ivf` mission target while keeping defaults finite.
- [x] 1.2 Preserve explicit low-limit tests so resource-limit diagnostics remain
  covered.
- [x] 1.3 Classify `OBU_REGULAR_TIP` as a planner frame candidate while keeping
  runtime TIP decode unsupported.

## 2. Tracking And Verification

- [x] 2.1 Update decoder support / implementation tracking to describe the
  raised finite defaults without claiming new decode support.
- [x] 2.2 Verify `splot decode` on `local-decoder-mission.ivf` advances past the old
  `max_frames_to_decode = 128` policy gate and the planner TIP gate to the next
  honest runtime gate.
- [x] 2.3 Run focused tests plus the required repository gates for this brick.
