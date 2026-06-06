# encoder-tools delta: toy-intra-encoder-v0

## ADDED Requirements

### Requirement: minimal toy intra path

`splot-encode` SHALL emit a single intra frame using only writer-supported syntax,
producing a stream that `splot validate` accepts.

#### Scenario: toy output validates

- **WHEN** the toy intra path encodes one frame
- **THEN** `splot validate` accepts the output with no errors
