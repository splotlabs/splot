# runtime delta: optimize-decode-mc-hot-paths

Adds the motion-compensation hot-path constraints. Non-normative
codec-runtime infrastructure: it adds no AV2 conformance coverage and
changes no decoded output.

## ADDED Requirements

### Requirement: sub-pel fast paths are exact special cases

A § 7.13.3.18 convolution fast path SHALL produce bit-identical output to
the two-pass filter it bypasses, proven either algebraically (the
zero-phase unscaled case reduces to the clipped reference sample scaled by
the residual rounding shift, exact because each partial product is a
multiple of the rounding divisor) or by an equivalence test against an
independent reference implementation.

#### Scenario: zero-phase block matches the general path

- **WHEN** a block predicts with unit steps and both sub-pel phases zero
- **THEN** the fast-path output equals the two-pass convolution output
  sample for sample

### Requirement: reference planes are borrowed, not copied

Motion compensation SHALL read reference-plane samples through a strided
borrowed view over the decoded frame's storage rather than linearizing the
plane into an owned buffer per predicted block.

#### Scenario: predicted block reads the reference in place

- **WHEN** an inter block predicts from a decoded reference frame
- **THEN** its reference reads resolve through a view borrowing the
  reference plane's storage without a per-block plane copy
