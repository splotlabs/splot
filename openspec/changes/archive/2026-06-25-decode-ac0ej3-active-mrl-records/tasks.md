## 1. MRL State And Mode Info
- [x] 1.1 Add tile-local `UsesMrls` state with left/above context helpers and focused unit tests.
- [x] 1.2 Thread `UsesMrls` state through general intra partition tree and leaf return state.
- [x] 1.3 Return MRL metadata from general intra luma/shared mode-info, select MRL CDF contexts from retained neighbours, and keep sample decode paths fail-closed.

## 2. ac0ej3 Runtime Handoff
- [x] 2.1 Wire active MRL metadata through the Wiener NS LR selectable transform-record callback without claiming prediction support.
- [x] 2.2 Probe `ac0ej3.ivf` and update the ignored CLI regression to the next structured frontier.

## 3. Tracking And Verification
- [x] 3.1 Update implementation/support matrices and regenerated decoder support/status docs for the new frontier.
- [x] 3.2 Validate OpenSpec, focused tests, `ac0ej3` probe, conformance corpus, fixtures, and full CI.
