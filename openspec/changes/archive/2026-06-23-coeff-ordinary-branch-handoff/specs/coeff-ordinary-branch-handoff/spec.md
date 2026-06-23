## ADDED Requirements

### Requirement: Compose ordinary coefficient branches after all-zero selection
The decoder coefficient-loop boundary SHALL provide a crate-private ordinary
branch handoff for AV2 § 5.20.7.27 after the caller has decoded `all_zero`.
When the caller selects the all-zero branch, the handoff SHALL apply the
all-zero coefficient-block state effects through the existing branch path.
When the caller selects the nonzero branch, the handoff SHALL initialize the
nonzero EOB start through the existing branch path and then run the
state-backed ordinary non-FSC coefficient pass. The handoff SHALL return typed
errors and SHALL NOT derive transform-block syntax facts, dequantize,
reconstruct, update output, refresh references, invoke AVM/dav2d, or expose a
public API.

#### Scenario: All-zero branch preserves existing runtime behavior
- **WHEN** the minimal all-zero trace selects the ordinary branch handoff's
  all-zero arm with caller-resolved transform geometry
- **THEN** the handoff applies the same all-zero coefficient-block state effects
  as the existing EOB branch helper
- **AND** no coefficient symbols beyond the already-decoded `all_zero` decision
  are consumed

#### Scenario: Nonzero branch reaches the state-backed ordinary pass
- **WHEN** the caller selects the nonzero arm with valid EOB, scan,
  derived-base, state-context, TCQ, and lossless facts
- **THEN** the handoff reads the nonzero EOB syntax, walks the scan, derives and
  reads ordinary base symbols, derives sign sources from tile DC context state,
  reads sign and `read_quant` syntax, writes signed `Quant[]`, and commits final
  above/left level and DC context lines

#### Scenario: Invalid nonzero start preserves mutable state before reads
- **WHEN** the caller selects the nonzero arm with invalid EOB selector facts
- **THEN** the handoff returns the typed EOB context error
- **AND** coefficient context state, tile CDF rows, and symbol-decoder counters
  remain unchanged

#### Scenario: Ordinary-pass failure preserves coefficient context state
- **WHEN** the nonzero EOB start succeeds but the later ordinary pass rejects
  caller-resolved facts
- **THEN** the handoff returns the typed ordinary-pass error
- **AND** the tile coefficient context state is not committed
