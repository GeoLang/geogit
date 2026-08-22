# GeoGit

[![CI](https://github.com/GeoLang/geogit/actions/workflows/ci.yml/badge.svg)](https://github.com/GeoLang/geogit/actions)
[![License: AGPL-3.0](https://img.shields.io/badge/License-AGPL--3.0-blue.svg)](LICENSE)

**Distributed version control for geospatial data.**

GeoGit brings Git-like branching, diffing, merging, and collaboration to geodata. Import GeoPackages, edit in QGIS, commit changes, push/pull between machines — all with row-level tracking.

## Quick Start

```bash
# Initialize a repo and import a GeoPackage
ggt init myproject --import GPKG:parcels.gpkg
cd myproject

# Check status
ggt status

# Commit
ggt commit -m "Initial import"

# Branch and edit
ggt branch feature/update-parcels
ggt switch feature/update-parcels
# ... edit the working copy GeoPackage in QGIS ...
ggt commit -m "Update parcel boundaries"

# Merge
ggt switch main
ggt merge feature/update-parcels

# Inspect data
ggt data ls
ggt data info parcels
ggt data schema parcels

# Export (GeoJSON export carries attributes only, geometry is not exported)
ggt export parcels output.gpkg
ggt export parcels output.geojson
ggt export parcels output.csv
```

## File & Document Version Control

Version arbitrary files alongside geospatial datasets:

```bash
# Add files to a file dataset
ggt files add report.pdf spec.docx

# Add to a custom dataset
ggt files add --dataset documents report.pdf

# List tracked files
ggt files ls

# Remove a file
ggt files rm report.pdf

# Commit as usual
ggt commit -m "Add project documents"
```

## Dataset Metadata & Licensing

Attach ISO 19115 XML metadata and license information to any dataset:

```bash
# Set XML metadata (ISO 19115, FGDC, or any XML)
ggt metadata set parcels metadata.xml
ggt metadata show parcels

# Set license (text or XML)
ggt license set parcels LICENSE.txt
ggt license show parcels

# Works with both table and file datasets
ggt metadata set documents meta.xml
```

## How It Works

GeoGit stores every feature row as a [MessagePack](https://msgpack.org/)-encoded blob inside a standard Git repository. The layout is modelled on [Kart](https://kartproject.org/), but interoperability with Kart repositories is not implemented and untested: geometry is encoded as a MessagePack array rather than a binary value, and the GeoPackage binary header keeps the source SRS id where Kart requires 0.

```
myproject/
├── .git/                       # Standard Git repository
├── .gitignore                  # Excludes working copy (.gpkg)
├── parcels/
│   └── .table-dataset/
│       ├── meta/
│       │   ├── title
│       │   ├── description
│       │   ├── schema.json     # Column definitions
│       │   ├── metadata.xml    # ISO 19115 metadata (optional)
│       │   ├── license         # License text (optional)
│       │   └── legend/         # Column ID mappings for stored features
│       └── feature/
│           ├── A/A/A/B/kU0=    # Feature with PK=77
│           └── ...             # One blob per row
├── documents/
│   └── .file-dataset/
│       ├── meta/
│       │   ├── title
│       │   └── metadata.xml    # Optional metadata
│       └── files/
│           ├── report.pdf      # Versioned files
│           └── spec.docx
└── myproject.gpkg              # Working copy (editable in QGIS)
```

**Key properties:**
- **Git deduplication** — unchanged features share blobs across commits (zero cost)
- **Efficient diffs** — compare blob OIDs, skip unchanged subtrees: O(changed) not O(total)
- **Standard remotes** — push/pull to GitHub, GitLab, or any Git host
- **Edit anywhere** — working copy is a GeoPackage, editable in any GIS software
- **File tracking** — version documents and files alongside geodata
- **Point clouds** — import and inspect LAS/LAZ point cloud tiles
- **Raster datasets** — import and inspect GeoTIFF raster tiles

Schema evolution is not implemented. The dataset schema is written once at import and never rewritten, so a column added or dropped in the working copy is ignored at commit time.

## Supported Formats

| Format | Import | Export |
|--------|--------|--------|
| GeoPackage (.gpkg) | ✅ | ✅ |
| Shapefile (.shp) | Attributes only, geometry is discarded | — |
| GeoJSON | — | Attributes only, geometry is always `null` |
| CSV | — | ✅ |
| PostGIS | Broken: every non-geometry column imports as Null | — |
| Files (any) | ✅ | ✅ |
| LAS/LAZ (point cloud) | ✅ | — |
| GeoTIFF (raster) | ✅ | — |

## Commands

| Command | Description |
|---------|-------------|
| `ggt init [dir]` | Initialize a new repository |
| `ggt clone <url>` | Clone a remote repository |
| `ggt import GPKG:file.gpkg` | Import a GeoPackage dataset |
| `ggt status` | Show working copy changes |
| `ggt diff [base] [target]` | Feature-level diff of the working copy; with a target, lists changed blob paths only |
| `ggt commit -m "msg"` | Commit changes |
| `ggt log [--oneline] [-n N]` | Show commit history |
| `ggt show [commit]` | Display commit details |
| `ggt branch [name] [-d]` | List/create/delete branches |
| `ggt switch <branch> [-c]` | Switch branches |
| `ggt merge <branch>` | Merge a branch with plain `git merge`. Feature-aware three-way merge is not implemented, so two edits to the same feature conflict as opaque binary blobs |
| `ggt push [remote] [branch]` | Push to a remote |
| `ggt pull [remote] [branch]` | Pull from a remote |
| `ggt remote add\|remove\|ls` | Manage remotes |
| `ggt reset [target]` | Reset to a commit |
| `ggt restore <datasets>` | Restore datasets from a commit |
| `ggt checkout [datasets]` | Checkout tree to working copy |
| `ggt export <ds> <path>` | Export to GPKG/GeoJSON/CSV |
| `ggt data ls\|info\|schema` | Inspect datasets |
| `ggt files add\|ls\|rm` | Manage versioned files |
| `ggt metadata set\|show` | Dataset XML metadata |
| `ggt license set\|show` | Dataset license management |
| `ggt pointcloud import\|ls\|info` | Point cloud datasets (LAS/LAZ) |
| `ggt raster import\|ls\|info` | Raster datasets (GeoTIFF) |
| `ggt conflicts [ls\|abort]` | View/manage merge conflicts |
| `ggt resolve [paths]` | Resolve conflicts |
| `ggt create-workingcopy <path>` | Create the GeoPackage working copy at `<path>`, recorded in `.geogit/workingcopy.json` so status, commit, diff and checkout read it. PostGIS targets are refused |
| `ggt lfs+ ls-files\|fetch\|gc` | Manage Git LFS objects |
| `ggt version` | Show version information |

## Building

```bash
cargo build --release
# Binary at target/release/ggt
```

## Architecture

Five Rust crates in a workspace:

| Crate | Purpose |
|-------|---------|
| `geogit-encoding` | MessagePack feature encoding, geometry, paths, schemas |
| `geogit-core` | Dataset model, diff engine |
| `geogit-git` | Git object storage via shell git |
| `geogit-wc` | GeoPackage working copy |
| `geogit` (CLI) | Command-line interface (`ggt`) via [clap](https://github.com/clap-rs/clap) |

## License

AGPL-3.0-or-later, see [LICENSE](LICENSE).

Copyright (C) 2026 Grok Image Compression Inc.
