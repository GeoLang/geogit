# GeoGit

[![CI](https://github.com/GeoLang/geogit/actions/workflows/ci.yml/badge.svg)](https://github.com/GeoLang/geogit/actions)
[![License: AGPL-3.0](https://img.shields.io/badge/License-AGPL--3.0-blue.svg)](LICENSE)

Version control for geospatial data on top of Git.

`ggt` imports GeoPackage, Shapefile and PostGIS tables into a Git repository as
one blob per feature row. You edit a GeoPackage working copy in QGIS or any
other GIS, and `ggt` turns the edits into commits. Branches, remotes, push and
pull are plain Git.

## Install

A `v*` tag builds `ggt` for x86_64 and aarch64 Linux and macOS and uploads one
tarball per target to [GitHub Releases](https://github.com/GeoLang/geogit/releases).
To build from a checkout, with Rust 1.88 or later:

```bash
cargo build --release
# binary at target/release/ggt
```

`ggt` runs the `git` on `PATH` for every repository operation, and `git-lfs` for
`ggt lfs+` and for point cloud and raster tiles.

## Quick Start

```bash
# create a repository and import every feature and attribute table in a GeoPackage
ggt init myproject
cd myproject
ggt import GPKG:../parcels.gpkg
ggt status
ggt commit -m "Initial import"

# branch and edit
ggt switch -c update-parcels
# edit myproject.gpkg, the working copy, in QGIS
ggt diff
ggt commit -m "Update parcel boundaries"

# merge back. The first branch is whatever git's init.defaultBranch names
ggt switch master
ggt merge update-parcels

# inspect
ggt data ls
ggt data info parcels
ggt data schema parcels

# export the committed dataset, format from the extension or a FORMAT: prefix
ggt export parcels output.gpkg
ggt export parcels output.geojson
ggt export parcels CSV:output.csv
```

`ggt export` reads the dataset as last committed or checked out, not uncommitted
edits in the working copy.

## Files, Metadata and Licences

```bash
# version arbitrary files, in the "files" dataset unless --dataset names another
ggt files add report.pdf spec.docx
ggt files add --dataset documents report.pdf
ggt files ls
ggt files rm report.pdf

# XML metadata (ISO 19115, FGDC, or any XML) on a table or file dataset
ggt metadata set parcels metadata.xml
ggt metadata show parcels

# a licence as text, or as XML when the file starts with <
ggt license set parcels LICENSE.txt
ggt license show parcels

ggt commit -m "Add project documents"
```

## How It Works

GeoGit stores every feature row as a [MessagePack](https://msgpack.org/)-encoded
blob inside a Git repository. The layout is modelled on
[Kart](https://kartproject.org/)'s dataset v3. Geometry is a MessagePack
extension of type 71 holding GeoPackage binary with srs id 0, and the CRS of a
vector dataset is written to `meta/crs/<identifier>.wkt`. Characters Windows
refuses in a file name are percent-encoded in the stem, so `EPSG:4326` is
stored as `EPSG%3A4326.wkt`.

A Shapefile import takes that identifier from the authority in the `.prj` file.
A GeoPackage import names the file `EPSG:` plus the srs id the source table uses,
where Kart reads the code out of the WKT, so the two disagree on a GeoPackage
that assigns its own srs ids. The working copy stamps a srs id into every
geometry header: the EPSG code when the identifier has one, and otherwise the
number Kart's hash of the identifier gives, in Kart's 200000 to 209199 custom
range.

Feature blobs from Kart's own test repositories decode and re-encode byte for
byte. No command has been run against a whole Kart repository.

```
myproject/
├── .git/
├── .gitignore                  # *.gpkg, so the working copy is not committed
├── parcels/
│   └── .table-dataset/
│       ├── meta/
│       │   ├── title
│       │   ├── description
│       │   ├── schema.json
│       │   ├── path-structure.json
│       │   ├── metadata.xml    # from ggt metadata set
│       │   ├── license         # from ggt license set, license.xml for XML
│       │   ├── crs/            # CRS definitions in WKT, named by identifier
│       │   └── legend/         # column id mappings for stored features
│       └── feature/
│           ├── A/A/A/B/kU0=    # the feature with primary key 77
│           └── ...
├── documents/
│   └── .file-dataset/
│       ├── meta/
│       └── files/
│           ├── report.pdf
│           └── spec.docx
├── lidar/
│   └── .point-cloud-dataset.v1/tile/   # tracked with git lfs when installed
├── dem/
│   └── .raster-dataset.v1/tile/        # tracked with git lfs when installed
└── myproject.gpkg              # working copy
```

- Unchanged features keep the same blob across commits, so Git stores them once.
- The working copy is a GeoPackage. SQL triggers record inserts, updates and
  deletes, and `ggt commit` writes those rows into the tree.
- `ggt switch`, `merge`, `pull`, `reset` and `restore` rebuild the working copy
  from the tree.
- `ggt create-workingcopy <path>` puts the working copy at another path and
  records it in `.geogit/workingcopy.json`.

### Limits

- **Merge** runs plain `git merge` on the feature blobs. There is no
  feature-aware three-way merge, so two edits to the same feature conflict as
  opaque binary blobs. `ggt resolve --with ours|theirs|ancestor|delete` or
  `--ours`/`--theirs` picks a whole blob, then `ggt merge --continue` commits.
  `ggt resolve <path> --with-file feature.geojson` stores one GeoJSON feature
  as the resolved feature instead.
- **Diff** between two commits lists changed blob paths only. Feature-level
  diffs with old and new values cover the working copy against the last commit.
  Dataset names after `--` limit either diff to those datasets. A `dataset:pk`
  filter limits it to the dataset, not the feature.
- **Schema evolution** is not implemented. The schema is written once at import,
  so a column added or dropped in the working copy is ignored at commit time.
- **Spatial filter** (`--spatial-filter minx,miny,maxx,maxy` on `init` and
  `clone`) is stored in `.geogit/spatial-filter.json`. Imports, `checkout` and
  every working copy rebuild leave out features whose bounding box misses the
  bbox. The tree keeps every feature. The bbox is compared in the dataset's own
  CRS, with no reprojection.
- **PostGIS working copies** are not implemented. `create-workingcopy` refuses a
  `postgresql://` target.

## Supported Formats

| Format | Import | Export |
|--------|--------|--------|
| GeoPackage (.gpkg) | every feature and attribute table | yes |
| Shapefile (.shp) | geometry and .dbf attributes | no |
| PostGIS | every table in `geometry_columns`, untested against a live database | no |
| GeoJSON | no | yes |
| CSV | no | yes |
| Any file | `ggt files add`, stored as is | read from `<dataset>/.file-dataset/files/` |
| LAS/LAZ point cloud | `ggt pointcloud import` | no |
| GeoTIFF raster | `ggt raster import` | no |

## Commands

| Command | Description |
|---------|-------------|
| `ggt init [dir] [--import SRC] [--spatial-filter BBOX]` | Create a repository, optionally importing `SRC`. A relative `SRC` path resolves against `dir`, not the current directory |
| `ggt clone <url> [dest] [--spatial-filter BBOX]` | Clone a remote repository |
| `ggt import <SRC> [--name NAME]` | Import `GPKG:file.gpkg`, `SHP:file.shp` or a `postgresql://` connection string. A bare `.gpkg` or `.shp` path also works |
| `ggt status` | Show working copy changes |
| `ggt diff [--stat] [-- DATASETS]` | Feature-level diff of the working copy |
| `ggt diff <base> <target> [--stat] [-- DATASETS]` | Changed blob paths between two commits |
| `ggt commit -m "msg" [datasets]` | Commit working copy changes, optionally only the named datasets or `dataset:pk` features |
| `ggt log [--oneline] [-n N]` | Show commit history |
| `ggt show [commit]` | Show a commit |
| `ggt branch [name] [-d]` | List, create or delete branches |
| `ggt switch <branch> [-c]` | Switch branches, `-c` creates the branch |
| `ggt merge <branch> [--abort] [--continue]` | Merge a branch with `git merge` |
| `ggt push [remote] [branch]` | Push, `origin` by default |
| `ggt pull [remote] [branch]` | Pull, `origin` by default |
| `ggt remote add\|remove\|ls` | Manage remotes |
| `ggt reset [target]` | `git reset --hard` to a commit and rebuild the working copy |
| `ggt restore <datasets> [--source REF]` | Restore datasets from a commit |
| `ggt checkout [datasets]` | Write datasets from the tree into the working copy |
| `ggt create-workingcopy <path>` | Create the GeoPackage working copy at `<path>` |
| `ggt conflicts [ls\|abort]` | List merge conflicts or abort the merge |
| `ggt resolve [path] [--with STRATEGY] [--ours] [--theirs] [--with-file FILE]` | Resolve conflicts, all of them when no path is given. `--with-file` needs a feature path and a GeoJSON feature |
| `ggt export <ds> <path> [--ref REF]` | Export to GPKG, GeoJSON or CSV. `--ref` reads the schema, CRS and features from `REF`. `--list-formats` prints the format names |
| `ggt data ls\|info\|schema` | Inspect datasets |
| `ggt files add\|ls\|rm` | Manage versioned files |
| `ggt metadata set\|show` | Dataset XML metadata |
| `ggt license set\|show` | Dataset licence |
| `ggt pointcloud import\|ls\|info` | Point cloud datasets (LAS/LAZ) |
| `ggt raster import\|ls\|info` | Raster datasets (GeoTIFF) |
| `ggt lfs+ ls-files\|fetch\|gc` | Git LFS objects: list, fetch from `origin`, prune |
| `ggt version` | Show the version |

## Crates

| Crate | Purpose |
|-------|---------|
| `geogit-encoding` | MessagePack feature encoding, geometry, paths, schemas, CRS |
| `geogit-core` | Dataset model and feature diffs |
| `geogit-git` | Git object storage through the `git` command |
| `geogit-wc` | GeoPackage working copy and change tracking |
| `geogit` | The `ggt` binary |

## License

AGPL-3.0-or-later, see [LICENSE](LICENSE).

Copyright (C) 2026 Grok Image Compression Inc.
