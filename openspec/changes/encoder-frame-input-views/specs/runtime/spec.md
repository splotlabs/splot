## ADDED Requirements

### Requirement: Explicit retained encoder input sharing

The encoder input API SHALL define retained input through an explicit shared
frame handle, not through hidden `Clone`, clone-on-write, or implicit copies in
`send_frame`. Retained input sharing SHALL use a visible `.share()` operation and
SHALL borrow back into the same validated frame input view shape.

#### Scenario: shared retained frame does not clone pixels

- **WHEN** a caller wraps an 8-bit YUV420 shared frame as retained encoder input
  and calls `.share()`
- **THEN** the result is another handle to the same frame storage
- **AND** no pixel samples are copied
- **AND** borrowing either retained handle as an encoder `Frame` exposes valid
  borrowed input planes

#### Scenario: unsupported retained format is rejected

- **WHEN** a caller wraps a shared frame whose bit depth or chroma layout is
  outside the current encoder input subset
- **THEN** retained input construction fails with a typed unsupported-input error
- **AND** the rejected frame is not accepted into the encoder lifecycle
