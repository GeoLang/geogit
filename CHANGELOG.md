# Changelog

All notable changes to this project will be documented in this file.

## [Unreleased] - 2026-08-02

### Changed

- README GeoJSON export is a real geometry export, not attributes-only.
- 2026-08-21: `ggt create-workingcopy <path>` honours its path. The path is
  recorded in `.geogit/workingcopy.json` and every command that touches the
  working copy reads it. PostGIS targets are refused instead of half-written.
- 2026-08-15: docs drop "∞ features tracked".
- README command table names `ggt lfs+`, which is the clap name.

### Fixed

- 2026-09-02: `ggt resolve --with ancestor` checks out the merge base of HEAD
  and MERGE_HEAD. It used to check out `MERGE_HEAD~1`, the first parent of the
  merged commit, which is the merge base only when the merged branch is one
  commit ahead.
- `ggt status` and `ggt diff` report the actual feature values of working copy
  edits. Updates and deletes keep their pre-edit row, so diffs show old and new
  values and `ggt commit` writes real values into the tree instead of nulls.

### Removed

- 2026-09-02: The `geogit_core::merge` module, which computed a three-way merge
  over feature deltas. No command called it, and `ggt merge` merges GeoPackage
  bytes through `git merge`.
- 2026-09-02: Shapefile export. `ggt export --list-formats` no longer lists
  SHP, and a `.shp` destination is an error instead of a GeoJSON file written
  next to it.
- `ggt import --all-tables`. Import already reads every table in the source and
  there is no way to select a subset.

## [0.1.0] - 2026-05-30

### Added

- Initial release.
