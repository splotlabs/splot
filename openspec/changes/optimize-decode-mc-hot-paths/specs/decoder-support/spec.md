## MODIFIED Requirements

### Requirement: Motion-compensation hot paths preserve exact output
The decoder SHALL take an exact fast path for zero-phase unscaled
§ 7.13.3.18 sub-pel predictions (unit steps, both phases zero) that
returns the clipped reference sample scaled by the residual rounding
shift, and SHALL read `u16` reference planes through a strided borrowed
view without per-block widening copies; decoded output SHALL remain
byte-identical to the two-pass convolution over linearized planes.

#### Scenario: Whole-pel motion output is unchanged
- **GIVEN** an admitted inter stream containing whole-pel and zero-MV
  skip-copy blocks
- **WHEN** the stream decodes end to end
- **THEN** the raw output is byte-identical to the AVM oracle and to
  the pre-optimization decoder
