# Changelog

All notable changes to this project will be documented in this file.

## [Unreleased] - 2026-08-02

### Changed

- 2026-08-15: docs drop "∞ features tracked".
- README command table names `ggt lfs+`, which is the clap name.

### Fixed

- `ggt status` and `ggt diff` report the actual feature values of working copy
  edits. Updates and deletes keep their pre-edit row, so diffs show old and new
  values and `ggt commit` writes real values into the tree instead of nulls.

### Removed

- `ggt import --all-tables`. Import already reads every table in the source and
  there is no way to select a subset.

## [0.1.0] - 2026-05-30

### Added

- Initial release.
