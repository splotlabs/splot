# process delta: enforce-conventional-commits

## ADDED Requirements

### Requirement: conventional PR titles and commit subjects

Repository pull request titles and commit subjects SHALL use Conventional Commits
text with the format `<type>[optional scope][!]: <description>`, enforced by
`cargo xtask check-conventional-title`, `cargo xtask check-conventional-commits`,
and CI. Tracked by `XTASK-CONVENTIONAL-COMMITS`.

#### Scenario: non-conventional pull request title

- **WHEN** a pull request title does not match the documented Conventional Commits
  format
- **THEN** the CI title check fails with the offending title and the allowed type
  list

#### Scenario: non-conventional commit subject

- **WHEN** a pull request or push contains a commit subject that does not match the
  documented Conventional Commits format
- **THEN** the CI commit-message check fails with the offending commit subject and
  the allowed type list
