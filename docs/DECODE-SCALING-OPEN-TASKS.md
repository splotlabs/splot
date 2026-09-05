# Decode Scaling Open Tasks

Feature IDs: `INFRA-DECODE-PARALLEL-STAGES`, `INFRA-DECODE-FRAME-PIPELINING`

Historical measurements, retained changes, and rejected experiments are recorded
once in the [historical experiment ledger](DECODE-SCALING-MISSION.md#historical-evidence). Its old
benchmark numbers and retention states describe the measured revisions.

## Remaining work

- **SCALE-010:** Continue reducing decoder CPU work toward the performance goal.
  Re-profile the current revision before selecting another candidate. Preserve
  exact output and retain only measured wins; remeasure survivors independently
  and together. SCALE-081 through SCALE-089 and SCALE-121 through SCALE-133 record
  the previous accepted and rejected candidates in that historical ledger.
- **SCALE-011-I:** The earlier architecture campaign was paused. Its historical
  state is retained in the mission ledger; reassess the current bottleneck before
  resuming any of its proposed designs.
