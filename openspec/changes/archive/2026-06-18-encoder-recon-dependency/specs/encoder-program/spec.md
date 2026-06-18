## MODIFIED Requirements

### Requirement: Recon dependency decision is isolated

After the docs-only contract lands, `encoder-recon-dependency` SHALL be the only
change that decides and lands the direct `splot-encode -> splot-recon` dependency
edge. That change SHALL update dependency policy and proof while preserving the
existing unimplemented encoder behavior.

#### Scenario: recon dependency lands explicitly

- **WHEN** the `encoder-recon-dependency` change is complete
- **THEN** `splot-encode` may depend directly on `splot-core`, `splot-parallel`,
  and `splot-recon`
- **AND** `send_frame`, `receive_packet`, and `flush` still do not expose public
  successful encode behavior.

#### Scenario: broader graph changes remain forbidden

- **WHEN** dependency direction is checked after the change
- **THEN** `splot-encode` still does not depend on `splot-decode`,
  `splot-validate`, or `splot-cli`
- **AND** `splot-recon` still depends only on `splot-tables`.
