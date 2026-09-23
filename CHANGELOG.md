# Changelog

All notable changes to this project will be documented in this file.

## [Unreleased] - 2026-08-02

### Changed

- 2026-09-16: Public docs match the code. The README says which import names the
  CRS file from the WKT authority and which names it from the source table's srs
  id, and how the working copy picks the srs id it stamps into geometry headers.
  The docs page counts 108 Rust tests, not 82, and its quick start drops a `~`
  that no shell expands inside `GPKG:`.
- README GeoJSON export is a real geometry export, not attributes-only.
- 2026-08-21: `ggt create-workingcopy <path>` honours its path. The path is
  recorded in `.geogit/workingcopy.json` and every command that touches the
  working copy reads it. PostGIS targets are refused instead of half-written.
- 2026-08-15: docs drop "∞ features tracked".
- README command table names `ggt lfs+`, which is the clap name.

### Fixed

- 2026-09-23: `ggt resolve <path> --with-file` encodes the GeoJSON feature the
  way an import does, as a MessagePack feature with GeoPackage binary geometry.
  It used to write the GeoJSON text over the feature blob.
- 2026-09-23: `ggt export --ref` writes the ref's CRS definitions into the
  export. It used to export with no CRS.
- 2026-09-23: `ggt diff` limits its output to the datasets named after `--`. It
  used to parse the names and ignore them.
- 2026-09-23: The spatial filter excludes features. Imports, `checkout` and
  working copy rebuilds compare each geometry's bounding box with the bbox,
  taken from the GeoPackage binary envelope or measured from the WKB for
  points. It used to read only WKT text geometry, which no import stores.
- 2026-09-23: CLI hints name the `ggt` binary instead of `geogit`.
- 2026-09-16: Stored features follow the Kart dataset v3 encoding. Geometry is a
  MessagePack extension of type 71 instead of an array of integers, blobs are
  MessagePack binary, a geometry taken from a source GeoPackage is rewritten
  with srs id 0 and the envelope Kart requires, and every vector import writes
  its CRS to `meta/crs/<identifier>.wkt`. The working copy registers that CRS in
  `gpkg_spatial_ref_sys` under the srs id Kart would give it, an EPSG code or a
  number in Kart's 200000 to 209199 custom range, and stamps that id into every
  geometry header, so the GeoPackage stays self-consistent. Features written by
  older versions still decode.

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
