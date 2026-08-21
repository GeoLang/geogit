use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};

use geogit_core::dataset::DatasetMeta;
use geogit_encoding::feature::StoredFeature;
use geogit_encoding::legend::Legend;
use geogit_encoding::path::PathStructure;
use geogit_encoding::schema::{Column, DataType, Schema};
use geogit_encoding::value::ColumnValue;
use geogit_git::Repository;
use geogit_git::tree::TreeBuilder;
use geogit_wc::gpkg::{GeoPackageWorkingCopy, read_sqlite_value};
use geogit_wc::traits::WorkingCopy;

/// A feature row: primary key values + column name-value map.
type FeatureRow = (Vec<ColumnValue>, HashMap<String, ColumnValue>);

#[derive(Parser)]
#[command(
    name = "ggt",
    version,
    about = "Distributed version control for geospatial data",
    long_about = "GeoGit provides Git-like version control for geospatial and tabular datasets.\n\
                  Store, branch, diff, merge, push, and pull geodata — edit directly in QGIS."
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Initialize a new GeoGit repository
    Init {
        #[arg(default_value = ".")]
        dir: PathBuf,
        #[arg(long)]
        import: Option<String>,
        /// Spatial filter for the repository (WKT or bbox: minx,miny,maxx,maxy)
        #[arg(long, allow_hyphen_values = true)]
        spatial_filter: Option<String>,
    },
    /// Clone a remote repository
    Clone {
        url: String,
        dest: Option<PathBuf>,
        /// Spatial filter (bbox: minx,miny,maxx,maxy)
        #[arg(long, allow_hyphen_values = true)]
        spatial_filter: Option<String>,
    },
    /// Import a dataset into the repository
    Import {
        /// Source: GPKG:path, SHP:path, or postgresql://user:pass@host/db/schema
        source: String,
        #[arg(long)]
        name: Option<String>,
        /// Dataset name for raster/point-cloud tiles
        #[arg(long)]
        dataset: Option<String>,
    },
    /// Show repository status
    Status,
    /// Commit changes from the working copy
    Commit {
        #[arg(short, long)]
        message: String,
        /// Only commit changes in these datasets (dataset or dataset:pk)
        #[arg(trailing_var_arg = true)]
        filters: Vec<String>,
    },
    /// Show commit history
    Log {
        #[arg(long)]
        oneline: bool,
        #[arg(short = 'n', long)]
        max_count: Option<usize>,
    },
    /// Show details of a commit
    Show {
        #[arg(default_value = "HEAD")]
        commit: String,
    },
    /// Show differences between versions
    Diff {
        /// Commit spec: HEAD, commit, commit...commit, or commit..commit
        #[arg(default_value = "HEAD")]
        base: String,
        /// Target (omit for working copy diff)
        target: Option<String>,
        #[arg(long)]
        stat: bool,
        /// Filter: dataset or dataset:pk
        #[arg(trailing_var_arg = true)]
        filters: Vec<String>,
    },
    /// List, create, or delete branches
    Branch {
        name: Option<String>,
        #[arg(short, long)]
        delete: bool,
    },
    /// Switch to a different branch
    Switch {
        branch: String,
        #[arg(short, long)]
        create: bool,
    },
    /// Merge a branch into the current branch
    Merge {
        branch: String,
        #[arg(long)]
        abort: bool,
        /// Continue merge after resolving conflicts
        #[arg(long = "continue")]
        cont: bool,
    },
    /// Push commits to a remote
    Push {
        #[arg(default_value = "origin")]
        remote: String,
        branch: Option<String>,
    },
    /// Pull commits from a remote
    Pull {
        #[arg(default_value = "origin")]
        remote: String,
        branch: Option<String>,
    },
    /// Manage remotes
    Remote {
        #[command(subcommand)]
        subcommand: RemoteCommand,
    },
    /// Reset the working copy to a clean state
    Reset {
        #[arg(default_value = "HEAD")]
        target: String,
    },
    /// Restore specific datasets from a commit
    Restore {
        datasets: Vec<String>,
        #[arg(long, default_value = "HEAD")]
        source: String,
    },
    /// Checkout dataset(s) to a working copy
    Checkout { datasets: Vec<String> },
    /// Create or change working copy type
    #[command(name = "create-workingcopy")]
    CreateWorkingcopy {
        /// Path (e.g. data.gpkg) or connection string (postgresql://...)
        target: String,
    },
    /// Manage merge conflicts
    Conflicts {
        #[command(subcommand)]
        subcommand: Option<ConflictsCommand>,
    },
    /// Resolve merge conflicts
    Resolve {
        /// Conflict path (e.g. layer:feature:123)
        conflict: Option<String>,
        /// Resolution: ours, theirs, ancestor, delete, workingcopy
        #[arg(long = "with")]
        with_strategy: Option<String>,
        /// GeoJSON file with resolution
        #[arg(long = "with-file")]
        with_file: Option<PathBuf>,
        /// Accept theirs for all
        #[arg(long)]
        theirs: bool,
        /// Accept ours for all
        #[arg(long)]
        ours: bool,
    },
    /// Export datasets to other formats
    Export {
        /// Dataset to export
        dataset: Option<String>,
        /// Destination path or format:path (GPKG:, SHP:, CSV:, GEOJSON:)
        destination: Option<String>,
        /// List supported export formats
        #[arg(long)]
        list_formats: bool,
        /// Export from a specific ref
        #[arg(long = "ref")]
        from_ref: Option<String>,
    },
    /// List and inspect datasets
    Data {
        #[command(subcommand)]
        subcommand: DataCommand,
    },
    /// Manage versioned files alongside datasets
    Files {
        #[command(subcommand)]
        subcommand: FilesCommand,
    },
    /// Manage dataset XML metadata
    Metadata {
        #[command(subcommand)]
        subcommand: MetadataCommand,
    },
    /// Manage dataset license information
    License {
        #[command(subcommand)]
        subcommand: LicenseCommand,
    },
    /// Manage point cloud datasets (LAS/LAZ)
    Pointcloud {
        #[command(subcommand)]
        subcommand: PointcloudCommand,
    },
    /// Manage raster datasets (GeoTIFF)
    Raster {
        #[command(subcommand)]
        subcommand: RasterCommand,
    },
    /// Manage Git LFS files
    #[command(name = "lfs+")]
    Lfs {
        #[command(subcommand)]
        subcommand: LfsCommand,
    },
    /// Show version information
    Version,
}

#[derive(Subcommand)]
enum RemoteCommand {
    Add {
        name: String,
        url: String,
    },
    Remove {
        name: String,
    },
    #[command(name = "ls")]
    List,
}

#[derive(Subcommand)]
enum ConflictsCommand {
    #[command(name = "ls")]
    List,
    Abort,
}

#[derive(Subcommand)]
enum DataCommand {
    Ls,
    Info { dataset: String },
    Schema { dataset: String },
}

#[derive(Subcommand)]
enum LfsCommand {
    /// List LFS files referenced by a commit
    #[command(name = "ls-files")]
    LsFiles {
        #[arg(default_value = "HEAD")]
        commit: String,
        #[arg(long)]
        all: bool,
    },
    /// Fetch LFS files from remote
    Fetch { commits: Vec<String> },
    /// Clean up unused LFS files
    Gc,
}

#[derive(Subcommand)]
enum FilesCommand {
    /// Add files to a file dataset
    Add {
        /// Dataset name for the file group
        #[arg(long, default_value = "files")]
        dataset: String,
        /// Files to add
        paths: Vec<PathBuf>,
    },
    /// List tracked files
    #[command(name = "ls")]
    Ls {
        /// Dataset name
        #[arg(long)]
        dataset: Option<String>,
    },
    /// Remove a tracked file
    Rm {
        /// Dataset name
        #[arg(long, default_value = "files")]
        dataset: String,
        /// File paths to remove
        paths: Vec<PathBuf>,
    },
}

#[derive(Subcommand)]
enum MetadataCommand {
    /// Set XML metadata for a dataset
    Set {
        /// Dataset name
        dataset: String,
        /// Path to XML metadata file
        file: PathBuf,
    },
    /// Show metadata for a dataset
    Show {
        /// Dataset name
        dataset: String,
    },
}

#[derive(Subcommand)]
enum LicenseCommand {
    /// Set license for a dataset
    Set {
        /// Dataset name
        dataset: String,
        /// Path to license file (text or XML)
        file: PathBuf,
    },
    /// Show license for a dataset
    Show {
        /// Dataset name
        dataset: String,
    },
}

#[derive(Subcommand)]
enum PointcloudCommand {
    /// Import LAS/LAZ point cloud files
    Import {
        /// Dataset name
        #[arg(long)]
        dataset: Option<String>,
        /// LAS/LAZ files to import
        files: Vec<PathBuf>,
    },
    /// List point cloud datasets
    #[command(name = "ls")]
    Ls,
    /// Show info about a point cloud dataset
    Info {
        /// Dataset name
        dataset: String,
    },
}

#[derive(Subcommand)]
enum RasterCommand {
    /// Import GeoTIFF raster files
    Import {
        /// Dataset name
        #[arg(long)]
        dataset: Option<String>,
        /// GeoTIFF files to import
        files: Vec<PathBuf>,
    },
    /// List raster datasets
    #[command(name = "ls")]
    Ls,
    /// Show info about a raster dataset
    Info {
        /// Dataset name
        dataset: String,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Init {
            dir,
            import,
            spatial_filter,
        } => cmd_init(&dir, import.as_deref(), spatial_filter.as_deref()),
        Command::Clone {
            url,
            dest,
            spatial_filter,
        } => cmd_clone(&url, dest.as_deref(), spatial_filter.as_deref()),
        Command::Import {
            source,
            name,
            dataset,
        } => cmd_import(&source, name.as_deref().or(dataset.as_deref())),
        Command::Status => cmd_status(),
        Command::Commit { message, filters } => cmd_commit(&message, &filters),
        Command::Log { oneline, max_count } => cmd_log(oneline, max_count),
        Command::Show { commit } => cmd_show(&commit),
        Command::Diff {
            base,
            target,
            stat,
            filters: _,
        } => cmd_diff(&base, target.as_deref(), stat),
        Command::Branch { name, delete } => cmd_branch(name.as_deref(), delete),
        Command::Switch { branch, create } => cmd_switch(&branch, create),
        Command::Merge {
            branch,
            abort,
            cont,
        } => cmd_merge(&branch, abort, cont),
        Command::Push { remote, branch } => cmd_push(&remote, branch.as_deref()),
        Command::Pull { remote, branch } => cmd_pull(&remote, branch.as_deref()),
        Command::Remote { subcommand } => match subcommand {
            RemoteCommand::Add { name, url } => cmd_remote_add(&name, &url),
            RemoteCommand::Remove { name } => cmd_remote_remove(&name),
            RemoteCommand::List => cmd_remote_list(),
        },
        Command::Reset { target } => cmd_reset(&target),
        Command::Restore { datasets, source } => cmd_restore(&datasets, &source),
        Command::Checkout { datasets } => cmd_checkout(&datasets),
        Command::CreateWorkingcopy { target } => cmd_create_workingcopy(&target),
        Command::Conflicts { subcommand } => match subcommand {
            Some(ConflictsCommand::List) | None => cmd_conflicts_list(),
            Some(ConflictsCommand::Abort) => cmd_conflicts_abort(),
        },
        Command::Resolve {
            conflict,
            with_strategy,
            with_file,
            theirs,
            ours,
        } => cmd_resolve(
            conflict.as_deref(),
            with_strategy.as_deref(),
            with_file.as_deref(),
            theirs,
            ours,
        ),
        Command::Export {
            dataset,
            destination,
            list_formats,
            from_ref,
        } => cmd_export(
            dataset.as_deref(),
            destination.as_deref(),
            list_formats,
            from_ref.as_deref(),
        ),
        Command::Data { subcommand } => match subcommand {
            DataCommand::Ls => cmd_data_ls(),
            DataCommand::Info { dataset } => cmd_data_info(&dataset),
            DataCommand::Schema { dataset } => cmd_data_schema(&dataset),
        },
        Command::Files { subcommand } => match subcommand {
            FilesCommand::Add { dataset, paths } => cmd_files_add(&dataset, &paths),
            FilesCommand::Ls { dataset } => cmd_files_ls(dataset.as_deref()),
            FilesCommand::Rm { dataset, paths } => cmd_files_rm(&dataset, &paths),
        },
        Command::Metadata { subcommand } => match subcommand {
            MetadataCommand::Set { dataset, file } => cmd_metadata_set(&dataset, &file),
            MetadataCommand::Show { dataset } => cmd_metadata_show(&dataset),
        },
        Command::License { subcommand } => match subcommand {
            LicenseCommand::Set { dataset, file } => cmd_license_set(&dataset, &file),
            LicenseCommand::Show { dataset } => cmd_license_show(&dataset),
        },
        Command::Pointcloud { subcommand } => match subcommand {
            PointcloudCommand::Import { dataset, files } => {
                cmd_pointcloud_import(dataset.as_deref(), &files)
            }
            PointcloudCommand::Ls => cmd_pointcloud_ls(),
            PointcloudCommand::Info { dataset } => cmd_pointcloud_info(&dataset),
        },
        Command::Raster { subcommand } => match subcommand {
            RasterCommand::Import { dataset, files } => {
                cmd_raster_import(dataset.as_deref(), &files)
            }
            RasterCommand::Ls => cmd_raster_ls(),
            RasterCommand::Info { dataset } => cmd_raster_info(&dataset),
        },
        Command::Lfs { subcommand } => match subcommand {
            LfsCommand::LsFiles { commit, all } => cmd_lfs_ls_files(&commit, all),
            LfsCommand::Fetch { commits } => cmd_lfs_fetch(&commits),
            LfsCommand::Gc => cmd_lfs_gc(),
        },
        Command::Version => {
            println!("geogit {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
    }
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

fn find_repo_root() -> Result<PathBuf> {
    let mut dir = std::env::current_dir()?;
    loop {
        if dir.join(".git").exists() {
            return Ok(dir);
        }
        if !dir.pop() {
            bail!("not a geogit repository (or any parent up to mount point)");
        }
    }
}

fn wc_path(root: &Path) -> PathBuf {
    root.join(format!(
        "{}.gpkg",
        root.file_name().unwrap().to_string_lossy()
    ))
}

fn find_datasets(dir: &Path, prefix: &str, results: &mut Vec<String>) {
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            if name == ".table-dataset" {
                results.push(prefix.trim_end_matches('/').to_string());
            } else if !name.starts_with('.') && name != "target" && !name.ends_with(".gpkg") {
                let new_prefix = if prefix.is_empty() {
                    name.clone()
                } else {
                    format!("{prefix}/{name}")
                };
                find_datasets(&path, &new_prefix, results);
            }
        }
    }
}

fn count_files(dir: &Path) -> usize {
    let mut count = 0;
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                count += count_files(&path);
            } else {
                count += 1;
            }
        }
    }
    count
}

fn format_pk(pk: &[ColumnValue]) -> String {
    pk.iter().map(format_value).collect::<Vec<_>>().join(",")
}

fn format_value(v: &ColumnValue) -> String {
    match v {
        ColumnValue::Null => "NULL".to_string(),
        ColumnValue::Bool(b) => b.to_string(),
        ColumnValue::Integer(i) => i.to_string(),
        ColumnValue::Float(f) => f.to_string(),
        ColumnValue::Text(s) => {
            if s.len() > 50 {
                format!("\"{}...\"", &s[..47])
            } else {
                format!("\"{s}\"")
            }
        }
        ColumnValue::Blob(b) => format!("<{} bytes>", b.len()),
    }
}

fn load_features_from_tree(feature_dir: &Path, meta: &DatasetMeta) -> Result<Vec<FeatureRow>> {
    let mut features = Vec::new();
    let legend_dir = feature_dir.parent().unwrap().join("meta/legend");
    let legends = load_legends(&legend_dir)?;
    walk_feature_files(feature_dir, &legends, &meta.schema, &mut features)?;
    Ok(features)
}

fn load_legends(legend_dir: &Path) -> Result<HashMap<String, Legend>> {
    let mut legends = HashMap::new();
    if let Ok(entries) = std::fs::read_dir(legend_dir) {
        for entry in entries.flatten() {
            let hash = entry.file_name().to_string_lossy().to_string();
            let data = std::fs::read(entry.path())?;
            let legend = Legend::from_msgpack(&data).map_err(|e| anyhow::anyhow!("{e}"))?;
            legends.insert(hash, legend);
        }
    }
    Ok(legends)
}

fn walk_feature_files(
    dir: &Path,
    legends: &HashMap<String, Legend>,
    schema: &Schema,
    out: &mut Vec<FeatureRow>,
) -> Result<()> {
    use base64::{Engine, engine::general_purpose::URL_SAFE};
    if !dir.exists() {
        return Ok(());
    }
    let pk_cols: Vec<&Column> = schema
        .0
        .iter()
        .filter(|c| c.primary_key_index.is_some())
        .collect();
    for entry in std::fs::read_dir(dir)?.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk_feature_files(&path, legends, schema, out)?;
        } else {
            let data = std::fs::read(&path)?;
            let feature = StoredFeature::from_msgpack(&data).map_err(|e| anyhow::anyhow!("{e}"))?;
            if let Some(legend) = legends.get(&feature.legend_hash) {
                let mut values = legend.decode_values(&feature.values, schema);
                // Decode PK from filename (base64-encoded msgpack)
                let filename = path.file_name().unwrap().to_string_lossy();
                let pk: Vec<ColumnValue> =
                    if let Ok(pk_bytes) = URL_SAFE.decode(filename.as_bytes()) {
                        if let Ok(pk_vals) = rmp_serde::from_slice::<Vec<ColumnValue>>(&pk_bytes) {
                            // Add PK values to the values map
                            for (i, col) in pk_cols.iter().enumerate() {
                                if i < pk_vals.len() {
                                    values.insert(col.name.clone(), pk_vals[i].clone());
                                }
                            }
                            pk_vals
                        } else {
                            vec![ColumnValue::Null; pk_cols.len()]
                        }
                    } else {
                        vec![ColumnValue::Null; pk_cols.len()]
                    };
                out.push((pk, values));
            }
        }
    }
    Ok(())
}

fn refresh_working_copy(root: &Path, wc_gpkg: &Path) -> Result<()> {
    let _ = std::fs::remove_file(wc_gpkg);
    let mut datasets = Vec::new();
    find_datasets(root, "", &mut datasets);
    if datasets.is_empty() {
        return Ok(());
    }
    let spatial_filter = load_spatial_filter(root);
    let mut wc = GeoPackageWorkingCopy::open(wc_gpkg)?;
    for ds in &datasets {
        let meta = load_dataset_meta(root, ds)?;
        let feature_dir = root.join(ds).join(".table-dataset/feature");
        let mut features = load_features_from_tree(&feature_dir, &meta)?;
        if let Some(ref bbox) = spatial_filter {
            features.retain(|(_pk, vals)| feature_in_bbox(vals, &meta.schema, bbox));
        }
        wc.checkout(ds, &meta, &features)?;
    }
    Ok(())
}

/// Load spatial filter bbox from .geogit/spatial-filter.json if present.
/// Returns (minx, miny, maxx, maxy).
fn load_spatial_filter(root: &Path) -> Option<(f64, f64, f64, f64)> {
    let filter_path = root.join(".geogit/spatial-filter.json");
    if !filter_path.exists() {
        return None;
    }
    let content = std::fs::read_to_string(&filter_path).ok()?;
    let parsed: serde_json::Value = serde_json::from_str(&content).ok()?;
    let bbox_str = parsed.get("bbox")?.as_str()?;
    let parts: Vec<f64> = bbox_str
        .split(',')
        .filter_map(|s| s.trim().parse().ok())
        .collect();
    if parts.len() == 4 {
        Some((parts[0], parts[1], parts[2], parts[3]))
    } else {
        None
    }
}

/// Check if a feature's geometry column value falls within the given bbox.
/// Uses a simple text-based check for Point geometries or WKT-based parsing.
fn feature_in_bbox(
    values: &HashMap<String, ColumnValue>,
    schema: &Schema,
    bbox: &(f64, f64, f64, f64),
) -> bool {
    let geom_col = schema.0.iter().find(|c| c.data_type == DataType::Geometry);
    let Some(gc) = geom_col else { return true };
    let Some(val) = values.get(&gc.name) else {
        return true;
    };
    match val {
        ColumnValue::Text(wkt) => {
            // Simple check: extract coordinates from POINT(x y) or first coord
            if let Some(coords) = extract_first_coord(wkt) {
                coords.0 >= bbox.0 && coords.0 <= bbox.2 && coords.1 >= bbox.1 && coords.1 <= bbox.3
            } else {
                true // Can't parse, include by default
            }
        }
        ColumnValue::Blob(data) => {
            // For WKB geometry blobs, we'd need to parse - for now include all
            let _ = data;
            true
        }
        _ => true,
    }
}

/// Extract the first (x, y) coordinate from a WKT string.
fn extract_first_coord(wkt: &str) -> Option<(f64, f64)> {
    // Find the first opening paren or "POINT " prefix
    let pos = wkt.find('(')?;
    let coord_str = &wkt[pos + 1..];
    // Get first coordinate pair before ) or ,
    let end = coord_str.find([',', ')'])?;
    let pair = &coord_str[..end];
    let parts: Vec<&str> = pair.split_whitespace().collect();
    if parts.len() >= 2 {
        let x = parts[0].parse().ok()?;
        let y = parts[1].parse().ok()?;
        Some((x, y))
    } else {
        None
    }
}

fn load_dataset_meta(root: &Path, ds: &str) -> Result<DatasetMeta> {
    let meta_dir = root.join(ds).join(".table-dataset/meta");
    let schema_json = std::fs::read_to_string(meta_dir.join("schema.json"))?;
    let schema: Schema = serde_json::from_str(&schema_json)?;
    let title = std::fs::read_to_string(meta_dir.join("title")).unwrap_or_default();
    let description = std::fs::read_to_string(meta_dir.join("description")).unwrap_or_default();
    let ps: PathStructure = if meta_dir.join("path-structure.json").exists() {
        serde_json::from_str(&std::fs::read_to_string(
            meta_dir.join("path-structure.json"),
        )?)?
    } else {
        PathStructure::default()
    };
    Ok(DatasetMeta {
        title: title.trim().to_string(),
        description: description.trim().to_string(),
        schema,
        path_structure: ps,
    })
}

fn sync_wc_to_tree(root: &Path, wc_gpkg: &Path, filter_datasets: &[String]) -> Result<()> {
    let wc = GeoPackageWorkingCopy::open(wc_gpkg)?;
    let datasets = wc.list_datasets()?;

    for ds in &datasets {
        if !filter_datasets.is_empty() && !filter_datasets.iter().any(|f| ds.starts_with(f)) {
            continue;
        }
        let changes = wc.status(ds)?;
        if changes.is_empty() {
            continue;
        }
        let schema_path = root.join(ds).join(".table-dataset/meta/schema.json");
        if !schema_path.exists() {
            continue;
        }
        let schema_json = std::fs::read_to_string(&schema_path)?;
        let schema: Schema = serde_json::from_str(&schema_json)?;
        let non_pk_ids: Vec<uuid::Uuid> = schema
            .0
            .iter()
            .filter(|c| c.primary_key_index.is_none())
            .map(|c| c.id)
            .collect();
        let legend = Legend::new(non_pk_ids);
        let legend_hash = legend.hash();
        let ps_path = root
            .join(ds)
            .join(".table-dataset/meta/path-structure.json");
        let ps: PathStructure = if ps_path.exists() {
            serde_json::from_str(&std::fs::read_to_string(&ps_path)?)?
        } else {
            PathStructure::default()
        };
        let feature_dir = root.join(ds).join(".table-dataset/feature");

        for delta in &changes {
            match delta {
                geogit_core::diff::FeatureDelta::Insert { pk, new }
                | geogit_core::diff::FeatureDelta::Update { pk, new, .. } => {
                    let stored_vals: Vec<ColumnValue> = schema
                        .0
                        .iter()
                        .filter(|c| c.primary_key_index.is_none())
                        .map(|c| new.get(&c.name).cloned().unwrap_or(ColumnValue::Null))
                        .collect();
                    let stored = StoredFeature {
                        legend_hash: legend_hash.clone(),
                        values: stored_vals,
                    };
                    let rel_path = ps.feature_path(pk);
                    let full_path = feature_dir.join(&rel_path);
                    if let Some(parent) = full_path.parent() {
                        std::fs::create_dir_all(parent)?;
                    }
                    std::fs::write(&full_path, stored.to_msgpack())?;
                }
                geogit_core::diff::FeatureDelta::Delete { pk, .. } => {
                    let rel_path = ps.feature_path(pk);
                    let full_path = feature_dir.join(&rel_path);
                    let _ = std::fs::remove_file(&full_path);
                }
            }
        }
        wc.clear_tracking(ds)?;
    }
    Ok(())
}

#[allow(clippy::collapsible_if)]
fn read_gpkg_schema(conn: &rusqlite::Connection, table_name: &str) -> Result<Schema> {
    let geom_cols: HashMap<String, (String, i64)> = {
        let mut map = HashMap::new();
        if let Ok(mut stmt) = conn.prepare(
            "SELECT column_name, geometry_type_name, COALESCE(srs_id, 4326)
             FROM gpkg_geometry_columns WHERE table_name = ?1",
        ) {
            if let Ok(rows) = stmt.query_map([table_name], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    (row.get::<_, String>(1)?, row.get::<_, i64>(2)?),
                ))
            }) {
                for row in rows.flatten() {
                    map.insert(row.0, row.1);
                }
            }
        }
        map
    };

    let mut stmt = conn.prepare(&format!("PRAGMA table_info(\"{}\")", table_name))?;
    let mut columns = Vec::new();
    let rows = stmt.query_map([], |row| {
        let name: String = row.get(1)?;
        let type_name: String = row.get(2)?;
        let pk: i32 = row.get(5)?;
        Ok((name, type_name, pk))
    })?;
    for row in rows {
        let (name, type_name, pk) = row?;
        let (data_type, geom_type, geom_crs, size) = if let Some((gt, srs_id)) =
            geom_cols.get(&name)
        {
            (
                DataType::Geometry,
                Some(gt.clone()),
                Some(format!("EPSG:{srs_id}")),
                None,
            )
        } else {
            let upper = type_name.to_uppercase();
            let dt = if upper.contains("INT") {
                DataType::Integer
            } else if upper.contains("REAL") || upper.contains("FLOAT") || upper.contains("DOUBLE")
            {
                DataType::Float
            } else if upper.contains("BLOB") {
                DataType::Blob
            } else if upper.contains("BOOL") {
                DataType::Boolean
            } else {
                DataType::Text
            };
            let size = match dt {
                DataType::Integer => Some(64u32),
                DataType::Float if upper.contains("REAL") => Some(32),
                DataType::Float => Some(64),
                _ => None,
            };
            (dt, None, None, size)
        };
        columns.push(Column {
            id: uuid::Uuid::new_v4(),
            name,
            data_type,
            primary_key_index: if pk > 0 { Some((pk - 1) as u32) } else { None },
            geometry_type: geom_type,
            geometry_crs: geom_crs,
            size,
            length: None,
            precision: None,
            scale: None,
            timezone: None,
        });
    }
    Ok(Schema(columns))
}

// ─── Commands ────────────────────────────────────────────────────────────────

fn cmd_init(dir: &Path, import: Option<&str>, spatial_filter: Option<&str>) -> Result<()> {
    let dir = if dir == Path::new(".") {
        std::env::current_dir()?
    } else {
        std::fs::create_dir_all(dir)?;
        dir.canonicalize()?
    };
    let _repo = Repository::init(&dir)?;

    // Create .gitignore to exclude working copy
    let gitignore = dir.join(".gitignore");
    if !gitignore.exists() {
        std::fs::write(&gitignore, "*.gpkg\n")?;
    }

    println!("Initialized empty GeoGit repository in {}", dir.display());

    if let Some(filter) = spatial_filter {
        let filter_path = dir.join(".geogit");
        std::fs::create_dir_all(&filter_path)?;
        std::fs::write(
            filter_path.join("spatial-filter.json"),
            format!("{{\"bbox\":\"{filter}\"}}"),
        )?;
        println!("Spatial filter set: {filter}");
    }

    if let Some(source) = import {
        std::env::set_current_dir(&dir)?;
        cmd_import(source, None)?;
    }
    Ok(())
}

fn cmd_clone(url: &str, dest: Option<&Path>, spatial_filter: Option<&str>) -> Result<()> {
    let dest = match dest {
        Some(d) => d.to_path_buf(),
        None => {
            let name = url
                .rsplit('/')
                .next()
                .unwrap_or("repo")
                .trim_end_matches(".git");
            PathBuf::from(name)
        }
    };
    println!("Cloning into '{}'...", dest.display());
    let _repo = Repository::clone_repo(url, &dest)?;

    if let Some(filter) = spatial_filter {
        let filter_path = dest.join(".geogit");
        std::fs::create_dir_all(&filter_path)?;
        std::fs::write(
            filter_path.join("spatial-filter.json"),
            format!("{{\"bbox\":\"{filter}\"}}"),
        )?;
        println!("Spatial filter set: {filter}");
    }
    println!("Done.");
    Ok(())
}

fn cmd_import(source: &str, name: Option<&str>) -> Result<()> {
    if source.starts_with("postgresql://") || source.starts_with("postgres://") {
        return import_postgis(source, name);
    }
    let (format, path) = if let Some((f, p)) = source.split_once(':') {
        if f.len() == 1
            && f.chars().next().unwrap_or(' ').is_ascii_alphabetic()
            && p.starts_with('\\')
        {
            // Windows absolute path like C:\foo\bar.gpkg — infer format from extension
            let pb = Path::new(source);
            let ext = pb
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_uppercase();
            let fmt = match ext.as_str() {
                "GPKG" => "GPKG",
                "SHP" => "SHP",
                _ => {
                    return Err(anyhow::anyhow!(
                        "cannot infer format from extension '.{ext}'. Use FORMAT:PATH syntax"
                    ));
                }
            };
            (fmt, source)
        } else {
            (f, p)
        }
    } else {
        // No colon — infer from extension
        let pb = Path::new(source);
        let ext = pb
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_uppercase();
        let fmt = match ext.as_str() {
            "GPKG" => "GPKG",
            "SHP" => "SHP",
            _ => {
                return Err(anyhow::anyhow!(
                    "source must be FORMAT:PATH (e.g. GPKG:data.gpkg, SHP:data.shp) or postgresql://..."
                ));
            }
        };
        (fmt, source)
    };
    match format.to_uppercase().as_str() {
        "GPKG" => import_gpkg(Path::new(path), name),
        "SHP" | "SHAPEFILE" => import_shapefile(Path::new(path), name),
        _ => bail!("unsupported format: {format}. Supported: GPKG, SHP, postgresql://"),
    }
}

fn import_gpkg(gpkg_path: &Path, dataset_name: Option<&str>) -> Result<()> {
    use rusqlite::Connection;
    let gpkg_path = if gpkg_path.is_relative() {
        std::env::current_dir()?.join(gpkg_path)
    } else {
        gpkg_path.to_path_buf()
    };
    let conn = Connection::open(&gpkg_path)
        .with_context(|| format!("failed to open GeoPackage: {}", gpkg_path.display()))?;

    let mut stmt = conn.prepare(
        "SELECT table_name, identifier FROM gpkg_contents WHERE data_type IN ('features', 'attributes')",
    )?;
    let tables: Vec<(String, String)> = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1).unwrap_or_default(),
            ))
        })?
        .filter_map(|r| r.ok())
        .collect();
    if tables.is_empty() {
        bail!("no feature/attribute tables found in GeoPackage");
    }

    let repo_root = find_repo_root()?;
    let repo = Repository::open(&repo_root)?;
    let bar = indicatif::ProgressBar::new(0);
    bar.set_style(
        indicatif::ProgressStyle::default_bar()
            .template("{msg} [{bar:40.cyan/blue}] {pos}/{len}")
            .unwrap()
            .progress_chars("=>-"),
    );

    for (table_name, identifier) in &tables {
        let ds_name = dataset_name.unwrap_or(table_name);
        bar.set_message(format!("Importing {ds_name}"));
        let schema = read_gpkg_schema(&conn, table_name)?;
        let meta = DatasetMeta {
            title: if identifier.is_empty() {
                table_name.clone()
            } else {
                identifier.clone()
            },
            description: String::new(),
            schema: schema.clone(),
            path_structure: PathStructure::default(),
        };

        let col_names: Vec<String> = schema.0.iter().map(|c| format!("\"{}\"", c.name)).collect();
        let select_sql = format!("SELECT {} FROM \"{}\"", col_names.join(", "), table_name);
        let pk_indices: Vec<usize> = schema
            .0
            .iter()
            .enumerate()
            .filter(|(_, c)| c.primary_key_index.is_some())
            .map(|(i, _)| i)
            .collect();

        let mut feat_stmt = conn.prepare(&select_sql)?;
        let mut features = Vec::new();
        let mut wc_features = Vec::new();
        let non_pk_ids: Vec<uuid::Uuid> = schema
            .0
            .iter()
            .filter(|c| c.primary_key_index.is_none())
            .map(|c| c.id)
            .collect();
        let legend = Legend::new(non_pk_ids);
        let legend_hash = legend.hash();

        let mut rows = feat_stmt.query([])?;
        while let Some(row) = rows.next()? {
            let mut values = HashMap::new();
            let mut pk = Vec::new();
            for (i, col) in schema.0.iter().enumerate() {
                let val = read_sqlite_value(row, i)?;
                if pk_indices.contains(&i) {
                    pk.push(val.clone());
                }
                values.insert(col.name.clone(), val);
            }
            let stored_vals: Vec<ColumnValue> = schema
                .0
                .iter()
                .filter(|c| c.primary_key_index.is_none())
                .map(|c| values.get(&c.name).cloned().unwrap_or(ColumnValue::Null))
                .collect();
            let stored = StoredFeature {
                legend_hash: legend_hash.clone(),
                values: stored_vals,
            };
            features.push((pk.clone(), stored));
            wc_features.push((pk, values));
        }
        drop(rows);

        bar.set_length(features.len() as u64);
        let builder = TreeBuilder::new(&repo);
        builder.import_dataset(ds_name, &meta, &features)?;
        let wc_gpkg_path = wc_path(&repo_root);
        let mut wc = GeoPackageWorkingCopy::open(&wc_gpkg_path)?;
        wc.checkout(ds_name, &meta, &wc_features)?;
        bar.set_position(features.len() as u64);
        bar.finish_and_clear();
        println!("Imported {ds_name} ({} features)", features.len());
    }
    println!("\nUse `geogit commit -m \"Initial import\"` to create the first commit.");
    Ok(())
}

fn fmt_xy_list<I>(points: I) -> String
where
    I: IntoIterator<Item = (f64, f64)>,
{
    points
        .into_iter()
        .map(|(x, y)| format!("{x} {y}"))
        .collect::<Vec<_>>()
        .join(",")
}

fn line_wkt<I, P>(parts: I) -> String
where
    I: IntoIterator<Item = P>,
    P: IntoIterator<Item = (f64, f64)>,
{
    let rings: Vec<String> = parts
        .into_iter()
        .map(|part| format!("({})", fmt_xy_list(part)))
        .collect();
    if rings.len() == 1 {
        format!("LINESTRING{}", rings[0])
    } else {
        format!("MULTILINESTRING({})", rings.join(","))
    }
}

fn polygon_wkt<I, P>(rings: I) -> String
where
    I: IntoIterator<Item = P>,
    P: IntoIterator<Item = (f64, f64)>,
{
    let parts: Vec<String> = rings
        .into_iter()
        .map(|ring| format!("({})", fmt_xy_list(ring)))
        .collect();
    format!("POLYGON({})", parts.join(","))
}

fn shape_to_wkt(shape: &shapefile::Shape) -> Option<String> {
    Some(match shape {
        shapefile::Shape::NullShape => return None,
        shapefile::Shape::Point(p) => format!("POINT({} {})", p.x, p.y),
        shapefile::Shape::PointM(p) => format!("POINT({} {})", p.x, p.y),
        shapefile::Shape::PointZ(p) => format!("POINT({} {})", p.x, p.y),
        shapefile::Shape::Polyline(p) => line_wkt(
            p.parts()
                .iter()
                .map(|part| part.iter().map(|pt| (pt.x, pt.y))),
        ),
        shapefile::Shape::PolylineM(p) => line_wkt(
            p.parts()
                .iter()
                .map(|part| part.iter().map(|pt| (pt.x, pt.y))),
        ),
        shapefile::Shape::PolylineZ(p) => line_wkt(
            p.parts()
                .iter()
                .map(|part| part.iter().map(|pt| (pt.x, pt.y))),
        ),
        shapefile::Shape::Polygon(p) => polygon_wkt(
            p.rings()
                .iter()
                .map(|ring| ring.points().iter().map(|pt| (pt.x, pt.y))),
        ),
        shapefile::Shape::PolygonM(p) => polygon_wkt(
            p.rings()
                .iter()
                .map(|ring| ring.points().iter().map(|pt| (pt.x, pt.y))),
        ),
        shapefile::Shape::PolygonZ(p) => polygon_wkt(
            p.rings()
                .iter()
                .map(|ring| ring.points().iter().map(|pt| (pt.x, pt.y))),
        ),
        shapefile::Shape::Multipoint(p) => {
            format!(
                "MULTIPOINT({})",
                fmt_xy_list(p.points().iter().map(|pt| (pt.x, pt.y)))
            )
        }
        shapefile::Shape::MultipointM(p) => {
            format!(
                "MULTIPOINT({})",
                fmt_xy_list(p.points().iter().map(|pt| (pt.x, pt.y)))
            )
        }
        shapefile::Shape::MultipointZ(p) => {
            format!(
                "MULTIPOINT({})",
                fmt_xy_list(p.points().iter().map(|pt| (pt.x, pt.y)))
            )
        }
        shapefile::Shape::Multipatch(_) => return None,
    })
}

fn import_shapefile(shp_path: &Path, dataset_name: Option<&str>) -> Result<()> {
    let shp_path = if shp_path.is_relative() {
        std::env::current_dir()?.join(shp_path)
    } else {
        shp_path.to_path_buf()
    };
    let ds_name = dataset_name.unwrap_or_else(|| {
        shp_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("shapefile")
    });

    let mut reader = shapefile::Reader::from_path(&shp_path)
        .with_context(|| format!("failed to open shapefile: {}", shp_path.display()))?;

    // Get field metadata from the .dbf file directly
    let dbf_path = shp_path.with_extension("dbf");
    let dbf_reader = shapefile::dbase::Reader::from_path(&dbf_path)
        .with_context(|| format!("failed to open .dbf: {}", dbf_path.display()))?;
    let fields: Vec<shapefile::dbase::FieldInfo> = dbf_reader.fields().to_vec();

    // Build schema from dBASE fields
    let mut columns = Vec::new();

    let geometry_type = match reader.header().shape_type {
        shapefile::ShapeType::Point
        | shapefile::ShapeType::PointM
        | shapefile::ShapeType::PointZ => "POINT",
        shapefile::ShapeType::Polyline
        | shapefile::ShapeType::PolylineM
        | shapefile::ShapeType::PolylineZ => "MULTILINESTRING",
        shapefile::ShapeType::Polygon
        | shapefile::ShapeType::PolygonM
        | shapefile::ShapeType::PolygonZ => "MULTIPOLYGON",
        shapefile::ShapeType::Multipoint
        | shapefile::ShapeType::MultipointM
        | shapefile::ShapeType::MultipointZ => "MULTIPOINT",
        _ => "GEOMETRY",
    };

    columns.push(Column {
        id: uuid::Uuid::new_v4(),
        name: "geom".to_string(),
        data_type: DataType::Geometry,
        primary_key_index: None,
        geometry_type: Some(geometry_type.to_string()),
        geometry_crs: Some("EPSG:4326".to_string()),
        size: None,
        length: None,
        precision: None,
        scale: None,
        timezone: None,
    });

    // Add FID as PK
    columns.push(Column {
        id: uuid::Uuid::new_v4(),
        name: "fid".to_string(),
        data_type: DataType::Integer,
        primary_key_index: Some(0),
        geometry_type: None,
        geometry_crs: None,
        size: Some(64),
        length: None,
        precision: None,
        scale: None,
        timezone: None,
    });

    for field in &fields {
        let ft = field.field_type();
        let dt = match ft {
            shapefile::dbase::FieldType::Numeric | shapefile::dbase::FieldType::Float => {
                DataType::Float
            }
            shapefile::dbase::FieldType::Logical => DataType::Boolean,
            shapefile::dbase::FieldType::Date => DataType::Date,
            shapefile::dbase::FieldType::Integer => DataType::Integer,
            _ => DataType::Text,
        };
        columns.push(Column {
            id: uuid::Uuid::new_v4(),
            name: field.name().to_string(),
            data_type: dt,
            primary_key_index: None,
            geometry_type: None,
            geometry_crs: None,
            size: None,
            length: Some(field.length() as u64),
            precision: None,
            scale: None,
            timezone: None,
        });
    }

    let schema = Schema(columns);
    let meta = DatasetMeta {
        title: ds_name.to_string(),
        description: String::new(),
        schema: schema.clone(),
        path_structure: PathStructure::default(),
    };

    let non_pk_ids: Vec<uuid::Uuid> = schema
        .0
        .iter()
        .filter(|c| c.primary_key_index.is_none())
        .map(|c| c.id)
        .collect();
    let legend = Legend::new(non_pk_ids);
    let legend_hash = legend.hash();

    let repo_root = find_repo_root()?;
    let repo = Repository::open(&repo_root)?;

    let mut features = Vec::new();
    let mut wc_features = Vec::new();

    for (fid, result) in (1_i64..).zip(reader.iter_shapes_and_records()) {
        let (shape, record) = result.context("reading shapefile record")?;
        let mut values = HashMap::new();

        let geom_val = match shape_to_wkt(&shape) {
            Some(wkt) => geogit_encoding::geometry::geometry_value_from_wkt(&wkt),
            None => ColumnValue::Null,
        };
        values.insert("geom".to_string(), geom_val);
        values.insert("fid".to_string(), ColumnValue::Integer(fid));

        for (name, value) in record.into_iter() {
            let cv = match value {
                shapefile::dbase::FieldValue::Numeric(Some(v)) => ColumnValue::Float(v),
                shapefile::dbase::FieldValue::Float(Some(v)) => ColumnValue::Float(v as f64),
                shapefile::dbase::FieldValue::Character(Some(s)) => ColumnValue::Text(s),
                shapefile::dbase::FieldValue::Logical(Some(b)) => ColumnValue::Bool(b),
                shapefile::dbase::FieldValue::Date(Some(d)) => {
                    ColumnValue::Text(format!("{:04}-{:02}-{:02}", d.year(), d.month(), d.day()))
                }
                shapefile::dbase::FieldValue::Integer(i) => ColumnValue::Integer(i as i64),
                _ => ColumnValue::Null,
            };
            values.insert(name, cv);
        }

        let pk = vec![ColumnValue::Integer(fid)];
        let stored_vals: Vec<ColumnValue> = schema
            .0
            .iter()
            .filter(|c| c.primary_key_index.is_none())
            .map(|c| values.get(&c.name).cloned().unwrap_or(ColumnValue::Null))
            .collect();
        let stored = StoredFeature {
            legend_hash: legend_hash.clone(),
            values: stored_vals,
        };
        features.push((pk.clone(), stored));
        wc_features.push((pk, values));
    }

    let builder = TreeBuilder::new(&repo);
    builder.import_dataset(ds_name, &meta, &features)?;

    let wc_gpkg_path = wc_path(&repo_root);
    let mut wc = GeoPackageWorkingCopy::open(&wc_gpkg_path)?;
    wc.checkout(ds_name, &meta, &wc_features)?;

    println!(
        "Imported {ds_name} ({} features from Shapefile)",
        features.len()
    );
    println!("Use `geogit commit -m \"Import {ds_name}\"` to commit.");
    Ok(())
}

trait TypedRow {
    fn get_bool(&self, name: &str) -> Option<bool>;
    fn get_i64(&self, name: &str) -> Option<i64>;
    fn get_f64(&self, name: &str) -> Option<f64>;
    fn get_text(&self, name: &str) -> Option<String>;
    fn get_blob(&self, name: &str) -> Option<Vec<u8>>;
}

impl TypedRow for tokio_postgres::Row {
    fn get_bool(&self, name: &str) -> Option<bool> {
        self.try_get::<_, Option<bool>>(name).ok().flatten()
    }

    fn get_i64(&self, name: &str) -> Option<i64> {
        self.try_get::<_, Option<i64>>(name)
            .ok()
            .flatten()
            .or_else(|| {
                self.try_get::<_, Option<i32>>(name)
                    .ok()
                    .flatten()
                    .map(i64::from)
            })
            .or_else(|| {
                self.try_get::<_, Option<i16>>(name)
                    .ok()
                    .flatten()
                    .map(i64::from)
            })
    }

    fn get_f64(&self, name: &str) -> Option<f64> {
        self.try_get::<_, Option<f64>>(name)
            .ok()
            .flatten()
            .or_else(|| {
                self.try_get::<_, Option<f32>>(name)
                    .ok()
                    .flatten()
                    .map(f64::from)
            })
    }

    fn get_text(&self, name: &str) -> Option<String> {
        self.try_get::<_, Option<String>>(name).ok().flatten()
    }

    fn get_blob(&self, name: &str) -> Option<Vec<u8>> {
        self.try_get::<_, Option<Vec<u8>>>(name).ok().flatten()
    }
}

fn read_typed_column(data_type: DataType, row: &impl TypedRow, name: &str) -> ColumnValue {
    match data_type {
        DataType::Boolean => row
            .get_bool(name)
            .map(ColumnValue::Bool)
            .unwrap_or(ColumnValue::Null),
        DataType::Integer => row
            .get_i64(name)
            .map(ColumnValue::Integer)
            .unwrap_or(ColumnValue::Null),
        DataType::Float | DataType::Numeric => row
            .get_f64(name)
            .map(ColumnValue::Float)
            .unwrap_or(ColumnValue::Null),
        DataType::Blob => row
            .get_blob(name)
            .map(ColumnValue::Blob)
            .unwrap_or(ColumnValue::Null),
        DataType::Geometry => row
            .get_text(name)
            .map(|wkt| geogit_encoding::geometry::geometry_value_from_wkt(&wkt))
            .or_else(|| row.get_blob(name).map(ColumnValue::Blob))
            .unwrap_or(ColumnValue::Null),
        DataType::Date
        | DataType::Time
        | DataType::Timestamp
        | DataType::Interval
        | DataType::Text => row
            .get_text(name)
            .map(ColumnValue::Text)
            .unwrap_or(ColumnValue::Null),
    }
}

fn import_postgis(conn_str: &str, dataset_name: Option<&str>) -> Result<()> {
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(async {
        let (client, connection) = tokio_postgres::connect(conn_str, tokio_postgres::NoTls)
            .await
            .context("failed to connect to PostGIS")?;

        tokio::spawn(async move {
            let _ = connection.await;
        });

        // Find tables with geometry columns
        let tables = client
            .query(
                "SELECT f_table_name, f_geometry_column, srid, type
             FROM geometry_columns
             ORDER BY f_table_name",
                &[],
            )
            .await
            .context("failed to query geometry_columns")?;

        if tables.is_empty() {
            bail!("no geometry tables found in the database");
        }

        let repo_root = find_repo_root()?;
        let repo = Repository::open(&repo_root)?;

        for table_row in &tables {
            let table_name: &str = table_row.get(0);
            let geom_col: &str = table_row.get(1);
            let srid: i32 = table_row.get(2);
            let geom_type: &str = table_row.get(3);

            let ds_name = dataset_name.unwrap_or(table_name);

            // Get column info
            let col_rows = client
                .query(
                    "SELECT column_name, data_type, is_nullable
                 FROM information_schema.columns
                 WHERE table_name = $1
                 ORDER BY ordinal_position",
                    &[&table_name],
                )
                .await?;

            let mut columns = Vec::new();
            let mut has_pk = false;
            for col_row in &col_rows {
                let col_name: &str = col_row.get(0);
                let pg_type: &str = col_row.get(1);

                let (dt, gt, crs, size) = if col_name == geom_col {
                    (
                        DataType::Geometry,
                        Some(geom_type.to_string()),
                        Some(format!("EPSG:{srid}")),
                        None,
                    )
                } else {
                    let dt = match pg_type {
                        "integer" | "bigint" | "smallint" => DataType::Integer,
                        "real" | "double precision" => DataType::Float,
                        "numeric" => DataType::Numeric,
                        "boolean" => DataType::Boolean,
                        "bytea" => DataType::Blob,
                        "date" => DataType::Date,
                        "time without time zone" | "time with time zone" => DataType::Time,
                        "timestamp without time zone" | "timestamp with time zone" => {
                            DataType::Timestamp
                        }
                        "interval" => DataType::Interval,
                        _ => DataType::Text,
                    };
                    (dt, None, None, None)
                };

                let is_pk = !has_pk && col_name == "id" || col_name == "gid" || col_name == "fid";
                if is_pk {
                    has_pk = true;
                }

                columns.push(Column {
                    id: uuid::Uuid::new_v4(),
                    name: col_name.to_string(),
                    data_type: dt,
                    primary_key_index: if is_pk { Some(0) } else { None },
                    geometry_type: gt,
                    geometry_crs: crs,
                    size,
                    length: None,
                    precision: None,
                    scale: None,
                    timezone: None,
                });
            }

            // If no PK found, add row number
            if !has_pk {
                columns.insert(
                    0,
                    Column {
                        id: uuid::Uuid::new_v4(),
                        name: "_rowid".to_string(),
                        data_type: DataType::Integer,
                        primary_key_index: Some(0),
                        geometry_type: None,
                        geometry_crs: None,
                        size: Some(64),
                        length: None,
                        precision: None,
                        scale: None,
                        timezone: None,
                    },
                );
            }

            let schema = Schema(columns);
            let meta = DatasetMeta {
                title: ds_name.to_string(),
                description: String::new(),
                schema: schema.clone(),
                path_structure: PathStructure::default(),
            };

            let non_pk_ids: Vec<uuid::Uuid> = schema
                .0
                .iter()
                .filter(|c| c.primary_key_index.is_none())
                .map(|c| c.id)
                .collect();
            let legend = Legend::new(non_pk_ids);
            let legend_hash = legend.hash();

            let select_list: Vec<String> = schema
                .0
                .iter()
                .filter(|c| c.name != "_rowid")
                .map(|c| match c.data_type {
                    DataType::Geometry => {
                        format!("ST_AsText(\"{}\") AS \"{}\"", c.name, c.name)
                    }
                    DataType::Date | DataType::Time | DataType::Timestamp | DataType::Interval => {
                        format!("\"{}\"::text AS \"{}\"", c.name, c.name)
                    }
                    DataType::Numeric => {
                        format!("\"{}\"::double precision AS \"{}\"", c.name, c.name)
                    }
                    _ => format!("\"{}\"", c.name),
                })
                .collect();
            let data_rows = client
                .query(
                    &format!("SELECT {} FROM \"{table_name}\"", select_list.join(", ")),
                    &[],
                )
                .await
                .context("failed to read table data")?;

            let mut features = Vec::new();
            let mut wc_features = Vec::new();
            let mut rowid: i64 = 1;

            for data_row in &data_rows {
                let mut values = HashMap::new();

                if !has_pk {
                    values.insert("_rowid".to_string(), ColumnValue::Integer(rowid));
                }

                for col in &schema.0 {
                    if col.name == "_rowid" {
                        continue;
                    }
                    values.insert(
                        col.name.clone(),
                        read_typed_column(col.data_type, data_row, &col.name),
                    );
                }

                let pk = if has_pk {
                    schema
                        .0
                        .iter()
                        .filter(|c| c.primary_key_index.is_some())
                        .map(|c| values.get(&c.name).cloned().unwrap_or(ColumnValue::Null))
                        .collect()
                } else {
                    vec![ColumnValue::Integer(rowid)]
                };

                let stored_vals: Vec<ColumnValue> = schema
                    .0
                    .iter()
                    .filter(|c| c.primary_key_index.is_none())
                    .map(|c| values.get(&c.name).cloned().unwrap_or(ColumnValue::Null))
                    .collect();
                let stored = StoredFeature {
                    legend_hash: legend_hash.clone(),
                    values: stored_vals,
                };
                features.push((pk.clone(), stored));
                wc_features.push((pk, values));
                rowid += 1;
            }

            let builder = TreeBuilder::new(&repo);
            builder.import_dataset(ds_name, &meta, &features)?;

            let wc_gpkg_path = wc_path(&repo_root);
            let mut wc = GeoPackageWorkingCopy::open(&wc_gpkg_path)?;
            wc.checkout(ds_name, &meta, &wc_features)?;

            println!(
                "Imported {ds_name} ({} features from PostGIS)",
                features.len()
            );
        }
        println!("Use `geogit commit -m \"Import from PostGIS\"` to commit.");
        Ok(())
    })
}

fn cmd_status() -> Result<()> {
    let root = find_repo_root()?;
    let repo = Repository::open(&root)?;
    let branch = repo
        .current_branch()?
        .unwrap_or_else(|| "HEAD detached".into());
    println!("On branch {branch}");
    if root.join(".git/MERGE_HEAD").exists() {
        println!("  (merge in progress — resolve conflicts then `geogit merge --continue`)");
    }
    let wc_gpkg = wc_path(&root);
    if wc_gpkg.exists() {
        let wc = GeoPackageWorkingCopy::open(&wc_gpkg)?;
        let datasets = wc.list_datasets()?;
        let mut total = 0;
        for ds in &datasets {
            let changes = wc.status(ds)?;
            if !changes.is_empty() {
                let i = changes.iter().filter(|d| d.is_insert()).count();
                let u = changes.iter().filter(|d| d.is_update()).count();
                let d = changes.iter().filter(|d| d.is_delete()).count();
                println!("  {ds}/");
                if u > 0 {
                    println!("    modified:   {u} features");
                }
                if i > 0 {
                    println!("    new:        {i} features");
                }
                if d > 0 {
                    println!("    deleted:    {d} features");
                }
                total += changes.len();
            }
        }
        if total == 0 {
            println!("Nothing to commit, working copy clean");
        }
    } else {
        println!("No working copy. Use `geogit checkout` to create one.");
    }
    Ok(())
}

fn cmd_commit(message: &str, filters: &[String]) -> Result<()> {
    let root = find_repo_root()?;
    let repo = Repository::open(&root)?;
    let wc_gpkg = wc_path(&root);
    if wc_gpkg.exists() {
        sync_wc_to_tree(&root, &wc_gpkg, filters)?;
    }
    let result = repo.commit(message)?;
    println!("{result}");
    Ok(())
}

fn cmd_log(oneline: bool, max_count: Option<usize>) -> Result<()> {
    let root = find_repo_root()?;
    let repo = Repository::open(&root)?;
    let entries = repo.log(max_count, oneline)?;
    if entries.is_empty() {
        println!("No commits yet.");
        return Ok(());
    }
    for entry in &entries {
        if oneline {
            println!("{} {}", entry.short_hash, entry.subject);
        } else {
            println!("\x1b[33mcommit {}\x1b[0m", entry.hash);
            println!("Author: {} <{}>", entry.author_name, entry.author_email);
            println!("Date:   {}", entry.date);
            println!();
            println!("    {}", entry.subject);
            if !entry.body.is_empty() {
                for line in entry.body.lines() {
                    println!("    {line}");
                }
            }
            println!();
        }
    }
    Ok(())
}

fn cmd_show(commit: &str) -> Result<()> {
    let root = find_repo_root()?;
    let repo = Repository::open(&root)?;
    print!("{}", repo.show_commit(commit)?);
    Ok(())
}

fn cmd_diff(base: &str, target: Option<&str>, stat: bool) -> Result<()> {
    let root = find_repo_root()?;
    let repo = Repository::open(&root)?;
    if let Some(target) = target {
        let entries = repo.diff_tree(base, target)?;
        if entries.is_empty() {
            println!("No differences.");
        } else {
            print_file_diff(&entries, stat);
        }
    } else {
        let wc_gpkg = wc_path(&root);
        if wc_gpkg.exists() {
            let wc = GeoPackageWorkingCopy::open(&wc_gpkg)?;
            let datasets = wc.list_datasets()?;
            let mut any = false;
            for ds in &datasets {
                let changes = wc.status(ds)?;
                if changes.is_empty() {
                    continue;
                }
                any = true;
                println!("--- {ds} ---");
                if stat {
                    println!(
                        "  {} inserts, {} updates, {} deletes",
                        changes.iter().filter(|d| d.is_insert()).count(),
                        changes.iter().filter(|d| d.is_update()).count(),
                        changes.iter().filter(|d| d.is_delete()).count()
                    );
                } else {
                    for delta in &changes {
                        print_delta(delta);
                    }
                }
            }
            if !any {
                println!("Nothing to diff, working copy clean.");
            }
        } else {
            let entries = repo.diff_working()?;
            if entries.is_empty() {
                println!("No differences.");
            } else {
                print_file_diff(&entries, stat);
            }
        }
    }
    Ok(())
}

fn print_file_diff(entries: &[geogit_git::DiffEntry], stat: bool) {
    if stat {
        let mut ds_changes: HashMap<String, (usize, usize, usize)> = HashMap::new();
        for e in entries {
            let ds = e.path.split("/.table-dataset/").next().unwrap_or(&e.path);
            let c = ds_changes.entry(ds.to_string()).or_default();
            match e.status {
                geogit_git::DiffStatus::Added => c.0 += 1,
                geogit_git::DiffStatus::Modified => c.1 += 1,
                geogit_git::DiffStatus::Deleted => c.2 += 1,
            }
        }
        for (ds, (a, m, d)) in &ds_changes {
            println!("{ds}: {a} added, {m} modified, {d} deleted");
        }
    } else {
        for e in entries {
            let ch = match e.status {
                geogit_git::DiffStatus::Added => "+",
                geogit_git::DiffStatus::Deleted => "-",
                geogit_git::DiffStatus::Modified => "~",
            };
            println!("{ch} {}", e.path);
        }
    }
}

fn print_delta(delta: &geogit_core::diff::FeatureDelta) {
    let pk_str = format_pk(delta.pk());
    match delta {
        geogit_core::diff::FeatureDelta::Insert { new, .. } => {
            println!("  \x1b[32m+ {pk_str}\x1b[0m");
            for (k, v) in new {
                println!("      {k}: {}", format_value(v));
            }
        }
        geogit_core::diff::FeatureDelta::Delete { .. } => {
            println!("  \x1b[31m- {pk_str}\x1b[0m");
        }
        geogit_core::diff::FeatureDelta::Update {
            new,
            old,
            changed_columns,
            ..
        } => {
            println!("  \x1b[33m~ {pk_str}\x1b[0m");
            for col in changed_columns {
                let old_v = old.get(col).map(format_value).unwrap_or_default();
                let new_v = new.get(col).map(format_value).unwrap_or_default();
                println!("      {col}: {old_v} → {new_v}");
            }
        }
    }
}

fn cmd_branch(name: Option<&str>, delete: bool) -> Result<()> {
    let root = find_repo_root()?;
    let repo = Repository::open(&root)?;
    if let Some(name) = name {
        if delete {
            repo.delete_branch(name)?;
            println!("Deleted branch {name}");
        } else {
            repo.create_branch(name, "HEAD")?;
            println!("Created branch {name}");
        }
    } else {
        let branches = repo.branches()?;
        let current = repo.current_branch()?;
        for b in branches {
            let m = if Some(&b.name) == current.as_ref() {
                "* "
            } else {
                "  "
            };
            println!("{m}{}", b.name);
        }
    }
    Ok(())
}

fn cmd_switch(branch: &str, create: bool) -> Result<()> {
    let root = find_repo_root()?;
    let repo = Repository::open(&root)?;
    repo.switch_branch(branch, create)?;
    println!("Switched to branch '{branch}'");
    let wc_gpkg = wc_path(&root);
    if wc_gpkg.exists() {
        refresh_working_copy(&root, &wc_gpkg)?;
    }
    Ok(())
}

fn cmd_merge(branch: &str, abort: bool, cont: bool) -> Result<()> {
    let root = find_repo_root()?;
    let repo = Repository::open(&root)?;
    if abort {
        repo.merge_abort()?;
        println!("Merge aborted.");
        return Ok(());
    }
    if cont {
        // Continue merge: just commit if no conflicts remain
        let conflicts = repo.list_conflicts()?;
        if !conflicts.is_empty() {
            bail!(
                "{} conflict(s) remaining. Resolve them first.",
                conflicts.len()
            );
        }
        let result = repo.commit("Merge commit")?;
        println!("{result}");
        return Ok(());
    }
    let result = repo.merge(branch)?;
    if result.success {
        println!("{}", result.message.trim());
        let wc_gpkg = wc_path(&root);
        if wc_gpkg.exists() {
            refresh_working_copy(&root, &wc_gpkg)?;
        }
    } else {
        println!("{}", result.message.trim());
        if !result.conflicts.is_empty() {
            println!("\nConflicts:");
            for c in &result.conflicts {
                println!("  {c}");
            }
            println!("\nResolve with `geogit resolve` then `geogit merge --continue`.");
        }
    }
    Ok(())
}

fn cmd_push(remote: &str, branch: Option<&str>) -> Result<()> {
    let root = find_repo_root()?;
    let repo = Repository::open(&root)?;
    let out = repo.push(remote, branch)?;
    if out.is_empty() {
        println!("Everything up-to-date");
    } else {
        println!("{out}");
    }
    Ok(())
}

fn cmd_pull(remote: &str, branch: Option<&str>) -> Result<()> {
    let root = find_repo_root()?;
    let repo = Repository::open(&root)?;
    let out = repo.pull(remote, branch)?;
    println!("{out}");
    let wc_gpkg = wc_path(&root);
    if wc_gpkg.exists() {
        refresh_working_copy(&root, &wc_gpkg)?;
    }
    Ok(())
}

fn cmd_remote_add(name: &str, url: &str) -> Result<()> {
    let root = find_repo_root()?;
    Repository::open(&root)?.remote_add(name, url)?;
    println!("Added remote '{name}' → {url}");
    Ok(())
}

fn cmd_remote_remove(name: &str) -> Result<()> {
    let root = find_repo_root()?;
    Repository::open(&root)?.remote_remove(name)?;
    println!("Removed remote '{name}'");
    Ok(())
}

fn cmd_remote_list() -> Result<()> {
    let root = find_repo_root()?;
    let remotes = Repository::open(&root)?.remotes()?;
    if remotes.is_empty() {
        println!("No remotes.");
    } else {
        for r in &remotes {
            println!("  {} → {}", r.name, r.url);
        }
    }
    Ok(())
}

fn cmd_reset(target: &str) -> Result<()> {
    let root = find_repo_root()?;
    Repository::open(&root)?.reset_hard(target)?;
    println!("Reset to {target}");
    let wc_gpkg = wc_path(&root);
    if wc_gpkg.exists() {
        refresh_working_copy(&root, &wc_gpkg)?;
    }
    Ok(())
}

fn cmd_restore(datasets: &[String], source: &str) -> Result<()> {
    let root = find_repo_root()?;
    let repo = Repository::open(&root)?;
    for ds in datasets {
        repo.checkout_path(source, &format!("{ds}/.table-dataset"))?;
        println!("Restored {ds} from {source}");
    }
    let wc_gpkg = wc_path(&root);
    if wc_gpkg.exists() {
        refresh_working_copy(&root, &wc_gpkg)?;
    }
    Ok(())
}

fn cmd_checkout(datasets: &[String]) -> Result<()> {
    let root = find_repo_root()?;
    let wc_gpkg = wc_path(&root);
    let ds_list = if datasets.is_empty() {
        let mut found = Vec::new();
        find_datasets(&root, "", &mut found);
        found
    } else {
        datasets.to_vec()
    };
    if ds_list.is_empty() {
        bail!("No datasets found. Import data first.");
    }

    let spatial_filter = load_spatial_filter(&root);
    let mut wc = GeoPackageWorkingCopy::open(&wc_gpkg)?;
    for ds in &ds_list {
        let meta_dir = root.join(ds).join(".table-dataset/meta");
        if !meta_dir.exists() {
            println!("Warning: '{ds}' not found, skipping.");
            continue;
        }
        let meta = load_dataset_meta(&root, ds)?;
        let feature_dir = root.join(ds).join(".table-dataset/feature");
        let mut features = load_features_from_tree(&feature_dir, &meta)?;
        if let Some(ref bbox) = spatial_filter {
            let before = features.len();
            features.retain(|(_pk, vals)| feature_in_bbox(vals, &meta.schema, bbox));
            if features.len() < before {
                println!(
                    "  (spatial filter applied: {} of {} features)",
                    features.len(),
                    before
                );
            }
        }
        wc.checkout(ds, &meta, &features)?;
        println!("Checked out {ds} ({} features)", features.len());
    }
    println!("\nWorking copy: {}", wc_gpkg.display());
    Ok(())
}

fn cmd_create_workingcopy(target: &str) -> Result<()> {
    let root = find_repo_root()?;
    if target.starts_with("postgresql://") || target.starts_with("postgres://") {
        // PostGIS working copy - store config and push datasets
        let config_dir = root.join(".geogit");
        std::fs::create_dir_all(&config_dir)?;
        std::fs::write(
            config_dir.join("workingcopy.json"),
            format!("{{\"type\":\"postgis\",\"url\":\"{target}\"}}"),
        )?;
        // Push all datasets to PostGIS
        let mut datasets = Vec::new();
        find_datasets(&root, "", &mut datasets);
        if !datasets.is_empty() {
            let rt = tokio::runtime::Runtime::new()?;
            rt.block_on(async {
                let (client, connection) = tokio_postgres::connect(target, tokio_postgres::NoTls)
                    .await
                    .context("failed to connect to PostGIS")?;
                tokio::spawn(async move {
                    let _ = connection.await;
                });
                for ds in &datasets {
                    let meta = load_dataset_meta(&root, ds)?;
                    let feature_dir = root.join(ds).join(".table-dataset/feature");
                    let features = load_features_from_tree(&feature_dir, &meta)?;
                    let table_name = ds.replace('/', "_");
                    // Create table
                    let mut col_defs = Vec::new();
                    for col in &meta.schema.0 {
                        let pg_type = match col.data_type {
                            DataType::Integer => "BIGINT",
                            DataType::Float => "DOUBLE PRECISION",
                            DataType::Boolean => "BOOLEAN",
                            DataType::Blob => "BYTEA",
                            DataType::Date => "DATE",
                            DataType::Timestamp => "TIMESTAMPTZ",
                            DataType::Geometry => "GEOMETRY",
                            _ => "TEXT",
                        };
                        let pk = if col.primary_key_index.is_some() {
                            " PRIMARY KEY"
                        } else {
                            ""
                        };
                        col_defs.push(format!("\"{0}\" {pg_type}{pk}", col.name));
                    }
                    let create_sql = format!(
                        "CREATE TABLE IF NOT EXISTS \"{table_name}\" ({})",
                        col_defs.join(", ")
                    );
                    client.execute(&create_sql, &[]).await?;
                    // Insert features
                    for (_pk, values) in &features {
                        let cols: Vec<String> = meta
                            .schema
                            .0
                            .iter()
                            .map(|c| format!("\"{}\"", c.name))
                            .collect();
                        let params: Vec<String> =
                            (1..=cols.len()).map(|i| format!("${i}")).collect();
                        let insert_sql = format!(
                            "INSERT INTO \"{table_name}\" ({}) VALUES ({}) ON CONFLICT DO NOTHING",
                            cols.join(", "),
                            params.join(", ")
                        );
                        let vals: Vec<String> = meta
                            .schema
                            .0
                            .iter()
                            .map(|c| match values.get(&c.name) {
                                Some(ColumnValue::Text(s)) => s.clone(),
                                Some(ColumnValue::Integer(i)) => i.to_string(),
                                Some(ColumnValue::Float(f)) => f.to_string(),
                                Some(ColumnValue::Bool(b)) => b.to_string(),
                                _ => String::new(),
                            })
                            .collect();
                        let val_refs: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = vals
                            .iter()
                            .map(|v| v as &(dyn tokio_postgres::types::ToSql + Sync))
                            .collect();
                        let _ = client.execute(&insert_sql, &val_refs).await;
                    }
                    println!("  Synced {ds} ({} features) to PostGIS", features.len());
                }
                Ok::<(), anyhow::Error>(())
            })?;
        }
        println!("Configured PostGIS working copy: {target}");
    } else {
        // GeoPackage - just create/refresh
        let path = if Path::new(target).is_absolute() {
            PathBuf::from(target)
        } else {
            root.join(target)
        };
        refresh_working_copy(&root, &path)?;
        println!("Created working copy: {}", path.display());
    }
    Ok(())
}

fn cmd_conflicts_list() -> Result<()> {
    let root = find_repo_root()?;
    let conflicts = Repository::open(&root)?.list_conflicts()?;
    if conflicts.is_empty() {
        println!("No conflicts.");
    } else {
        println!("{} conflict(s):", conflicts.len());
        for c in &conflicts {
            println!("  {c}");
        }
    }
    Ok(())
}

fn cmd_conflicts_abort() -> Result<()> {
    let root = find_repo_root()?;
    Repository::open(&root)?.merge_abort()?;
    println!("Merge aborted.");
    Ok(())
}

fn cmd_resolve(
    conflict: Option<&str>,
    with_strategy: Option<&str>,
    with_file: Option<&Path>,
    theirs: bool,
    ours: bool,
) -> Result<()> {
    let root = find_repo_root()?;
    let repo = Repository::open(&root)?;

    // Handle --with-file (GeoJSON resolution)
    if let Some(file_path) = with_file {
        let conflict = conflict.context("must specify conflict path with --with-file")?;
        let geojson = std::fs::read_to_string(file_path)
            .with_context(|| format!("failed to read {}", file_path.display()))?;
        // Parse GeoJSON and write features to the conflict path
        let parsed: serde_json::Value =
            serde_json::from_str(&geojson).context("invalid GeoJSON")?;
        // Write the GeoJSON content directly to the conflict file to resolve it
        let conflict_path = root.join(conflict);
        if let Some(parent) = conflict_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&conflict_path, serde_json::to_string_pretty(&parsed)?)
            .with_context(|| format!("failed to write resolution to {conflict}"))?;
        // Stage the resolved file
        repo.resolve_conflicts(&[conflict])?;
        println!("Resolved {conflict} with file {}", file_path.display());
        return Ok(());
    }

    let strategy = if theirs {
        "theirs"
    } else if ours {
        "ours"
    } else {
        with_strategy.unwrap_or("ours")
    };

    let conflicts = match conflict {
        Some(c) => vec![c.to_string()],
        None => repo.list_conflicts()?,
    };

    if conflicts.is_empty() {
        println!("No conflicts to resolve.");
        return Ok(());
    }

    match strategy {
        "ours" | "theirs" => {
            let git_strategy = format!("--{strategy}");
            for path in &conflicts {
                let _ = std::process::Command::new("git")
                    .args(["checkout", &git_strategy, "--", path])
                    .current_dir(&root)
                    .output();
            }
        }
        "ancestor" => {
            // Checkout from merge base
            for path in &conflicts {
                let _ = std::process::Command::new("git")
                    .args(["checkout", "MERGE_HEAD~1", "--", path])
                    .current_dir(&root)
                    .output();
            }
        }
        "delete" => {
            for path in &conflicts {
                let full = root.join(path);
                let _ = std::fs::remove_file(&full);
            }
        }
        "workingcopy" => {
            // Accept whatever is in the working copy as-is
        }
        _ => bail!(
            "unknown resolution strategy: {strategy}. Use: ours, theirs, ancestor, delete, workingcopy"
        ),
    }

    let refs: Vec<&str> = conflicts.iter().map(|s| s.as_str()).collect();
    repo.resolve_conflicts(&refs)?;
    println!(
        "Resolved {} conflict(s) with strategy '{strategy}'",
        conflicts.len()
    );
    Ok(())
}

fn cmd_export(
    dataset: Option<&str>,
    destination: Option<&str>,
    list_formats: bool,
    from_ref: Option<&str>,
) -> Result<()> {
    if list_formats {
        println!("Supported export formats:");
        println!("  GPKG      GeoPackage (.gpkg)");
        println!("  SHP       Shapefile (.shp)");
        println!("  CSV       Comma-separated values (.csv)");
        println!("  GEOJSON   GeoJSON (.geojson)");
        return Ok(());
    }

    let dataset = dataset.context("specify dataset to export")?;
    let destination = destination.context("specify destination path or format:path")?;

    let root = find_repo_root()?;

    let (meta, features) = if let Some(ref_name) = from_ref {
        // Export from a specific git ref by reading from that commit's tree
        let repo = Repository::open(&root)?;
        let schema_data = repo
            .read_file_at(
                ref_name,
                &format!("{dataset}/.table-dataset/meta/schema.json"),
            )?
            .context("dataset not found at specified ref")?;
        let schema: Schema = serde_json::from_slice(&schema_data)?;
        let title_data = repo
            .read_file_at(ref_name, &format!("{dataset}/.table-dataset/meta/title"))?
            .unwrap_or_default();
        let ps_data = repo.read_file_at(
            ref_name,
            &format!("{dataset}/.table-dataset/meta/path-structure.json"),
        )?;
        let ps: PathStructure = if let Some(d) = ps_data {
            serde_json::from_slice(&d)?
        } else {
            PathStructure::default()
        };
        let meta = DatasetMeta {
            title: String::from_utf8_lossy(&title_data).trim().to_string(),
            description: String::new(),
            schema,
            path_structure: ps,
        };
        // For ref-based export, we need to checkout to a temp dir
        let tmp = std::env::temp_dir().join(format!("geogit-export-{}", std::process::id()));
        std::fs::create_dir_all(&tmp)?;
        repo.checkout_path(ref_name, &format!("{dataset}/.table-dataset"))?;
        let feature_dir = root.join(dataset).join(".table-dataset/feature");
        let features = load_features_from_tree(&feature_dir, &meta)?;
        // Restore original state
        let _ = repo.checkout_path("HEAD", &format!("{dataset}/.table-dataset"));
        (meta, features)
    } else {
        let meta = load_dataset_meta(&root, dataset)?;
        let feature_dir = root.join(dataset).join(".table-dataset/feature");
        let features = load_features_from_tree(&feature_dir, &meta)?;
        (meta, features)
    };

    // Parse destination format
    // On Windows, skip drive letter (e.g. C:\path) when looking for format prefix
    let (format, path) = if let Some((f, p)) = destination.split_once(':') {
        if f.len() == 1
            && f.chars().next().unwrap_or(' ').is_ascii_alphabetic()
            && p.starts_with('\\')
        {
            // Windows absolute path like C:\foo\bar.csv — infer from extension
            let p = PathBuf::from(destination);
            let ext = p
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_uppercase();
            let fmt = match ext.as_str() {
                "GPKG" => "GPKG",
                "SHP" => "SHP",
                "CSV" => "CSV",
                "GEOJSON" | "JSON" => "GEOJSON",
                _ => "GEOJSON",
            };
            (fmt.to_string(), p)
        } else {
            (f.to_uppercase(), PathBuf::from(p))
        }
    } else {
        // Infer from extension
        let p = PathBuf::from(destination);
        let ext = p
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_uppercase();
        let fmt = match ext.as_str() {
            "GPKG" => "GPKG",
            "SHP" => "SHP",
            "CSV" => "CSV",
            "GEOJSON" | "JSON" => "GEOJSON",
            _ => "GEOJSON",
        };
        (fmt.to_string(), p)
    };

    match format.as_str() {
        "GPKG" => export_gpkg(&path, dataset, &meta, &features)?,
        "CSV" => export_csv(&path, &meta, &features)?,
        "GEOJSON" => export_geojson(&path, &meta, &features)?,
        "SHP" => {
            println!("Shapefile export: writing GeoJSON instead (SHP write not yet supported)");
            let json_path = path.with_extension("geojson");
            export_geojson(&json_path, &meta, &features)?;
        }
        _ => bail!("unsupported export format: {format}"),
    }

    println!(
        "Exported {dataset} ({} features) → {}",
        features.len(),
        path.display()
    );
    Ok(())
}

fn export_gpkg(
    path: &Path,
    dataset: &str,
    meta: &DatasetMeta,
    features: &[FeatureRow],
) -> Result<()> {
    let mut wc = GeoPackageWorkingCopy::open(path)?;
    wc.checkout(dataset, meta, features)?;
    Ok(())
}

fn export_csv(path: &Path, meta: &DatasetMeta, features: &[FeatureRow]) -> Result<()> {
    let mut writer = csv::Writer::from_path(path)?;
    // Header
    let headers: Vec<&str> = meta.schema.0.iter().map(|c| c.name.as_str()).collect();
    writer.write_record(&headers)?;
    // Rows
    for (_pk, values) in features {
        let row: Vec<String> = meta
            .schema
            .0
            .iter()
            .map(|c| values.get(&c.name).map(format_value).unwrap_or_default())
            .collect();
        writer.write_record(&row)?;
    }
    writer.flush()?;
    Ok(())
}

fn export_geojson(path: &Path, meta: &DatasetMeta, features: &[FeatureRow]) -> Result<()> {
    let geom_col = meta
        .schema
        .0
        .iter()
        .find(|c| c.data_type == DataType::Geometry);

    let mut geojson_features = Vec::new();
    for (_pk, values) in features {
        let mut properties = serde_json::Map::new();
        for col in &meta.schema.0 {
            if col.data_type == DataType::Geometry {
                continue;
            }
            let v = values.get(&col.name).cloned().unwrap_or(ColumnValue::Null);
            let json_val = match v {
                ColumnValue::Null => serde_json::Value::Null,
                ColumnValue::Bool(b) => serde_json::Value::Bool(b),
                ColumnValue::Integer(i) => serde_json::json!(i),
                ColumnValue::Float(f) => serde_json::json!(f),
                ColumnValue::Text(s) => serde_json::Value::String(s),
                ColumnValue::Blob(_) => serde_json::Value::String("<blob>".into()),
            };
            properties.insert(col.name.clone(), json_val);
        }

        let geometry = if let Some(gc) = geom_col {
            values
                .get(&gc.name)
                .map(geogit_encoding::geometry::geometry_value_to_geojson)
                .unwrap_or(serde_json::Value::Null)
        } else {
            serde_json::Value::Null
        };

        geojson_features.push(serde_json::json!({
            "type": "Feature",
            "geometry": geometry,
            "properties": properties,
        }));
    }

    let collection = serde_json::json!({
        "type": "FeatureCollection",
        "features": geojson_features,
    });

    std::fs::write(path, serde_json::to_string_pretty(&collection)?)?;
    Ok(())
}

fn cmd_data_ls() -> Result<()> {
    let root = find_repo_root()?;
    let mut datasets = Vec::new();
    find_datasets(&root, "", &mut datasets);
    let mut file_datasets = Vec::new();
    find_file_datasets(&root, "", &mut file_datasets);
    let mut pc_datasets = Vec::new();
    find_pointcloud_datasets(&root, "", &mut pc_datasets);
    let mut raster_datasets = Vec::new();
    find_raster_datasets(&root, "", &mut raster_datasets);

    let total = datasets.len() + file_datasets.len() + pc_datasets.len() + raster_datasets.len();
    if total == 0 {
        println!("No datasets found.");
    } else {
        for ds in &datasets {
            println!("  {ds} (table)");
        }
        for ds in &file_datasets {
            println!("  {ds} (file)");
        }
        for ds in &pc_datasets {
            println!("  {ds} (point cloud)");
        }
        for ds in &raster_datasets {
            println!("  {ds} (raster)");
        }
        println!("\n{total} dataset(s)");
    }
    Ok(())
}

fn cmd_data_info(dataset: &str) -> Result<()> {
    let root = find_repo_root()?;

    // Check for file dataset first
    let file_meta_dir = root.join(dataset).join(".file-dataset/meta");
    if file_meta_dir.exists() {
        let title = std::fs::read_to_string(file_meta_dir.join("title")).unwrap_or_default();
        let description =
            std::fs::read_to_string(file_meta_dir.join("description")).unwrap_or_default();
        let files_dir = root.join(dataset).join(".file-dataset/files");
        let count = count_files(&files_dir);
        println!("Dataset: {dataset}");
        println!("Type: file");
        println!("Title: {}", title.trim());
        if !description.trim().is_empty() {
            println!("Description: {}", description.trim());
        }
        println!("Files: {count}");
        if file_meta_dir.join("metadata.xml").exists() {
            println!("Has metadata: yes");
        }
        if file_meta_dir.join("license").exists() || file_meta_dir.join("license.xml").exists() {
            println!("Has license: yes");
        }
        return Ok(());
    }

    let meta_dir = root.join(dataset).join(".table-dataset/meta");
    if !meta_dir.exists() {
        bail!("dataset '{dataset}' not found");
    }

    let meta = load_dataset_meta(&root, dataset)?;
    let feature_dir = root.join(dataset).join(".table-dataset/feature");
    let count = count_files(&feature_dir);

    println!("Dataset: {dataset}");
    println!("Type: table");
    println!("Title: {}", meta.title);
    if !meta.description.is_empty() {
        println!("Description: {}", meta.description);
    }
    println!("Columns: {}", meta.schema.0.len());
    println!("Features: {count}");
    println!(
        "Primary key: {}",
        meta.schema
            .primary_key_columns()
            .iter()
            .map(|c| c.name.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    );
    for col in &meta.schema.0 {
        if col.data_type == DataType::Geometry {
            println!(
                "Geometry: {} ({}) [{}]",
                col.name,
                col.geometry_type.as_deref().unwrap_or("GEOMETRY"),
                col.geometry_crs.as_deref().unwrap_or("unknown CRS")
            );
        }
    }
    if meta_dir.join("metadata.xml").exists() {
        println!("Has metadata: yes");
    }
    if meta_dir.join("license").exists() || meta_dir.join("license.xml").exists() {
        println!("Has license: yes");
    }
    Ok(())
}

fn cmd_data_schema(dataset: &str) -> Result<()> {
    let root = find_repo_root()?;
    let meta = load_dataset_meta(&root, dataset)?;
    println!("{:<4} {:<20} {:<15} Info", "#", "Name", "Type");
    println!("{}", "-".repeat(60));
    for (i, col) in meta.schema.0.iter().enumerate() {
        let type_str = format!("{:?}", col.data_type).to_lowercase();
        let mut info = Vec::new();
        if col.primary_key_index.is_some() {
            info.push("PK".to_string());
        }
        if let Some(ref gt) = col.geometry_type {
            info.push(gt.clone());
        }
        if let Some(ref crs) = col.geometry_crs {
            info.push(crs.clone());
        }
        if let Some(size) = col.size {
            info.push(format!("{size}-bit"));
        }
        if let Some(len) = col.length {
            info.push(format!("max {len} chars"));
        }
        println!(
            "{:<4} {:<20} {:<15} {}",
            i + 1,
            col.name,
            type_str,
            info.join(", ")
        );
    }
    Ok(())
}

fn cmd_lfs_ls_files(commit: &str, all: bool) -> Result<()> {
    let root = find_repo_root()?;
    let mut args = vec!["lfs", "ls-files"];
    if all {
        args.push("--all");
    } else {
        args.push(commit);
    }
    let output = std::process::Command::new("git")
        .args(&args)
        .current_dir(&root)
        .output()
        .context("failed to run git lfs ls-files (is git-lfs installed?)")?;
    if output.status.success() {
        print!("{}", String::from_utf8_lossy(&output.stdout));
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if stderr.contains("not installed") || stderr.contains("not a git command") {
            println!("Git LFS is not installed. Install with: https://git-lfs.com/");
        } else {
            print!("{stderr}");
        }
    }
    Ok(())
}

fn cmd_lfs_fetch(commits: &[String]) -> Result<()> {
    let root = find_repo_root()?;
    let mut args = vec!["lfs".to_string(), "fetch".to_string(), "origin".to_string()];
    args.extend(commits.iter().cloned());
    let output = std::process::Command::new("git")
        .args(&args)
        .current_dir(&root)
        .output()
        .context("failed to run git lfs fetch")?;
    print!("{}", String::from_utf8_lossy(&output.stdout));
    print!("{}", String::from_utf8_lossy(&output.stderr));
    Ok(())
}

fn cmd_lfs_gc() -> Result<()> {
    let root = find_repo_root()?;
    let output = std::process::Command::new("git")
        .args(["lfs", "prune"])
        .current_dir(&root)
        .output()
        .context("failed to run git lfs prune")?;
    print!("{}", String::from_utf8_lossy(&output.stdout));
    print!("{}", String::from_utf8_lossy(&output.stderr));
    Ok(())
}

// ─── File Dataset Commands ───────────────────────────────────────────────────

fn cmd_files_add(dataset: &str, paths: &[PathBuf]) -> Result<()> {
    let root = find_repo_root()?;
    let ds_dir = root.join(dataset).join(".file-dataset");
    let files_dir = ds_dir.join("files");
    std::fs::create_dir_all(&files_dir)?;

    // Create meta directory with basic info
    let meta_dir = ds_dir.join("meta");
    std::fs::create_dir_all(&meta_dir)?;
    if !meta_dir.join("title").exists() {
        std::fs::write(meta_dir.join("title"), dataset)?;
    }
    if !meta_dir.join("description").exists() {
        std::fs::write(meta_dir.join("description"), "")?;
    }

    for path in paths {
        let abs_path = if path.is_relative() {
            std::env::current_dir()?.join(path)
        } else {
            path.to_path_buf()
        };
        if !abs_path.exists() {
            bail!("file not found: {}", abs_path.display());
        }
        let filename = abs_path
            .file_name()
            .context("invalid file path")?
            .to_string_lossy()
            .to_string();
        let dest = files_dir.join(&filename);
        std::fs::copy(&abs_path, &dest)
            .with_context(|| format!("failed to copy {}", abs_path.display()))?;
        println!("Added: {dataset}/{filename}");
    }

    // Track with git
    std::process::Command::new("git")
        .args(["add", &format!("{dataset}/.file-dataset")])
        .current_dir(&root)
        .output()?;

    Ok(())
}

fn cmd_files_ls(dataset: Option<&str>) -> Result<()> {
    let root = find_repo_root()?;

    let datasets = if let Some(ds) = dataset {
        vec![ds.to_string()]
    } else {
        // Find all file datasets
        let mut results = Vec::new();
        find_file_datasets(&root, "", &mut results);
        results
    };

    if datasets.is_empty() {
        println!("No file datasets found.");
        return Ok(());
    }

    for ds in &datasets {
        let files_dir = root.join(ds).join(".file-dataset/files");
        if !files_dir.exists() {
            continue;
        }
        println!("{ds}/");
        if let Ok(entries) = std::fs::read_dir(&files_dir) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                let meta = entry.metadata()?;
                let size = meta.len();
                let size_str = format_file_size(size);
                println!("  {name}  ({size_str})");
            }
        }
    }
    Ok(())
}

fn cmd_files_rm(dataset: &str, paths: &[PathBuf]) -> Result<()> {
    let root = find_repo_root()?;
    let files_dir = root.join(dataset).join(".file-dataset/files");
    if !files_dir.exists() {
        bail!("file dataset '{dataset}' not found");
    }

    for path in paths {
        let filename = path
            .file_name()
            .unwrap_or(path.as_os_str())
            .to_string_lossy()
            .to_string();
        let target = files_dir.join(&filename);
        if target.exists() {
            std::fs::remove_file(&target)?;
            println!("Removed: {dataset}/{filename}");
        } else {
            println!("Not found: {dataset}/{filename}");
        }
    }

    // Stage removal in git
    std::process::Command::new("git")
        .args(["add", &format!("{dataset}/.file-dataset")])
        .current_dir(&root)
        .output()?;

    Ok(())
}

fn find_file_datasets(dir: &Path, prefix: &str, results: &mut Vec<String>) {
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            if name == ".file-dataset" {
                results.push(prefix.trim_end_matches('/').to_string());
            } else if !name.starts_with('.') && name != "target" && !name.ends_with(".gpkg") {
                let new_prefix = if prefix.is_empty() {
                    name.clone()
                } else {
                    format!("{prefix}/{name}")
                };
                find_file_datasets(&path, &new_prefix, results);
            }
        }
    }
}

fn format_file_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else if bytes < 1024 * 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    } else {
        format!("{:.2} GB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
    }
}

// ─── Metadata Commands ───────────────────────────────────────────────────────

fn cmd_metadata_set(dataset: &str, file: &Path) -> Result<()> {
    let root = find_repo_root()?;
    let meta_dir = root.join(dataset).join(".table-dataset/meta");
    if !meta_dir.exists() {
        // Check for file dataset
        let file_meta = root.join(dataset).join(".file-dataset/meta");
        if file_meta.exists() {
            let content = std::fs::read_to_string(file)
                .with_context(|| format!("failed to read {}", file.display()))?;
            std::fs::write(file_meta.join("metadata.xml"), &content)?;
            println!("Metadata set for dataset '{dataset}'");
            std::process::Command::new("git")
                .args(["add", &format!("{dataset}/.file-dataset/meta/metadata.xml")])
                .current_dir(&root)
                .output()?;
            return Ok(());
        }
        bail!("dataset '{dataset}' not found");
    }

    let content = std::fs::read_to_string(file)
        .with_context(|| format!("failed to read {}", file.display()))?;

    // Validate XML minimally (check for opening tag)
    if !content.trim_start().starts_with('<') {
        bail!("file does not appear to be valid XML");
    }

    std::fs::write(meta_dir.join("metadata.xml"), &content)?;
    println!("Metadata set for dataset '{dataset}'");

    std::process::Command::new("git")
        .args([
            "add",
            &format!("{dataset}/.table-dataset/meta/metadata.xml"),
        ])
        .current_dir(&root)
        .output()?;
    Ok(())
}

fn cmd_metadata_show(dataset: &str) -> Result<()> {
    let root = find_repo_root()?;

    // Try table dataset first, then file dataset
    let paths = [
        root.join(dataset).join(".table-dataset/meta/metadata.xml"),
        root.join(dataset).join(".file-dataset/meta/metadata.xml"),
    ];

    for path in &paths {
        if path.exists() {
            let content = std::fs::read_to_string(path)?;
            println!("{content}");
            return Ok(());
        }
    }

    bail!("no metadata found for dataset '{dataset}'");
}

// ─── License Commands ────────────────────────────────────────────────────────

fn cmd_license_set(dataset: &str, file: &Path) -> Result<()> {
    let root = find_repo_root()?;

    // Determine which dataset type
    let (meta_dir, ds_type) = if root.join(dataset).join(".table-dataset/meta").exists() {
        (
            root.join(dataset).join(".table-dataset/meta"),
            ".table-dataset",
        )
    } else if root.join(dataset).join(".file-dataset/meta").exists() {
        (
            root.join(dataset).join(".file-dataset/meta"),
            ".file-dataset",
        )
    } else {
        bail!("dataset '{dataset}' not found");
    };

    let content = std::fs::read_to_string(file)
        .with_context(|| format!("failed to read {}", file.display()))?;

    // Determine filename based on content
    let license_name = if content.trim_start().starts_with('<') {
        "license.xml"
    } else {
        "license"
    };

    std::fs::write(meta_dir.join(license_name), &content)?;
    println!("License set for dataset '{dataset}'");

    std::process::Command::new("git")
        .args(["add", &format!("{dataset}/{ds_type}/meta/{license_name}")])
        .current_dir(&root)
        .output()?;
    Ok(())
}

fn cmd_license_show(dataset: &str) -> Result<()> {
    let root = find_repo_root()?;

    let candidates = [
        root.join(dataset).join(".table-dataset/meta/license"),
        root.join(dataset).join(".table-dataset/meta/license.xml"),
        root.join(dataset).join(".file-dataset/meta/license"),
        root.join(dataset).join(".file-dataset/meta/license.xml"),
    ];

    for path in &candidates {
        if path.exists() {
            let content = std::fs::read_to_string(path)?;
            println!("{content}");
            return Ok(());
        }
    }

    bail!("no license found for dataset '{dataset}'");
}

// ─── Point Cloud Dataset Commands ────────────────────────────────────────────

/// Compute the tile path for a given filename using SHA256 sharding.
/// Strips known extensions, takes first 2 hex chars of SHA256(name) as shard dir.
fn tile_path(filename: &str, extensions: &[&str]) -> String {
    use sha2::{Digest, Sha256};
    let mut name = filename.to_string();
    for ext in extensions {
        if name.ends_with(ext) {
            name = name[..name.len() - ext.len()].to_string();
            break;
        }
    }
    let hash = Sha256::digest(name.as_bytes());
    let shard = format!("{:02x}", hash[0]);
    format!("{shard}/{name}")
}

fn find_pointcloud_datasets(dir: &Path, prefix: &str, results: &mut Vec<String>) {
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            if name == ".point-cloud-dataset.v1" {
                results.push(prefix.trim_end_matches('/').to_string());
            } else if !name.starts_with('.') && name != "target" && !name.ends_with(".gpkg") {
                let new_prefix = if prefix.is_empty() {
                    name.clone()
                } else {
                    format!("{prefix}/{name}")
                };
                find_pointcloud_datasets(&path, &new_prefix, results);
            }
        }
    }
}

fn find_raster_datasets(dir: &Path, prefix: &str, results: &mut Vec<String>) {
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            if name == ".raster-dataset.v1" {
                results.push(prefix.trim_end_matches('/').to_string());
            } else if !name.starts_with('.') && name != "target" && !name.ends_with(".gpkg") {
                let new_prefix = if prefix.is_empty() {
                    name.clone()
                } else {
                    format!("{prefix}/{name}")
                };
                find_raster_datasets(&path, &new_prefix, results);
            }
        }
    }
}

fn cmd_pointcloud_import(dataset: Option<&str>, files: &[PathBuf]) -> Result<()> {
    if files.is_empty() {
        bail!("no files specified");
    }

    let root = find_repo_root()?;

    // Determine dataset name from --dataset or first file's stem
    let ds_name = if let Some(name) = dataset {
        name.to_string()
    } else {
        files[0]
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "pointcloud".to_string())
    };

    let ds_dir = root.join(&ds_name).join(".point-cloud-dataset.v1");
    let meta_dir = ds_dir.join("meta");
    let tile_dir = ds_dir.join("tile");
    std::fs::create_dir_all(&meta_dir)?;
    std::fs::create_dir_all(&tile_dir)?;

    // Read the first file to extract metadata
    let first_file = &files[0];
    let reader = las::Reader::from_path(first_file)
        .with_context(|| format!("failed to read LAS file: {}", first_file.display()))?;
    let header = reader.header();

    // Write meta/title
    std::fs::write(meta_dir.join("title"), &ds_name)?;

    // Write meta/description
    std::fs::write(meta_dir.join("description"), "")?;

    // Write meta/format.json
    let version = header.version();
    let format = header.point_format();
    let is_compressed = format.is_compressed;
    let pdrf = format.to_u8().unwrap_or(0);
    let point_length = format.len();

    let mut format_json = serde_json::Map::new();
    format_json.insert(
        "compression".to_string(),
        serde_json::Value::String(if is_compressed { "laz" } else { "las" }.to_string()),
    );
    format_json.insert(
        "lasVersion".to_string(),
        serde_json::Value::String(format!("{}.{}", version.major, version.minor)),
    );
    format_json.insert(
        "pointDataRecordFormat".to_string(),
        serde_json::Value::Number(serde_json::Number::from(pdrf)),
    );
    format_json.insert(
        "pointDataRecordLength".to_string(),
        serde_json::Value::Number(serde_json::Number::from(point_length)),
    );
    std::fs::write(
        meta_dir.join("format.json"),
        serde_json::to_string_pretty(&format_json)?,
    )?;

    // Write meta/schema.json - build from point format
    let schema = build_las_schema(format, pdrf);
    std::fs::write(
        meta_dir.join("schema.json"),
        serde_json::to_string_pretty(&schema)?,
    )?;

    // Write meta/crs.wkt if available
    if let Some(wkt_bytes) = header.get_wkt_crs_bytes() {
        let wkt = String::from_utf8_lossy(wkt_bytes);
        std::fs::write(meta_dir.join("crs.wkt"), wkt.as_ref())?;
    }

    // Import tiles
    let pc_extensions = &[".copc.laz", ".laz", ".las"];
    for file in files {
        let filename = file
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        let tp = tile_path(&filename, pc_extensions);
        let dest = tile_dir.join(&tp);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::copy(file, &dest)?;
        println!("  imported tile: {tp}");
    }

    // Track with git lfs if available
    let lfs_pattern = format!("{ds_name}/.point-cloud-dataset.v1/tile/**");
    let _ = std::process::Command::new("git")
        .args(["lfs", "track", &lfs_pattern])
        .current_dir(&root)
        .output();

    println!(
        "\nImported {} tile(s) into point cloud dataset '{ds_name}'",
        files.len()
    );
    Ok(())
}

fn build_las_schema(format: &las::point::Format, pdrf: u8) -> Vec<serde_json::Value> {
    let mut schema = Vec::new();

    // All PDRFs have X, Y, Z (scaled integers stored as 32-bit)
    schema.push(serde_json::json!({"name": "X", "dataType": "integer", "size": 32}));
    schema.push(serde_json::json!({"name": "Y", "dataType": "integer", "size": 32}));
    schema.push(serde_json::json!({"name": "Z", "dataType": "integer", "size": 32}));
    schema.push(serde_json::json!({"name": "Intensity", "dataType": "integer", "size": 16, "unsigned": true}));

    if pdrf >= 6 {
        // Extended format (PDRF 6-10)
        schema.push(serde_json::json!({"name": "Return Number", "dataType": "integer", "size": 4, "unsigned": true}));
        schema.push(serde_json::json!({"name": "Number of Returns", "dataType": "integer", "size": 4, "unsigned": true}));
        schema.push(serde_json::json!({"name": "Synthetic", "dataType": "integer", "size": 1}));
        schema.push(serde_json::json!({"name": "Key-Point", "dataType": "integer", "size": 1}));
        schema.push(serde_json::json!({"name": "Withheld", "dataType": "integer", "size": 1}));
        schema.push(serde_json::json!({"name": "Overlap", "dataType": "integer", "size": 1}));
        schema.push(serde_json::json!({"name": "Scanner Channel", "dataType": "integer", "size": 2, "unsigned": true}));
        schema.push(
            serde_json::json!({"name": "Scan Direction Flag", "dataType": "integer", "size": 1}),
        );
        schema.push(
            serde_json::json!({"name": "Edge of Flight Line", "dataType": "integer", "size": 1}),
        );
        schema.push(serde_json::json!({"name": "Classification", "dataType": "integer", "size": 8, "unsigned": true}));
        schema.push(serde_json::json!({"name": "User Data", "dataType": "integer", "size": 8, "unsigned": true}));
        schema.push(serde_json::json!({"name": "Scan Angle", "dataType": "integer", "size": 16}));
        schema.push(serde_json::json!({"name": "Point Source ID", "dataType": "integer", "size": 16, "unsigned": true}));
    } else {
        // Legacy format (PDRF 0-5)
        schema.push(serde_json::json!({"name": "Return Number", "dataType": "integer", "size": 3, "unsigned": true}));
        schema.push(serde_json::json!({"name": "Number of Returns", "dataType": "integer", "size": 3, "unsigned": true}));
        schema.push(
            serde_json::json!({"name": "Scan Direction Flag", "dataType": "integer", "size": 1}),
        );
        schema.push(
            serde_json::json!({"name": "Edge of Flight Line", "dataType": "integer", "size": 1}),
        );
        schema.push(serde_json::json!({"name": "Classification", "dataType": "integer", "size": 8, "unsigned": true}));
        schema
            .push(serde_json::json!({"name": "Scan Angle Rank", "dataType": "integer", "size": 8}));
        schema.push(serde_json::json!({"name": "User Data", "dataType": "integer", "size": 8, "unsigned": true}));
        schema.push(serde_json::json!({"name": "Point Source ID", "dataType": "integer", "size": 16, "unsigned": true}));
    }

    // GPS Time present in PDRFs 1, 3, 4, 5, 6-10
    if format.has_gps_time {
        schema.push(serde_json::json!({"name": "GPS Time", "dataType": "float", "size": 64}));
    }

    // Color present in PDRFs 2, 3, 5, 7, 8, 10
    if format.has_color {
        schema.push(
            serde_json::json!({"name": "Red", "dataType": "integer", "size": 16, "unsigned": true}),
        );
        schema.push(serde_json::json!({"name": "Green", "dataType": "integer", "size": 16, "unsigned": true}));
        schema.push(serde_json::json!({"name": "Blue", "dataType": "integer", "size": 16, "unsigned": true}));
    }

    // NIR present in PDRFs 8, 10
    if format.has_nir {
        schema.push(serde_json::json!({"name": "Infrared", "dataType": "integer", "size": 16, "unsigned": true}));
    }

    schema
}

fn cmd_pointcloud_ls() -> Result<()> {
    let root = find_repo_root()?;
    let mut datasets = Vec::new();
    find_pointcloud_datasets(&root, "", &mut datasets);

    if datasets.is_empty() {
        println!("No point cloud datasets found.");
    } else {
        for ds in &datasets {
            let tile_dir = root.join(ds).join(".point-cloud-dataset.v1/tile");
            let tile_count = count_files(&tile_dir);
            println!("  {ds} ({tile_count} tile(s))");
        }
        println!("\n{} point cloud dataset(s)", datasets.len());
    }
    Ok(())
}

fn cmd_pointcloud_info(dataset: &str) -> Result<()> {
    let root = find_repo_root()?;
    let ds_dir = root.join(dataset).join(".point-cloud-dataset.v1");
    if !ds_dir.exists() {
        bail!("point cloud dataset '{dataset}' not found");
    }

    let meta_dir = ds_dir.join("meta");

    // Title
    if let Ok(title) = std::fs::read_to_string(meta_dir.join("title")) {
        println!("Title: {}", title.trim());
    }

    // Description
    if let Ok(desc) = std::fs::read_to_string(meta_dir.join("description")) {
        let desc = desc.trim();
        if !desc.is_empty() {
            println!("Description: {desc}");
        }
    }

    // Format
    if let Ok(format_str) = std::fs::read_to_string(meta_dir.join("format.json"))
        && let Ok(format) = serde_json::from_str::<serde_json::Value>(&format_str)
    {
        println!("Format:");
        if let Some(compression) = format.get("compression").and_then(|v| v.as_str()) {
            println!("  Compression: {compression}");
        }
        if let Some(version) = format.get("lasVersion").and_then(|v| v.as_str()) {
            println!("  LAS Version: {version}");
        }
        if let Some(pdrf) = format.get("pointDataRecordFormat").and_then(|v| v.as_u64()) {
            println!("  Point Data Record Format: {pdrf}");
        }
        if let Some(len) = format.get("pointDataRecordLength").and_then(|v| v.as_u64()) {
            println!("  Point Data Record Length: {len}");
        }
    }

    // Schema summary
    if let Ok(schema_str) = std::fs::read_to_string(meta_dir.join("schema.json"))
        && let Ok(schema) = serde_json::from_str::<Vec<serde_json::Value>>(&schema_str)
    {
        println!("Schema: {} field(s)", schema.len());
        for field in &schema {
            let name = field.get("name").and_then(|v| v.as_str()).unwrap_or("?");
            let dt = field
                .get("dataType")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            let size = field.get("size").and_then(|v| v.as_u64()).unwrap_or(0);
            println!("  {name}: {dt}({size})");
        }
    }

    // CRS
    if let Ok(crs) = std::fs::read_to_string(meta_dir.join("crs.wkt")) {
        let crs = crs.trim();
        if !crs.is_empty() {
            // Show first line of CRS
            let first_line = crs.lines().next().unwrap_or(crs);
            println!("CRS: {first_line}");
        }
    }

    // Tile count
    let tile_dir = ds_dir.join("tile");
    let tile_count = count_files(&tile_dir);
    println!("Tiles: {tile_count}");

    Ok(())
}

// ─── Raster Dataset Commands ─────────────────────────────────────────────────

fn cmd_raster_import(dataset: Option<&str>, files: &[PathBuf]) -> Result<()> {
    if files.is_empty() {
        bail!("no files specified");
    }

    let root = find_repo_root()?;

    // Determine dataset name
    let ds_name = if let Some(name) = dataset {
        name.to_string()
    } else {
        files[0]
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "raster".to_string())
    };

    let ds_dir = root.join(&ds_name).join(".raster-dataset.v1");
    let meta_dir = ds_dir.join("meta");
    let tile_dir = ds_dir.join("tile");
    std::fs::create_dir_all(&meta_dir)?;
    std::fs::create_dir_all(&tile_dir)?;

    // Read first file to extract metadata
    let first_file = &files[0];
    let f = std::fs::File::open(first_file)
        .with_context(|| format!("failed to open TIFF: {}", first_file.display()))?;
    let mut decoder = tiff::decoder::Decoder::new(std::io::BufReader::new(f))
        .with_context(|| format!("failed to decode TIFF: {}", first_file.display()))?;

    // Write meta/title
    std::fs::write(meta_dir.join("title"), &ds_name)?;

    // Write meta/description
    std::fs::write(meta_dir.join("description"), "")?;

    // Write meta/format.json
    let format_json = serde_json::json!({
        "fileType": "geotiff"
    });
    std::fs::write(
        meta_dir.join("format.json"),
        serde_json::to_string_pretty(&format_json)?,
    )?;

    // Write meta/schema.json from TIFF metadata
    let schema = build_tiff_schema(&mut decoder)?;
    std::fs::write(
        meta_dir.join("schema.json"),
        serde_json::to_string_pretty(&schema)?,
    )?;

    // Import tiles
    let tiff_extensions = &[".tiff", ".tif"];
    for file in files {
        let filename = file
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        let tp = tile_path(&filename, tiff_extensions);
        let dest = tile_dir.join(&tp);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::copy(file, &dest)?;
        println!("  imported tile: {tp}");
    }

    // Track with git lfs if available
    let lfs_pattern = format!("{ds_name}/.raster-dataset.v1/tile/**");
    let _ = std::process::Command::new("git")
        .args(["lfs", "track", &lfs_pattern])
        .current_dir(&root)
        .output();

    println!(
        "\nImported {} tile(s) into raster dataset '{ds_name}'",
        files.len()
    );
    Ok(())
}

fn build_tiff_schema(
    decoder: &mut tiff::decoder::Decoder<std::io::BufReader<std::fs::File>>,
) -> Result<Vec<serde_json::Value>> {
    use tiff::tags::Tag;

    let samples_per_pixel = decoder.get_tag_u32(Tag::SamplesPerPixel).unwrap_or(1);

    let bits_per_sample = decoder
        .find_tag_unsigned_vec::<u32>(Tag::BitsPerSample)
        .ok()
        .flatten()
        .unwrap_or_else(|| vec![8; samples_per_pixel as usize]);

    let sample_format = decoder
        .find_tag_unsigned_vec::<u32>(Tag::SampleFormat)
        .ok()
        .flatten()
        .unwrap_or_else(|| vec![1; samples_per_pixel as usize]); // 1 = unsigned int

    let photometric = decoder
        .find_tag_unsigned::<u32>(Tag::PhotometricInterpretation)
        .ok()
        .flatten();

    let mut schema = Vec::new();
    for i in 0..samples_per_pixel as usize {
        let bits = *bits_per_sample.get(i).unwrap_or(&8);
        let sf = *sample_format.get(i).unwrap_or(&1);

        let (data_type, unsigned) = match sf {
            1 => ("integer", true),  // unsigned integer
            2 => ("integer", false), // signed integer
            3 => ("float", false),   // floating point
            _ => ("integer", true),  // default to unsigned int
        };

        let mut band = serde_json::Map::new();
        band.insert(
            "dataType".to_string(),
            serde_json::Value::String(data_type.to_string()),
        );
        band.insert(
            "size".to_string(),
            serde_json::Value::Number(serde_json::Number::from(bits)),
        );

        if data_type == "integer" && unsigned {
            band.insert("unsigned".to_string(), serde_json::Value::Bool(true));
        }

        // Add interpretation based on photometric and band index
        if let Some(pi) = photometric {
            let interp = match (pi, i) {
                (2, 0) => Some("red"), // RGB
                (2, 1) => Some("green"),
                (2, 2) => Some("blue"),
                (2, 3) => Some("alpha"),
                (3, _) => Some("palette"), // Palette
                _ => None,
            };
            if let Some(interp) = interp {
                band.insert(
                    "interpretation".to_string(),
                    serde_json::Value::String(interp.to_string()),
                );
            }
        }

        schema.push(serde_json::Value::Object(band));
    }

    Ok(schema)
}

fn cmd_raster_ls() -> Result<()> {
    let root = find_repo_root()?;
    let mut datasets = Vec::new();
    find_raster_datasets(&root, "", &mut datasets);

    if datasets.is_empty() {
        println!("No raster datasets found.");
    } else {
        for ds in &datasets {
            let tile_dir = root.join(ds).join(".raster-dataset.v1/tile");
            let tile_count = count_files(&tile_dir);
            println!("  {ds} ({tile_count} tile(s))");
        }
        println!("\n{} raster dataset(s)", datasets.len());
    }
    Ok(())
}

fn cmd_raster_info(dataset: &str) -> Result<()> {
    let root = find_repo_root()?;
    let ds_dir = root.join(dataset).join(".raster-dataset.v1");
    if !ds_dir.exists() {
        bail!("raster dataset '{dataset}' not found");
    }

    let meta_dir = ds_dir.join("meta");

    // Title
    if let Ok(title) = std::fs::read_to_string(meta_dir.join("title")) {
        println!("Title: {}", title.trim());
    }

    // Description
    if let Ok(desc) = std::fs::read_to_string(meta_dir.join("description")) {
        let desc = desc.trim();
        if !desc.is_empty() {
            println!("Description: {desc}");
        }
    }

    // Format
    if let Ok(format_str) = std::fs::read_to_string(meta_dir.join("format.json"))
        && let Ok(format) = serde_json::from_str::<serde_json::Value>(&format_str)
    {
        println!("Format:");
        if let Some(ft) = format.get("fileType").and_then(|v| v.as_str()) {
            println!("  File Type: {ft}");
        }
        if let Some(profile) = format.get("profile").and_then(|v| v.as_str()) {
            println!("  Profile: {profile}");
        }
    }

    // Schema summary
    if let Ok(schema_str) = std::fs::read_to_string(meta_dir.join("schema.json"))
        && let Ok(schema) = serde_json::from_str::<Vec<serde_json::Value>>(&schema_str)
    {
        println!("Schema: {} band(s)", schema.len());
        for (i, band) in schema.iter().enumerate() {
            let dt = band.get("dataType").and_then(|v| v.as_str()).unwrap_or("?");
            let size = band.get("size").and_then(|v| v.as_u64()).unwrap_or(0);
            let interp = band.get("interpretation").and_then(|v| v.as_str());
            if let Some(interp) = interp {
                println!("  Band {}: {dt}({size}) [{interp}]", i + 1);
            } else {
                println!("  Band {}: {dt}({size})", i + 1);
            }
        }
    }

    // CRS
    if let Ok(crs) = std::fs::read_to_string(meta_dir.join("crs.wkt")) {
        let crs = crs.trim();
        if !crs.is_empty() {
            let first_line = crs.lines().next().unwrap_or(crs);
            println!("CRS: {first_line}");
        }
    }

    // Tile count
    let tile_dir = ds_dir.join("tile");
    let tile_count = count_files(&tile_dir);
    println!("Tiles: {tile_count}");

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct MemRow {
        bools: HashMap<String, bool>,
        ints: HashMap<String, i64>,
        floats: HashMap<String, f64>,
        texts: HashMap<String, String>,
        blobs: HashMap<String, Vec<u8>>,
    }

    impl TypedRow for MemRow {
        fn get_bool(&self, name: &str) -> Option<bool> {
            self.bools.get(name).copied()
        }
        fn get_i64(&self, name: &str) -> Option<i64> {
            self.ints.get(name).copied()
        }
        fn get_f64(&self, name: &str) -> Option<f64> {
            self.floats.get(name).copied()
        }
        fn get_text(&self, name: &str) -> Option<String> {
            self.texts.get(name).cloned()
        }
        fn get_blob(&self, name: &str) -> Option<Vec<u8>> {
            self.blobs.get(name).cloned()
        }
    }

    #[test]
    fn test_read_typed_column_keeps_integers_and_floats() {
        let mut row = MemRow::default();
        row.ints.insert("population".into(), 13960000);
        row.floats.insert("area".into(), 2194.07);
        row.bools.insert("capital".into(), true);
        row.texts.insert("founded".into(), "1457-01-01".into());

        assert_eq!(
            read_typed_column(DataType::Integer, &row, "population"),
            ColumnValue::Integer(13960000)
        );
        assert_eq!(
            read_typed_column(DataType::Float, &row, "area"),
            ColumnValue::Float(2194.07)
        );
        assert_eq!(
            read_typed_column(DataType::Boolean, &row, "capital"),
            ColumnValue::Bool(true)
        );
        assert_eq!(
            read_typed_column(DataType::Date, &row, "founded"),
            ColumnValue::Text("1457-01-01".into())
        );
        assert_eq!(
            read_typed_column(DataType::Integer, &row, "missing"),
            ColumnValue::Null
        );
    }

    #[test]
    fn test_read_typed_column_geometry_from_wkt() {
        let mut row = MemRow::default();
        row.texts
            .insert("geom".into(), "POINT(139.6917 35.6895)".into());
        let value = read_typed_column(DataType::Geometry, &row, "geom");
        match &value {
            ColumnValue::Blob(data) => assert_ne!(data.as_slice(), b"GEOMETRY"),
            ColumnValue::Text(wkt) => {
                assert!(wkt.contains("139.6917"));
                assert_ne!(wkt, "GEOMETRY");
            }
            other => panic!("expected geometry blob or wkt, got {other:?}"),
        }
        let json = geogit_encoding::geometry::geometry_value_to_geojson(&value);
        assert_eq!(json["type"], "Point");
        assert!((json["coordinates"][0].as_f64().unwrap() - 139.6917).abs() < 1e-6);
    }

    #[test]
    fn test_shape_to_wkt_point() {
        let wkt = shape_to_wkt(&shapefile::Shape::Point(shapefile::Point::new(
            139.6917, 35.6895,
        )))
        .unwrap();
        assert!(wkt.starts_with("POINT("));
        assert!(wkt.contains("139.6917"));
        assert!(wkt.contains("35.6895"));
        assert_ne!(wkt, "GEOMETRY");
    }
}
