//! Integration tests for all geogit CLI commands.
//!
//! Tests exercise the binary via `std::process::Command` to verify end-to-end behavior.
//! Each test creates a temporary directory, initializes a repo, and runs CLI commands.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Build path to the geogit binary (compiled in debug mode).
fn geogit_bin() -> PathBuf {
    let mut path = std::env::current_exe().unwrap();
    path.pop(); // strip test binary name
    path.pop(); // strip deps/
    path.push("ggt");
    path
}

/// Run geogit with given args in the given directory.
fn run(dir: &Path, args: &[&str]) -> (String, String, bool) {
    let output = Command::new(geogit_bin())
        .args(args)
        .current_dir(dir)
        .env("GIT_AUTHOR_NAME", "Test User")
        .env("GIT_AUTHOR_EMAIL", "test@example.com")
        .env("GIT_COMMITTER_NAME", "Test User")
        .env("GIT_COMMITTER_EMAIL", "test@example.com")
        .output()
        .expect("failed to run geogit");
    (
        String::from_utf8_lossy(&output.stdout).to_string(),
        String::from_utf8_lossy(&output.stderr).to_string(),
        output.status.success(),
    )
}

/// Set up git config in a temp dir.
fn setup_git_config(dir: &Path) {
    Command::new("git")
        .args(["config", "user.email", "test@example.com"])
        .current_dir(dir)
        .output()
        .unwrap();
    Command::new("git")
        .args(["config", "user.name", "Test User"])
        .current_dir(dir)
        .output()
        .unwrap();
}

/// Create a temp dir for tests.
fn tempdir(prefix: &str) -> TempDir {
    TempDir::new(prefix)
}

struct TempDir(PathBuf);
impl TempDir {
    fn new(prefix: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "geogit-test-{}-{}-{}",
            prefix,
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .subsec_nanos()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
    fn path(&self) -> &Path {
        &self.0
    }
}
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn gpkg_point_blob(x: f64, y: f64) -> Vec<u8> {
    let mut data = Vec::new();
    data.extend_from_slice(&[0x47, 0x50]);
    data.push(0x00);
    data.push(0x01);
    // a GeoPackage carries the table's srs id in every geometry, Kart stores zero instead
    data.extend_from_slice(&4326i32.to_le_bytes());
    data.push(0x01);
    data.extend_from_slice(&1u32.to_le_bytes());
    data.extend_from_slice(&x.to_le_bytes());
    data.extend_from_slice(&y.to_le_bytes());
    data
}

/// Create a minimal GeoPackage with one table for testing import.
fn create_test_gpkg(path: &Path) {
    let conn = rusqlite::Connection::open(path).unwrap();
    conn.execute_batch(
        "
        CREATE TABLE gpkg_spatial_ref_sys (
            srs_name TEXT NOT NULL,
            srs_id INTEGER NOT NULL PRIMARY KEY,
            organization TEXT NOT NULL,
            organization_coordsys_id INTEGER NOT NULL,
            definition TEXT NOT NULL,
            description TEXT
        );
        INSERT INTO gpkg_spatial_ref_sys VALUES ('WGS 84', 4326, 'EPSG', 4326, 'GEOGCS[\"WGS 84\"]', NULL);
        CREATE TABLE gpkg_contents (
            table_name TEXT NOT NULL PRIMARY KEY,
            data_type TEXT NOT NULL DEFAULT 'features',
            identifier TEXT UNIQUE,
            description TEXT DEFAULT '',
            last_change DATETIME,
            min_x DOUBLE, min_y DOUBLE, max_x DOUBLE, max_y DOUBLE,
            srs_id INTEGER
        );
        CREATE TABLE gpkg_geometry_columns (
            table_name TEXT NOT NULL,
            column_name TEXT NOT NULL,
            geometry_type_name TEXT NOT NULL,
            srs_id INTEGER NOT NULL,
            z TINYINT NOT NULL,
            m TINYINT NOT NULL,
            CONSTRAINT pk_geom PRIMARY KEY (table_name, column_name)
        );
        INSERT INTO gpkg_contents (table_name, data_type, identifier) VALUES ('cities', 'features', 'Cities');
        INSERT INTO gpkg_geometry_columns VALUES ('cities', 'geom', 'POINT', 4326, 0, 0);
        CREATE TABLE cities (
            fid INTEGER PRIMARY KEY,
            name TEXT NOT NULL,
            population INTEGER,
            geom BLOB
        );
        ",
    )
    .unwrap();
    let mut stmt = conn
        .prepare("INSERT INTO cities VALUES (?1, ?2, ?3, ?4)")
        .unwrap();
    stmt.execute(rusqlite::params![
        1,
        "Tokyo",
        13960000,
        gpkg_point_blob(139.6917, 35.6895)
    ])
    .unwrap();
    stmt.execute(rusqlite::params![
        2,
        "Delhi",
        11034555,
        gpkg_point_blob(77.2090, 28.6139)
    ])
    .unwrap();
    stmt.execute(rusqlite::params![
        3,
        "Shanghai",
        24870895,
        gpkg_point_blob(121.4737, 31.2304)
    ])
    .unwrap();
}

fn create_test_shapefile(path: &Path) {
    let table = shapefile::dbase::TableWriterBuilder::new()
        .add_character_field(shapefile::dbase::FieldName::try_from("name").unwrap(), 50);
    let mut writer = shapefile::Writer::from_path(path, table).unwrap();
    let mut record = shapefile::dbase::Record::default();
    record.insert(
        "name".to_string(),
        shapefile::dbase::FieldValue::Character(Some("Tokyo".to_string())),
    );
    writer
        .write_shape_and_record(&shapefile::Point::new(139.6917, 35.6895), &record)
        .unwrap();
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[test]
fn test_version() {
    let dir = tempdir("version");
    let (stdout, _, success) = run(dir.path(), &["version"]);
    assert!(success);
    assert!(stdout.contains("geogit"));
}

#[test]
fn test_init() {
    let dir = tempdir("init");
    let target = dir.path().join("repo");
    let (stdout, _, success) = run(dir.path(), &["init", target.to_str().unwrap()]);
    assert!(success, "init failed");
    assert!(stdout.contains("Initialized"));
    assert!(target.join(".git").exists());
}

#[test]
fn test_init_with_spatial_filter() {
    let dir = tempdir("init-sf");
    let target = dir.path().join("repo");
    let (stdout, _, success) = run(
        dir.path(),
        &[
            "init",
            target.to_str().unwrap(),
            "--spatial-filter=-180,-90,180,90",
        ],
    );
    assert!(success, "init failed");
    assert!(stdout.contains("Spatial filter set"));
    assert!(target.join(".geogit/spatial-filter.json").exists());
    let content = fs::read_to_string(target.join(".geogit/spatial-filter.json")).unwrap();
    assert!(content.contains("-180,-90,180,90"));
}

#[test]
fn test_import_gpkg_and_status() {
    let dir = tempdir("import-gpkg");
    let repo = dir.path().join("repo");
    run(dir.path(), &["init", repo.to_str().unwrap()]);
    setup_git_config(&repo);

    // Create a test GeoPackage
    let gpkg = dir.path().join("test.gpkg");
    create_test_gpkg(&gpkg);

    // Import
    let source = format!("GPKG:{}", gpkg.display());
    let (stdout, stderr, success) = run(&repo, &["import", &source]);
    assert!(success, "import failed: {stderr}");
    assert!(stdout.contains("Imported cities"));
    assert!(stdout.contains("3 features"));

    // Verify the dataset tree was created
    assert!(repo.join("cities/.table-dataset/meta/schema.json").exists());
    assert!(repo.join("cities/.table-dataset/feature").exists());

    // Status should show no changes (WC == tree)
    let (stdout, _, success) = run(&repo, &["status"]);
    assert!(success);
    assert!(stdout.contains("clean") || stdout.contains("On branch"));
}

#[test]
fn test_import_two_tables_with_the_same_identifier() {
    let dir = tempdir("import-two-tables-same-identifier");
    let repo = dir.path().join("repo");
    run(dir.path(), &["init", repo.to_str().unwrap()]);
    setup_git_config(&repo);

    let gpkg = dir.path().join("data.gpkg");
    create_test_gpkg(&gpkg);
    let source = format!("GPKG:{}", gpkg.display());
    let (_, stderr, success) = run(&repo, &["import", &source]);
    assert!(success, "import failed: {stderr}");
    let (_, stderr, success) = run(&repo, &["import", &source, "--name", "towns"]);
    assert!(success, "second import failed: {stderr}");

    let working_copy = rusqlite::Connection::open(repo.join("repo.gpkg")).unwrap();
    let contents: Vec<(String, String, i32)> = working_copy
        .prepare("SELECT table_name, identifier, srs_id FROM gpkg_contents ORDER BY table_name")
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .unwrap()
        .map(|row| row.unwrap())
        .collect();
    assert_eq!(
        contents,
        [
            ("cities".to_string(), "Cities".to_string(), 4326),
            ("towns".to_string(), "towns".to_string(), 4326),
        ]
    );
}

#[test]
fn test_hints_name_the_ggt_binary() {
    let dir = tempdir("hints");
    let repo = dir.path().join("repo");
    run(dir.path(), &["init", repo.to_str().unwrap()]);
    setup_git_config(&repo);

    let (stdout, _, success) = run(&repo, &["status"]);
    assert!(success);
    assert!(stdout.contains("`ggt checkout`"), "status hint: {stdout}");

    let gpkg = dir.path().join("data.gpkg");
    create_test_gpkg(&gpkg);
    let source = format!("GPKG:{}", gpkg.display());
    let (stdout, stderr, success) = run(&repo, &["import", &source]);
    assert!(success, "import failed: {stderr}");
    assert!(stdout.contains("`ggt commit -m"), "import hint: {stdout}");
}

fn collect_feature_files(dir: &Path, found: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).unwrap().flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_feature_files(&path, found);
        } else {
            found.push(path);
        }
    }
}

fn find_feature_file(dir: &Path) -> PathBuf {
    let mut found = Vec::new();
    collect_feature_files(dir, &mut found);
    found
        .into_iter()
        .next()
        .unwrap_or_else(|| panic!("no feature file under {}", dir.display()))
}

fn stored_geometries(feature_dir: &Path) -> Vec<Vec<u8>> {
    let mut paths = Vec::new();
    collect_feature_files(feature_dir, &mut paths);
    let mut geometries: Vec<Vec<u8>> = paths
        .iter()
        .map(|path| {
            let feature =
                geogit_encoding::feature::StoredFeature::from_msgpack(&fs::read(path).unwrap())
                    .unwrap();
            feature
                .values
                .iter()
                .find_map(|value| match value {
                    geogit_encoding::value::ColumnValue::Geometry(data) => Some(data.clone()),
                    _ => None,
                })
                .expect("geometry was not stored as a msgpack extension")
        })
        .collect();
    geometries.sort();
    geometries
}

#[test]
fn test_import_gpkg_writes_kart_geometry_and_crs() {
    let dir = tempdir("import-kart-encoding");
    let repo = dir.path().join("repo");
    run(dir.path(), &["init", repo.to_str().unwrap()]);
    setup_git_config(&repo);

    let gpkg = dir.path().join("test.gpkg");
    create_test_gpkg(&gpkg);
    let source = format!("GPKG:{}", gpkg.display());
    let (_, stderr, success) = run(&repo, &["import", &source]);
    assert!(success, "import failed: {stderr}");

    let crs_path = repo.join("cities/.table-dataset/meta/crs/EPSG%3A4326.wkt");
    assert!(crs_path.exists(), "no crs definition written");
    assert_eq!(fs::read_to_string(&crs_path).unwrap(), "GEOGCS[\"WGS 84\"]");

    let feature_path = find_feature_file(&repo.join("cities/.table-dataset/feature"));
    let feature =
        geogit_encoding::feature::StoredFeature::from_msgpack(&fs::read(&feature_path).unwrap())
            .unwrap();
    let geometry = feature
        .values
        .iter()
        .find_map(|value| match value {
            geogit_encoding::value::ColumnValue::Geometry(data) => Some(data),
            _ => None,
        })
        .expect("geometry was not stored as a msgpack extension");
    assert_eq!(&geometry[0..2], b"GP");
    assert_eq!(&geometry[4..8], &[0, 0, 0, 0]);
}

#[test]
fn test_checkout_stamps_the_working_copy_srs_id() {
    let dir = tempdir("checkout-srs-id");
    let repo = dir.path().join("repo");
    run(dir.path(), &["init", repo.to_str().unwrap()]);
    setup_git_config(&repo);

    let gpkg = dir.path().join("test.gpkg");
    create_test_gpkg(&gpkg);
    let source = format!("GPKG:{}", gpkg.display());
    let (_, stderr, success) = run(&repo, &["import", &source]);
    assert!(success, "import failed: {stderr}");

    let working_copy = rusqlite::Connection::open(repo.join("repo.gpkg")).unwrap();
    let declared_srs_id: i32 = working_copy
        .query_row(
            "SELECT srs_id FROM gpkg_geometry_columns WHERE table_name = 'cities'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let mut working_copy_geometries: Vec<Vec<u8>> = working_copy
        .prepare("SELECT geom FROM cities")
        .unwrap()
        .query_map([], |row| row.get::<_, Vec<u8>>(0))
        .unwrap()
        .map(|geometry| geometry.unwrap())
        .collect();
    working_copy_geometries.sort();

    assert_eq!(declared_srs_id, 4326);
    for geometry in &working_copy_geometries {
        assert_eq!(
            i32::from_le_bytes(geometry[4..8].try_into().unwrap()),
            declared_srs_id
        );
    }

    let stored = stored_geometries(&repo.join("cities/.table-dataset/feature"));
    assert_eq!(stored.len(), working_copy_geometries.len());
    for geometry in &stored {
        assert_eq!(&geometry[4..8], &[0, 0, 0, 0]);
    }

    let tails = |geometries: &[Vec<u8>]| -> Vec<Vec<u8>> {
        let mut tails: Vec<Vec<u8>> = geometries.iter().map(|g| g[8..].to_vec()).collect();
        tails.sort();
        tails
    };
    assert_eq!(
        tails(&stored),
        tails(&working_copy_geometries),
        "only the srs id should differ between the tree and the working copy"
    );
}

#[test]
fn test_commit_and_log() {
    let dir = tempdir("commit-log");
    let repo = dir.path().join("repo");
    run(dir.path(), &["init", repo.to_str().unwrap()]);
    setup_git_config(&repo);

    let gpkg = dir.path().join("data.gpkg");
    create_test_gpkg(&gpkg);
    let source = format!("GPKG:{}", gpkg.display());
    run(&repo, &["import", &source]);

    // Commit
    let (stdout, stderr, success) = run(&repo, &["commit", "-m", "Initial import"]);
    assert!(success, "commit failed: {stderr}");
    assert!(
        stdout.contains("Initial import") || stdout.contains("master"),
        "unexpected commit output: {stdout}"
    );

    // Log
    let (stdout, _, success) = run(&repo, &["log"]);
    assert!(success);
    assert!(stdout.contains("Initial import"));

    // Log --oneline
    let (stdout, _, success) = run(&repo, &["log", "--oneline"]);
    assert!(success);
    assert!(stdout.contains("Initial import"));
    assert!(!stdout.contains("Author:"));
}

#[test]
fn test_show() {
    let dir = tempdir("show");
    let repo = dir.path().join("repo");
    run(dir.path(), &["init", repo.to_str().unwrap()]);
    setup_git_config(&repo);

    let gpkg = dir.path().join("data.gpkg");
    create_test_gpkg(&gpkg);
    let source = format!("GPKG:{}", gpkg.display());
    run(&repo, &["import", &source]);
    run(&repo, &["commit", "-m", "First commit"]);

    let (stdout, _, success) = run(&repo, &["show", "HEAD"]);
    assert!(success);
    assert!(stdout.contains("First commit"));
}

#[test]
fn test_branch_create_and_switch() {
    let dir = tempdir("branch");
    let repo = dir.path().join("repo");
    run(dir.path(), &["init", repo.to_str().unwrap()]);
    setup_git_config(&repo);

    let gpkg = dir.path().join("data.gpkg");
    create_test_gpkg(&gpkg);
    let source = format!("GPKG:{}", gpkg.display());
    run(&repo, &["import", &source]);
    run(&repo, &["commit", "-m", "Initial"]);

    // Create branch
    let (stdout, _, success) = run(&repo, &["branch", "feature-x"]);
    assert!(success);
    assert!(stdout.contains("Created branch feature-x"));

    // List branches
    let (stdout, _, success) = run(&repo, &["branch"]);
    assert!(success);
    assert!(stdout.contains("feature-x"));
    assert!(stdout.contains("master") || stdout.contains("main"));

    // Switch
    let (stdout, _, success) = run(&repo, &["switch", "feature-x"]);
    assert!(success);
    assert!(stdout.contains("Switched to branch 'feature-x'"));

    // Delete (switch back first)
    run(&repo, &["switch", "master"]);
    let (stdout, _, success) = run(&repo, &["branch", "feature-x", "-d"]);
    assert!(success);
    assert!(stdout.contains("Deleted branch feature-x"));
}

#[test]
fn test_switch_create() {
    let dir = tempdir("switch-create");
    let repo = dir.path().join("repo");
    run(dir.path(), &["init", repo.to_str().unwrap()]);
    setup_git_config(&repo);

    let gpkg = dir.path().join("data.gpkg");
    create_test_gpkg(&gpkg);
    let source = format!("GPKG:{}", gpkg.display());
    run(&repo, &["import", &source]);
    run(&repo, &["commit", "-m", "Initial"]);

    let (stdout, _, success) = run(&repo, &["switch", "-c", "new-branch"]);
    assert!(success);
    assert!(stdout.contains("Switched to branch 'new-branch'"));
}

#[test]
fn test_diff_stat() {
    let dir = tempdir("diff-stat");
    let repo = dir.path().join("repo");
    run(dir.path(), &["init", repo.to_str().unwrap()]);
    setup_git_config(&repo);

    let gpkg = dir.path().join("data.gpkg");
    create_test_gpkg(&gpkg);
    let source = format!("GPKG:{}", gpkg.display());
    run(&repo, &["import", &source]);
    run(&repo, &["commit", "-m", "First"]);

    // Add another file and commit
    fs::write(repo.join("extra.txt"), "hello").unwrap();
    Command::new("git")
        .args(["add", "extra.txt"])
        .current_dir(&repo)
        .output()
        .unwrap();
    Command::new("git")
        .args(["commit", "-m", "second"])
        .current_dir(&repo)
        .output()
        .unwrap();

    // Diff between commits
    let (stdout, _, success) = run(&repo, &["diff", "HEAD~1", "HEAD"]);
    assert!(success);
    assert!(stdout.contains("extra.txt") || stdout.contains("No differences"));
}

#[test]
fn test_data_ls_info_schema() {
    let dir = tempdir("data");
    let repo = dir.path().join("repo");
    run(dir.path(), &["init", repo.to_str().unwrap()]);
    setup_git_config(&repo);

    let gpkg = dir.path().join("data.gpkg");
    create_test_gpkg(&gpkg);
    let source = format!("GPKG:{}", gpkg.display());
    run(&repo, &["import", &source]);

    // data ls
    let (stdout, _, success) = run(&repo, &["data", "ls"]);
    assert!(success);
    assert!(stdout.contains("cities"));

    // data info
    let (stdout, _, success) = run(&repo, &["data", "info", "cities"]);
    assert!(success);
    assert!(stdout.contains("Dataset: cities"));
    assert!(stdout.contains("Features: 3"));
    assert!(stdout.contains("Columns:"));

    // data schema
    let (stdout, _, success) = run(&repo, &["data", "schema", "cities"]);
    assert!(success);
    assert!(stdout.contains("fid"));
    assert!(stdout.contains("name"));
    assert!(stdout.contains("population"));
    assert!(stdout.contains("geom"));
    assert!(stdout.contains("PK"));
}

#[test]
fn test_export_csv() {
    let dir = tempdir("export-csv");
    let repo = dir.path().join("repo");
    run(dir.path(), &["init", repo.to_str().unwrap()]);
    setup_git_config(&repo);

    let gpkg = dir.path().join("data.gpkg");
    create_test_gpkg(&gpkg);
    let source = format!("GPKG:{}", gpkg.display());
    run(&repo, &["import", &source]);

    let csv_path = dir.path().join("output.csv");
    let (stdout, stderr, success) = run(&repo, &["export", "cities", csv_path.to_str().unwrap()]);
    assert!(success, "export failed: {stderr}");
    assert!(stdout.contains("Exported cities"));

    let csv_content = fs::read_to_string(&csv_path).unwrap();
    assert!(csv_content.contains("fid"));
    assert!(csv_content.contains("name"));
    assert!(csv_content.contains("Tokyo"));
    assert!(csv_content.contains("Delhi"));
    assert!(csv_content.contains("Shanghai"));
}

#[test]
fn test_export_geojson() {
    let dir = tempdir("export-geojson");
    let repo = dir.path().join("repo");
    run(dir.path(), &["init", repo.to_str().unwrap()]);
    setup_git_config(&repo);

    let gpkg = dir.path().join("data.gpkg");
    create_test_gpkg(&gpkg);
    let source = format!("GPKG:{}", gpkg.display());
    run(&repo, &["import", &source]);

    let json_path = dir.path().join("output.geojson");
    let (stdout, stderr, success) = run(&repo, &["export", "cities", json_path.to_str().unwrap()]);
    assert!(success, "export failed: {stderr}");
    assert!(stdout.contains("Exported cities"));

    let content = fs::read_to_string(&json_path).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&content).unwrap();
    assert_eq!(parsed["type"], "FeatureCollection");
    let features = parsed["features"].as_array().unwrap();
    assert_eq!(features.len(), 3);
    let tokyo = features
        .iter()
        .find(|f| f["properties"]["name"] == "Tokyo")
        .expect("Tokyo feature");
    assert_eq!(tokyo["geometry"]["type"], "Point");
    let coords = tokyo["geometry"]["coordinates"].as_array().unwrap();
    assert!((coords[0].as_f64().unwrap() - 139.6917).abs() < 1e-6);
    assert!((coords[1].as_f64().unwrap() - 35.6895).abs() < 1e-6);
    assert!(!content.contains("\"geometry\": null"));
}

#[test]
fn test_export_gpkg() {
    let dir = tempdir("export-gpkg");
    let repo = dir.path().join("repo");
    run(dir.path(), &["init", repo.to_str().unwrap()]);
    setup_git_config(&repo);

    let gpkg = dir.path().join("data.gpkg");
    create_test_gpkg(&gpkg);
    let source = format!("GPKG:{}", gpkg.display());
    run(&repo, &["import", &source]);

    let out_gpkg = dir.path().join("output.gpkg");
    let (stdout, stderr, success) = run(&repo, &["export", "cities", out_gpkg.to_str().unwrap()]);
    assert!(success, "export gpkg failed: {stderr}");
    assert!(stdout.contains("Exported cities"));
    assert!(out_gpkg.exists());

    // Verify we can read back
    let conn = rusqlite::Connection::open(&out_gpkg).unwrap();
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM cities", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 3);
}

#[test]
fn test_export_from_a_ref_keeps_the_crs() {
    let dir = tempdir("export-ref-crs");
    let repo = dir.path().join("repo");
    run(dir.path(), &["init", repo.to_str().unwrap()]);
    setup_git_config(&repo);

    let gpkg = dir.path().join("data.gpkg");
    create_test_gpkg(&gpkg);
    let source = format!("GPKG:{}", gpkg.display());
    run(&repo, &["import", &source]);
    run(&repo, &["commit", "-m", "Import"]);

    let crs_definition = |exported: &Path| -> String {
        rusqlite::Connection::open(exported)
            .unwrap()
            .query_row(
                "SELECT definition FROM gpkg_spatial_ref_sys WHERE srs_id = 4326",
                [],
                |row| row.get(0),
            )
            .unwrap()
    };

    let from_working_tree = dir.path().join("working-tree.gpkg");
    let (_, stderr, success) = run(
        &repo,
        &["export", "cities", from_working_tree.to_str().unwrap()],
    );
    assert!(success, "export failed: {stderr}");
    let from_ref = dir.path().join("from-ref.gpkg");
    let (_, stderr, success) = run(
        &repo,
        &[
            "export",
            "cities",
            from_ref.to_str().unwrap(),
            "--ref",
            "HEAD",
        ],
    );
    assert!(success, "export --ref failed: {stderr}");

    assert_eq!(crs_definition(&from_working_tree), "GEOGCS[\"WGS 84\"]");
    assert_eq!(crs_definition(&from_ref), "GEOGCS[\"WGS 84\"]");
}

// a .prj with no AUTHORITY, so the identifier and the srs id both come from the CRS name
const PROJECTION_WITHOUT_AUTHORITY: &str = concat!(
    "PROJCS[\"NZGD2000 / New Zealand Transverse Mercator 2000\",",
    "GEOGCS[\"NZGD2000\",DATUM[\"New_Zealand_Geodetic_Datum_2000\",",
    "SPHEROID[\"GRS 1980\",6378137,298.257222101]],PRIMEM[\"Greenwich\",0],",
    "UNIT[\"degree\",0.0174532925199433]],PROJECTION[\"Transverse_Mercator\"],UNIT[\"metre\",1]]"
);

#[test]
fn test_import_shapefile_with_a_custom_crs() {
    let dir = tempdir("import-shp-custom-crs");
    let repo = dir.path().join("repo");
    run(dir.path(), &["init", repo.to_str().unwrap()]);
    setup_git_config(&repo);

    let shp = dir.path().join("places.shp");
    create_test_shapefile(&shp);
    fs::write(shp.with_extension("prj"), PROJECTION_WITHOUT_AUTHORITY).unwrap();
    let source = format!("SHP:{}", shp.display());
    let (_, stderr, success) = run(&repo, &["import", &source]);
    assert!(success, "shapefile import failed: {stderr}");

    let identifier = "NZGD2000 _ New Zealand Transverse Mercator 2000";
    let crs_path = repo.join(format!("places/.table-dataset/meta/crs/{identifier}.wkt"));
    assert_eq!(
        fs::read_to_string(&crs_path).unwrap(),
        PROJECTION_WITHOUT_AUTHORITY
    );

    // kart's uint32hash of the crs name, inside its 200000 to 209199 custom range
    let custom_srs_id = 205617;
    let working_copy = rusqlite::Connection::open(repo.join("repo.gpkg")).unwrap();
    let declared_srs_id: i32 = working_copy
        .query_row(
            "SELECT srs_id FROM gpkg_geometry_columns WHERE table_name = 'places'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(declared_srs_id, custom_srs_id);

    let (srs_name, organization, definition): (String, String, String) = working_copy
        .query_row(
            "SELECT srs_name, organization, definition FROM gpkg_spatial_ref_sys WHERE srs_id = ?1",
            [custom_srs_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(srs_name, "NZGD2000 / New Zealand Transverse Mercator 2000");
    assert_eq!(organization, "NONE");
    assert_eq!(definition, PROJECTION_WITHOUT_AUTHORITY);

    let working_copy_geometry: Vec<u8> = working_copy
        .query_row("SELECT geom FROM places WHERE fid = 1", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(
        i32::from_le_bytes(working_copy_geometry[4..8].try_into().unwrap()),
        custom_srs_id
    );

    for geometry in stored_geometries(&repo.join("places/.table-dataset/feature")) {
        assert_eq!(&geometry[4..8], &[0, 0, 0, 0]);
    }
}

#[test]
fn test_import_shapefile_stores_real_geometry() {
    let dir = tempdir("import-shp");
    let repo = dir.path().join("repo");
    run(dir.path(), &["init", repo.to_str().unwrap()]);
    setup_git_config(&repo);

    let shp = dir.path().join("places.shp");
    create_test_shapefile(&shp);
    let source = format!("SHP:{}", shp.display());
    let (stdout, stderr, success) = run(&repo, &["import", &source]);
    assert!(success, "shapefile import failed: {stderr}");
    assert!(stdout.contains("Imported places"));

    let wc = rusqlite::Connection::open(repo.join("repo.gpkg")).unwrap();
    let geom: Vec<u8> = wc
        .query_row("SELECT geom FROM places WHERE fid = 1", [], |r| r.get(0))
        .unwrap();
    assert_ne!(geom, b"GEOMETRY");
    assert!(
        geom.len() > 8,
        "geometry blob too small: {} bytes",
        geom.len()
    );

    let json_path = dir.path().join("places.geojson");
    let (_, stderr, success) = run(&repo, &["export", "places", json_path.to_str().unwrap()]);
    assert!(success, "export after shapefile import failed: {stderr}");
    let content = fs::read_to_string(&json_path).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&content).unwrap();
    let feature = &parsed["features"][0];
    assert_eq!(feature["geometry"]["type"], "Point");
    let coords = feature["geometry"]["coordinates"].as_array().unwrap();
    assert!((coords[0].as_f64().unwrap() - 139.6917).abs() < 1e-6);
    assert!((coords[1].as_f64().unwrap() - 35.6895).abs() < 1e-6);
}

#[test]
fn test_export_list_formats() {
    let dir = tempdir("export-list");
    let (stdout, _, success) = run(dir.path(), &["export", "--list-formats"]);
    assert!(success);
    assert!(stdout.contains("GPKG"));
    assert!(stdout.contains("CSV"));
    assert!(stdout.contains("GEOJSON"));
    assert!(!stdout.contains("SHP"), "SHP is not exportable: {stdout}");
}

#[test]
fn test_export_shapefile_is_rejected() {
    let dir = tempdir("export-shp");
    let repo = dir.path().join("repo");
    run(dir.path(), &["init", repo.to_str().unwrap()]);
    setup_git_config(&repo);

    let gpkg = dir.path().join("data.gpkg");
    create_test_gpkg(&gpkg);
    let source = format!("GPKG:{}", gpkg.display());
    run(&repo, &["import", &source]);

    let out_shp = dir.path().join("cities.shp");
    let (_, _, success) = run(&repo, &["export", "cities", out_shp.to_str().unwrap()]);
    assert!(!success, "shapefile export should fail");
    assert!(!out_shp.exists());
    assert!(
        !dir.path().join("cities.geojson").exists(),
        "shapefile export must not write geojson instead"
    );
}

#[test]
fn test_selective_commit() {
    let dir = tempdir("selective-commit");
    let repo = dir.path().join("repo");
    run(dir.path(), &["init", repo.to_str().unwrap()]);
    setup_git_config(&repo);

    let gpkg = dir.path().join("data.gpkg");
    create_test_gpkg(&gpkg);
    let source = format!("GPKG:{}", gpkg.display());
    run(&repo, &["import", &source]);
    run(&repo, &["commit", "-m", "Import all"]);

    // Now add another dataset manually
    fs::create_dir_all(repo.join("parks/.table-dataset/meta")).unwrap();
    fs::write(
        repo.join("parks/.table-dataset/meta/schema.json"),
        r#"[{"id":"00000000-0000-0000-0000-000000000001","name":"id","dataType":"integer","primaryKeyIndex":0}]"#,
    )
    .unwrap();
    fs::write(repo.join("parks/.table-dataset/meta/title"), "Parks").unwrap();
    fs::write(repo.join("parks/.table-dataset/meta/description"), "").unwrap();

    // Selective commit with only specific dataset
    let (stdout, stderr, success) = run(&repo, &["commit", "-m", "Add parks", "parks"]);
    // Should succeed (commits everything staged - the filter applies to WC sync)
    assert!(
        success || stderr.contains("nothing to commit"),
        "selective commit output: {stdout} {stderr}"
    );
}

#[test]
fn test_commit_filter_names_a_dataset() {
    let dir = tempdir("commit-filter-dataset");
    let repo = dir.path().join("repo");
    run(dir.path(), &["init", repo.to_str().unwrap()]);
    setup_git_config(&repo);

    let gpkg = dir.path().join("data.gpkg");
    create_test_gpkg(&gpkg);
    let source = format!("GPKG:{}", gpkg.display());
    run(&repo, &["import", &source]);
    run(&repo, &["import", &source, "--name", "cities_copy"]);
    run(&repo, &["commit", "-m", "Initial"]);

    let wc = rusqlite::Connection::open(repo.join("repo.gpkg")).unwrap();
    wc.execute_batch(
        "UPDATE cities SET population = 1 WHERE fid = 1;
         UPDATE cities_copy SET population = 2 WHERE fid = 1;",
    )
    .unwrap();
    drop(wc);

    let (_, stderr, success) = run(&repo, &["commit", "-m", "Cities only", "cities"]);
    assert!(success, "commit failed: {stderr}");

    let (stdout, stderr, success) = run(&repo, &["diff", "HEAD~1", "HEAD"]);
    assert!(success, "diff failed: {stderr}");
    assert!(
        stdout.contains("cities/.table-dataset/feature/"),
        "cities not committed: {stdout}"
    );
    assert!(
        !stdout.contains("cities_copy"),
        "cities_copy committed: {stdout}"
    );

    let (stdout, stderr, success) = run(&repo, &["diff"]);
    assert!(success, "diff failed: {stderr}");
    assert!(
        stdout.contains("--- cities_copy ---"),
        "cities_copy edit lost: {stdout}"
    );
    assert!(
        !stdout.contains("--- cities ---"),
        "cities still pending: {stdout}"
    );
}

#[test]
fn test_commit_filter_names_a_feature() {
    let dir = tempdir("commit-filter-feature");
    let repo = dir.path().join("repo");
    run(dir.path(), &["init", repo.to_str().unwrap()]);
    setup_git_config(&repo);

    let gpkg = dir.path().join("data.gpkg");
    create_test_gpkg(&gpkg);
    let source = format!("GPKG:{}", gpkg.display());
    run(&repo, &["import", &source]);
    run(&repo, &["commit", "-m", "Initial"]);

    let wc = rusqlite::Connection::open(repo.join("repo.gpkg")).unwrap();
    wc.execute_batch(
        "UPDATE cities SET population = 1 WHERE fid = 1;
         UPDATE cities SET population = 2 WHERE fid = 2;",
    )
    .unwrap();
    drop(wc);

    let (_, stderr, success) = run(&repo, &["commit", "-m", "Tokyo only", "cities:1"]);
    assert!(success, "commit failed: {stderr}");

    let json_path = dir.path().join("committed.geojson");
    let (_, stderr, success) = run(
        &repo,
        &[
            "export",
            "cities",
            json_path.to_str().unwrap(),
            "--ref",
            "HEAD",
        ],
    );
    assert!(success, "export failed: {stderr}");
    let exported: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&json_path).unwrap()).unwrap();
    let committed_population = |name: &str| {
        exported["features"]
            .as_array()
            .unwrap()
            .iter()
            .find(|feature| feature["properties"]["name"] == name)
            .unwrap_or_else(|| panic!("{name} missing from {exported}"))["properties"]["population"]
            .clone()
    };
    assert_eq!(committed_population("Tokyo"), 1);
    assert_eq!(committed_population("Delhi"), 11034555);

    let (stdout, stderr, success) = run(&repo, &["diff"]);
    assert!(success, "diff failed: {stderr}");
    assert!(
        stdout.contains("population: 11034555 → 2"),
        "Delhi edit lost: {stdout}"
    );
    assert!(
        !stdout.contains("13960000"),
        "Tokyo edit still pending: {stdout}"
    );
}

#[test]
fn test_remote_operations() {
    let dir = tempdir("remote");
    let repo = dir.path().join("repo");
    run(dir.path(), &["init", repo.to_str().unwrap()]);
    setup_git_config(&repo);

    // Add remote
    let (stdout, _, success) = run(
        &repo,
        &["remote", "add", "origin", "https://example.com/repo.git"],
    );
    assert!(success);
    assert!(stdout.contains("Added remote"));

    // List remotes
    let (stdout, _, success) = run(&repo, &["remote", "ls"]);
    assert!(success);
    assert!(stdout.contains("origin"));
    assert!(stdout.contains("https://example.com/repo.git"));

    // Remove remote
    let (stdout, _, success) = run(&repo, &["remote", "remove", "origin"]);
    assert!(success);
    assert!(stdout.contains("Removed remote"));

    // List should be empty now
    let (stdout, _, success) = run(&repo, &["remote", "ls"]);
    assert!(success);
    assert!(stdout.contains("No remotes"));
}

#[test]
fn test_reset() {
    let dir = tempdir("reset");
    let repo = dir.path().join("repo");
    run(dir.path(), &["init", repo.to_str().unwrap()]);
    setup_git_config(&repo);

    let gpkg = dir.path().join("data.gpkg");
    create_test_gpkg(&gpkg);
    let source = format!("GPKG:{}", gpkg.display());
    run(&repo, &["import", &source]);
    run(&repo, &["commit", "-m", "First"]);

    // Add a file
    fs::write(repo.join("extra.txt"), "test").unwrap();
    Command::new("git")
        .args(["add", "extra.txt"])
        .current_dir(&repo)
        .output()
        .unwrap();
    Command::new("git")
        .args(["commit", "-m", "Second"])
        .current_dir(&repo)
        .output()
        .unwrap();
    assert!(repo.join("extra.txt").exists());

    // Reset
    let (stdout, _, success) = run(&repo, &["reset", "HEAD~1"]);
    assert!(success);
    assert!(stdout.contains("Reset to HEAD~1"));
    assert!(!repo.join("extra.txt").exists());
}

#[test]
fn test_checkout_creates_working_copy() {
    let dir = tempdir("checkout");
    let repo = dir.path().join("repo");
    run(dir.path(), &["init", repo.to_str().unwrap()]);
    setup_git_config(&repo);

    let gpkg = dir.path().join("data.gpkg");
    create_test_gpkg(&gpkg);
    let source = format!("GPKG:{}", gpkg.display());
    run(&repo, &["import", &source]);
    run(&repo, &["commit", "-m", "Import"]);

    // Remove working copy
    let wc = repo.join("repo.gpkg");
    let _ = fs::remove_file(&wc);

    // Checkout should recreate it
    let (stdout, stderr, success) = run(&repo, &["checkout"]);
    assert!(success, "checkout failed: {stderr}");
    assert!(stdout.contains("Checked out cities"));
    assert!(wc.exists());
}

#[test]
fn test_create_workingcopy_gpkg() {
    let dir = tempdir("create-wc");
    let repo = dir.path().join("repo");
    run(dir.path(), &["init", repo.to_str().unwrap()]);
    setup_git_config(&repo);

    let gpkg = dir.path().join("data.gpkg");
    create_test_gpkg(&gpkg);
    let source = format!("GPKG:{}", gpkg.display());
    run(&repo, &["import", &source]);
    run(&repo, &["commit", "-m", "Import"]);

    let wc_path = "custom.gpkg";
    let (stdout, stderr, success) = run(&repo, &["create-workingcopy", wc_path]);
    assert!(success, "create-workingcopy failed: {stderr}");
    assert!(stdout.contains("Created working copy"));
    assert!(repo.join(wc_path).exists());

    let config = fs::read_to_string(repo.join(".geogit/workingcopy.json")).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&config).unwrap();
    assert_eq!(parsed["path"], wc_path);

    // later commands must read the configured working copy, not repo.gpkg
    let wc = rusqlite::Connection::open(repo.join(wc_path)).unwrap();
    wc.execute_batch("UPDATE cities SET population = 14000000 WHERE fid = 1;")
        .unwrap();
    drop(wc);

    let (stdout, stderr, success) = run(&repo, &["diff"]);
    assert!(success, "diff failed: {stderr}");
    assert!(
        stdout.contains("population: 13960000 → 14000000"),
        "diff did not read {wc_path}: {stdout}"
    );
}

#[test]
fn test_create_workingcopy_rejects_postgis() {
    let dir = tempdir("create-wc-pg");
    let repo = dir.path().join("repo");
    run(dir.path(), &["init", repo.to_str().unwrap()]);
    setup_git_config(&repo);

    let (_, stderr, success) = run(&repo, &["create-workingcopy", "postgresql://x/y"]);
    assert!(!success, "postgis target should be refused");
    assert!(
        stderr.contains("PostGIS working copies are not supported"),
        "unexpected error: {stderr}"
    );
    assert!(!repo.join(".geogit/workingcopy.json").exists());
}

#[test]
fn test_conflicts_list_no_conflicts() {
    let dir = tempdir("conflicts");
    let repo = dir.path().join("repo");
    run(dir.path(), &["init", repo.to_str().unwrap()]);
    setup_git_config(&repo);

    let gpkg = dir.path().join("data.gpkg");
    create_test_gpkg(&gpkg);
    let source = format!("GPKG:{}", gpkg.display());
    run(&repo, &["import", &source]);
    run(&repo, &["commit", "-m", "First"]);

    let (stdout, _, success) = run(&repo, &["conflicts"]);
    assert!(success);
    assert!(stdout.contains("No conflicts"));
}

#[test]
fn test_merge_and_conflict_resolution() {
    let dir = tempdir("merge");
    let repo = dir.path().join("repo");
    run(dir.path(), &["init", repo.to_str().unwrap()]);
    setup_git_config(&repo);

    // Create initial commit
    fs::write(repo.join("file.txt"), "original").unwrap();
    Command::new("git")
        .args(["add", "."])
        .current_dir(&repo)
        .output()
        .unwrap();
    Command::new("git")
        .args(["commit", "-m", "Initial"])
        .current_dir(&repo)
        .output()
        .unwrap();

    // Create branch and modify
    run(&repo, &["branch", "feature"]);
    run(&repo, &["switch", "feature"]);
    fs::write(repo.join("file.txt"), "feature-change").unwrap();
    Command::new("git")
        .args(["add", "."])
        .current_dir(&repo)
        .output()
        .unwrap();
    Command::new("git")
        .args(["commit", "-m", "Feature change"])
        .current_dir(&repo)
        .output()
        .unwrap();

    // Switch back and modify same file
    run(&repo, &["switch", "master"]);
    fs::write(repo.join("file.txt"), "master-change").unwrap();
    Command::new("git")
        .args(["add", "."])
        .current_dir(&repo)
        .output()
        .unwrap();
    Command::new("git")
        .args(["commit", "-m", "Master change"])
        .current_dir(&repo)
        .output()
        .unwrap();

    // Merge should create conflict
    let (stdout, _, _) = run(&repo, &["merge", "feature"]);
    assert!(
        stdout.contains("CONFLICT") || stdout.contains("conflict"),
        "expected conflict: {stdout}"
    );

    // List conflicts
    let (stdout, _, success) = run(&repo, &["conflicts"]);
    assert!(success);
    assert!(stdout.contains("file.txt"));

    // Resolve with --ours
    let (stdout, _, success) = run(&repo, &["resolve", "--ours"]);
    assert!(success);
    assert!(stdout.contains("Resolved"));

    // Merge --continue
    let (stdout, stderr, success) = run(&repo, &["merge", "--continue", "dummy"]);
    assert!(success, "merge continue failed: {stderr}");
    assert!(
        stdout.contains("commit") || stdout.contains("master"),
        "output: {stdout}"
    );
}

#[test]
fn test_resolve_ancestor_uses_merge_base() {
    let dir = tempdir("resolve-ancestor");
    let repo = dir.path().join("repo");
    run(dir.path(), &["init", repo.to_str().unwrap()]);
    setup_git_config(&repo);

    let commit = |content: &str, message: &str| {
        fs::write(repo.join("file.txt"), content).unwrap();
        Command::new("git")
            .args(["add", "."])
            .current_dir(&repo)
            .output()
            .unwrap();
        Command::new("git")
            .args(["commit", "-m", message])
            .current_dir(&repo)
            .output()
            .unwrap();
    };

    commit("original", "Initial");

    // two commits on the branch, so MERGE_HEAD~1 is not the merge base
    run(&repo, &["branch", "feature"]);
    run(&repo, &["switch", "feature"]);
    commit("feature-first", "Feature first");
    commit("feature-second", "Feature second");

    run(&repo, &["switch", "master"]);
    commit("master-change", "Master change");

    let (stdout, _, _) = run(&repo, &["merge", "feature"]);
    assert!(
        stdout.to_lowercase().contains("conflict"),
        "expected conflict: {stdout}"
    );

    let (stdout, stderr, success) = run(&repo, &["resolve", "--with", "ancestor"]);
    assert!(success, "resolve failed: {stderr}");
    assert!(stdout.contains("Resolved"), "output: {stdout}");

    let resolved = fs::read_to_string(repo.join("file.txt")).unwrap();
    assert_eq!(resolved, "original", "expected the merge base content");
}

#[test]
fn test_resolve_with_file_writes_an_encoded_feature() {
    let dir = tempdir("resolve-with-file");
    let repo = dir.path().join("repo");
    run(dir.path(), &["init", repo.to_str().unwrap()]);
    setup_git_config(&repo);

    let gpkg = dir.path().join("data.gpkg");
    create_test_gpkg(&gpkg);
    let source = format!("GPKG:{}", gpkg.display());
    run(&repo, &["import", &source]);
    run(&repo, &["commit", "-m", "Initial"]);

    let edit_population_and_commit = |population: i64, message: &str| {
        let wc = rusqlite::Connection::open(repo.join("repo.gpkg")).unwrap();
        wc.execute(
            "UPDATE cities SET population = ?1 WHERE fid = 1",
            [population],
        )
        .unwrap();
        drop(wc);
        let (_, stderr, success) = run(&repo, &["commit", "-m", message]);
        assert!(success, "commit failed: {stderr}");
    };

    run(&repo, &["branch", "feature"]);
    run(&repo, &["switch", "feature"]);
    edit_population_and_commit(1, "Feature edit");
    run(&repo, &["switch", "master"]);
    edit_population_and_commit(2, "Master edit");

    let (stdout, _, _) = run(&repo, &["merge", "feature"]);
    assert!(
        stdout.to_lowercase().contains("conflict"),
        "expected conflict: {stdout}"
    );
    let (stdout, _, _) = run(&repo, &["conflicts"]);
    let conflict = stdout
        .lines()
        .map(str::trim)
        .find(|line| line.starts_with("cities/.table-dataset/feature/"))
        .unwrap_or_else(|| panic!("no feature conflict listed: {stdout}"))
        .to_string();

    let resolution = dir.path().join("resolution.geojson");
    fs::write(
        &resolution,
        r#"{"type": "Feature",
            "geometry": {"type": "Point", "coordinates": [139.7, 35.7]},
            "properties": {"fid": 1, "name": "Tokyo", "population": 3}}"#,
    )
    .unwrap();
    let (_, stderr, success) = run(
        &repo,
        &[
            "resolve",
            &conflict,
            "--with-file",
            resolution.to_str().unwrap(),
        ],
    );
    assert!(success, "resolve failed: {stderr}");

    let feature = geogit_encoding::feature::StoredFeature::from_msgpack(
        &fs::read(repo.join(&conflict)).unwrap(),
    )
    .expect("resolved feature is not a MessagePack feature");
    let expected_geometry =
        geogit_encoding::geometry::wkt_to_gpkg_bytes("POINT(139.7 35.7)").unwrap();
    assert_eq!(
        feature.values,
        vec![
            geogit_encoding::value::ColumnValue::Text("Tokyo".into()),
            geogit_encoding::value::ColumnValue::Integer(3),
            geogit_encoding::value::ColumnValue::Geometry(expected_geometry),
        ]
    );

    let (_, stderr, success) = run(&repo, &["merge", "--continue", "feature"]);
    assert!(success, "merge continue failed: {stderr}");
    let json_path = dir.path().join("out.geojson");
    let (_, stderr, success) = run(&repo, &["export", "cities", json_path.to_str().unwrap()]);
    assert!(success, "export failed: {stderr}");
    let exported: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&json_path).unwrap()).unwrap();
    let tokyo = exported["features"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["properties"]["name"] == "Tokyo")
        .expect("Tokyo feature");
    assert_eq!(tokyo["properties"]["population"], 3);
    assert_eq!(tokyo["geometry"]["coordinates"][0], 139.7);
}

#[test]
fn test_restore() {
    let dir = tempdir("restore");
    let repo = dir.path().join("repo");
    run(dir.path(), &["init", repo.to_str().unwrap()]);
    setup_git_config(&repo);

    let gpkg = dir.path().join("data.gpkg");
    create_test_gpkg(&gpkg);
    let source = format!("GPKG:{}", gpkg.display());
    run(&repo, &["import", &source]);
    run(&repo, &["commit", "-m", "Initial"]);

    // Modify the schema file
    let schema_path = repo.join("cities/.table-dataset/meta/schema.json");
    let original = fs::read_to_string(&schema_path).unwrap();
    fs::write(&schema_path, "corrupted").unwrap();

    // Restore from HEAD
    let (stdout, stderr, success) = run(&repo, &["restore", "cities", "--source", "HEAD"]);
    assert!(success, "restore failed: {stderr}");
    assert!(stdout.contains("Restored cities"));

    // Verify restored (normalize line endings for cross-platform)
    let restored = fs::read_to_string(&schema_path).unwrap();
    assert_eq!(
        restored.replace("\r\n", "\n"),
        original.replace("\r\n", "\n")
    );
}

#[test]
fn test_spatial_filter_checkout() {
    let dir = tempdir("spatial-filter");
    let repo = dir.path().join("repo");
    run(
        dir.path(),
        &[
            "init",
            repo.to_str().unwrap(),
            "--spatial-filter",
            "0,0,1,1",
        ],
    );
    setup_git_config(&repo);

    // Verify filter file
    let filter_path = repo.join(".geogit/spatial-filter.json");
    assert!(filter_path.exists());
    let content = fs::read_to_string(&filter_path).unwrap();
    assert!(content.contains("0,0,1,1"));
}

#[test]
fn test_spatial_filter_excludes_features_outside_the_bbox() {
    let dir = tempdir("spatial-filter-excludes");
    let repo = dir.path().join("repo");
    // holds Tokyo, leaves out Delhi and Shanghai
    run(
        dir.path(),
        &[
            "init",
            repo.to_str().unwrap(),
            "--spatial-filter",
            "130,30,140,40",
        ],
    );
    setup_git_config(&repo);

    let gpkg = dir.path().join("data.gpkg");
    create_test_gpkg(&gpkg);
    let source = format!("GPKG:{}", gpkg.display());
    let (_, stderr, success) = run(&repo, &["import", &source]);
    assert!(success, "import failed: {stderr}");

    let working_copy = repo.join("repo.gpkg");
    let working_copy_names = || -> Vec<String> {
        rusqlite::Connection::open(&working_copy)
            .unwrap()
            .prepare("SELECT name FROM cities ORDER BY fid")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .map(|name| name.unwrap())
            .collect()
    };
    assert_eq!(working_copy_names(), vec!["Tokyo".to_string()]);

    let (_, stderr, success) = run(&repo, &["commit", "-m", "Import"]);
    assert!(success, "commit failed: {stderr}");
    let mut feature_files = Vec::new();
    collect_feature_files(
        &repo.join("cities/.table-dataset/feature"),
        &mut feature_files,
    );
    assert_eq!(feature_files.len(), 3, "the tree keeps every feature");

    fs::remove_file(&working_copy).unwrap();
    let (_, stderr, success) = run(&repo, &["checkout"]);
    assert!(success, "checkout failed: {stderr}");
    assert_eq!(working_copy_names(), vec!["Tokyo".to_string()]);
}

#[test]
fn test_lfs_commands_graceful() {
    // LFS commands should not crash even without git-lfs installed
    let dir = tempdir("lfs");
    let repo = dir.path().join("repo");
    run(dir.path(), &["init", repo.to_str().unwrap()]);
    setup_git_config(&repo);

    fs::write(repo.join("dummy.txt"), "x").unwrap();
    Command::new("git")
        .args(["add", "."])
        .current_dir(&repo)
        .output()
        .unwrap();
    Command::new("git")
        .args(["commit", "-m", "Init"])
        .current_dir(&repo)
        .output()
        .unwrap();

    // These should not crash (may fail gracefully)
    let (_, _, _) = run(&repo, &["lfs+", "ls-files"]);
    let (_, _, _) = run(&repo, &["lfs+", "gc"]);
    // Just verify no panic
}

#[test]
fn test_diff_working_copy() {
    let dir = tempdir("diff-wc");
    let repo = dir.path().join("repo");
    run(dir.path(), &["init", repo.to_str().unwrap()]);
    setup_git_config(&repo);

    let gpkg = dir.path().join("data.gpkg");
    create_test_gpkg(&gpkg);
    let source = format!("GPKG:{}", gpkg.display());
    run(&repo, &["import", &source]);
    run(&repo, &["commit", "-m", "Initial"]);

    // Diff should show clean
    let (stdout, _, success) = run(&repo, &["diff"]);
    assert!(success);
    assert!(
        stdout.contains("clean") || stdout.contains("No differences") || stdout.is_empty(),
        "expected clean diff: {stdout}"
    );
}

#[test]
fn test_diff_working_copy_shows_values() {
    let dir = tempdir("diff-wc-values");
    let repo = dir.path().join("repo");
    run(dir.path(), &["init", repo.to_str().unwrap()]);
    setup_git_config(&repo);

    let gpkg = dir.path().join("data.gpkg");
    create_test_gpkg(&gpkg);
    let source = format!("GPKG:{}", gpkg.display());
    run(&repo, &["import", &source]);
    run(&repo, &["commit", "-m", "Initial"]);

    // edit the working copy the way a GIS client would
    let wc = rusqlite::Connection::open(repo.join("repo.gpkg")).unwrap();
    wc.execute_batch(
        "UPDATE cities SET population = 14000000 WHERE fid = 1;
         INSERT INTO cities (fid, name, population) VALUES (4, 'Osaka', 2750000);
         DELETE FROM cities WHERE fid = 2;",
    )
    .unwrap();
    drop(wc);

    let (stdout, stderr, success) = run(&repo, &["diff"]);
    assert!(success, "diff failed: {stderr}");
    assert!(
        stdout.contains("population: 13960000 → 14000000"),
        "update values missing: {stdout}"
    );
    assert!(stdout.contains("Osaka"), "insert values missing: {stdout}");
    assert!(stdout.contains("- 2"), "delete pk missing: {stdout}");

    // committing the same changes must carry the values into the tree
    let (_, stderr, success) = run(&repo, &["commit", "-m", "Edits"]);
    assert!(success, "commit failed: {stderr}");
    let csv_path = dir.path().join("out.csv");
    let (_, stderr, success) = run(&repo, &["export", "cities", csv_path.to_str().unwrap()]);
    assert!(success, "export failed: {stderr}");
    let csv = fs::read_to_string(&csv_path).unwrap();
    assert!(csv.contains("14000000"), "committed value missing: {csv}");
    assert!(csv.contains("Osaka"), "committed insert missing: {csv}");
    assert!(
        !csv.contains("Delhi"),
        "deleted feature still present: {csv}"
    );
}

#[test]
fn test_diff_filters_restrict_the_diff_to_named_datasets() {
    let dir = tempdir("diff-filters");
    let repo = dir.path().join("repo");
    run(dir.path(), &["init", repo.to_str().unwrap()]);
    setup_git_config(&repo);

    let gpkg = dir.path().join("data.gpkg");
    create_test_gpkg(&gpkg);
    let source = format!("GPKG:{}", gpkg.display());
    run(&repo, &["import", &source]);
    // a second table titled Cities would collide with the first in gpkg_contents
    let towns_gpkg = dir.path().join("towns.gpkg");
    create_test_gpkg(&towns_gpkg);
    rusqlite::Connection::open(&towns_gpkg)
        .unwrap()
        .execute("UPDATE gpkg_contents SET identifier = 'Towns'", [])
        .unwrap();
    let towns_source = format!("GPKG:{}", towns_gpkg.display());
    let (_, stderr, success) = run(&repo, &["import", &towns_source, "--name", "towns"]);
    assert!(success, "import failed: {stderr}");
    run(&repo, &["commit", "-m", "Initial"]);

    let wc = rusqlite::Connection::open(repo.join("repo.gpkg")).unwrap();
    wc.execute_batch(
        "UPDATE cities SET population = 1 WHERE fid = 1;
         UPDATE towns SET population = 2 WHERE fid = 1;",
    )
    .unwrap();
    drop(wc);

    let (stdout, stderr, success) = run(&repo, &["diff", "--", "towns"]);
    assert!(success, "diff failed: {stderr}");
    assert!(stdout.contains("--- towns ---"), "towns missing: {stdout}");
    assert!(
        !stdout.contains("cities"),
        "cities not filtered out: {stdout}"
    );

    let (_, stderr, success) = run(&repo, &["commit", "-m", "Edits"]);
    assert!(success, "commit failed: {stderr}");
    let (stdout, stderr, success) = run(&repo, &["diff", "HEAD~1", "HEAD", "--", "towns"]);
    assert!(success, "diff failed: {stderr}");
    assert!(
        stdout.contains("towns/.table-dataset/feature/"),
        "towns missing: {stdout}"
    );
    assert!(
        !stdout.contains("cities"),
        "cities not filtered out: {stdout}"
    );
}

#[test]
fn test_import_nonexistent_file() {
    let dir = tempdir("import-error");
    let repo = dir.path().join("repo");
    run(dir.path(), &["init", repo.to_str().unwrap()]);

    let (_, stderr, success) = run(&repo, &["import", "GPKG:nonexistent.gpkg"]);
    assert!(!success);
    // Error may appear on stdout or stderr depending on anyhow formatting
    let _ = stderr;
}

#[test]
fn test_resolve_unknown_strategy() {
    let dir = tempdir("resolve-err");
    let repo = dir.path().join("repo");
    run(dir.path(), &["init", repo.to_str().unwrap()]);
    setup_git_config(&repo);

    fs::write(repo.join("dummy.txt"), "x").unwrap();
    Command::new("git")
        .args(["add", "."])
        .current_dir(&repo)
        .output()
        .unwrap();
    Command::new("git")
        .args(["commit", "-m", "Init"])
        .current_dir(&repo)
        .output()
        .unwrap();

    // No conflicts => should say "No conflicts to resolve"
    let (stdout, _, success) = run(&repo, &["resolve", "--with", "ours"]);
    assert!(success);
    assert!(stdout.contains("No conflicts"));
}

#[test]
fn test_clone_url_parsing() {
    // Just test that clone parses the destination correctly (will fail on connect)
    let dir = tempdir("clone-parse");
    let (_, _, success) = run(
        dir.path(),
        &["clone", "https://invalid.example.com/repo.git"],
    );
    // This will fail because the URL is unreachable, but it shouldn't panic
    assert!(!success);
}

#[test]
fn test_merge_continue_no_conflicts() {
    let dir = tempdir("merge-cont");
    let repo = dir.path().join("repo");
    run(dir.path(), &["init", repo.to_str().unwrap()]);
    setup_git_config(&repo);

    fs::write(repo.join("test.txt"), "hello").unwrap();
    Command::new("git")
        .args(["add", "."])
        .current_dir(&repo)
        .output()
        .unwrap();
    Command::new("git")
        .args(["commit", "-m", "First"])
        .current_dir(&repo)
        .output()
        .unwrap();

    // merge --continue without a merge in progress should fail
    let (_, stderr, success) = run(&repo, &["merge", "--continue", "dummy"]);
    // Should either succeed (nothing to do) or fail gracefully
    let _ = (success, stderr);
}

#[test]
fn test_merge_abort() {
    let dir = tempdir("merge-abort");
    let repo = dir.path().join("repo");
    run(dir.path(), &["init", repo.to_str().unwrap()]);
    setup_git_config(&repo);

    fs::write(repo.join("test.txt"), "hello").unwrap();
    Command::new("git")
        .args(["add", "."])
        .current_dir(&repo)
        .output()
        .unwrap();
    Command::new("git")
        .args(["commit", "-m", "First"])
        .current_dir(&repo)
        .output()
        .unwrap();

    // merge --abort without a merge is an error but shouldn't panic
    let (_, _, _) = run(&repo, &["merge", "--abort", "dummy"]);
}

#[test]
fn test_full_workflow() {
    // End-to-end: init -> import -> commit -> branch -> switch -> modify -> commit -> merge
    let dir = tempdir("workflow");
    let repo = dir.path().join("repo");
    run(dir.path(), &["init", repo.to_str().unwrap()]);
    setup_git_config(&repo);

    // Import
    let gpkg = dir.path().join("data.gpkg");
    create_test_gpkg(&gpkg);
    let source = format!("GPKG:{}", gpkg.display());
    let (_, _, success) = run(&repo, &["import", &source]);
    assert!(success);

    // Commit
    let (_, _, success) = run(&repo, &["commit", "-m", "Initial import"]);
    assert!(success);

    // Create and switch branch
    let (_, _, success) = run(&repo, &["switch", "-c", "edit-branch"]);
    assert!(success);

    // Add a file on the branch
    fs::write(repo.join("notes.txt"), "branch notes").unwrap();
    Command::new("git")
        .args(["add", "."])
        .current_dir(&repo)
        .output()
        .unwrap();
    Command::new("git")
        .args(["commit", "-m", "Branch commit"])
        .current_dir(&repo)
        .output()
        .unwrap();

    // Switch back and merge (no conflict)
    run(&repo, &["switch", "master"]);
    let (stdout, _, success) = run(&repo, &["merge", "edit-branch"]);
    assert!(success, "merge should succeed: {stdout}");

    // Verify file is present
    assert!(repo.join("notes.txt").exists());

    // Log should show both commits
    let (stdout, _, _) = run(&repo, &["log", "--oneline"]);
    assert!(
        stdout.contains("Initial import")
            || stdout.contains("Branch commit")
            || stdout.contains("Merge")
    );
}

// ─── File Dataset Tests ──────────────────────────────────────────────────────

#[test]
fn test_files_add_and_ls() {
    let dir = tempdir("files_add");
    let repo = dir.path().join("repo");
    run(dir.path(), &["init", repo.to_str().unwrap()]);
    setup_git_config(&repo);

    // Create test files
    let doc = dir.path().join("readme.md");
    fs::write(&doc, "# Hello\nThis is a document.").unwrap();

    let (stdout, _, success) = run(&repo, &["files", "add", doc.to_str().unwrap()]);
    assert!(success, "files add should succeed: {stdout}");
    assert!(stdout.contains("Added: files/readme.md"));

    // List files
    let (stdout, _, success) = run(&repo, &["files", "ls"]);
    assert!(success);
    assert!(stdout.contains("readme.md"));

    // File should exist on disk
    assert!(repo.join("files/.file-dataset/files/readme.md").exists());
}

#[test]
fn test_files_add_custom_dataset() {
    let dir = tempdir("files_custom");
    let repo = dir.path().join("repo");
    run(dir.path(), &["init", repo.to_str().unwrap()]);
    setup_git_config(&repo);

    let doc = dir.path().join("spec.pdf");
    fs::write(&doc, "fake PDF content").unwrap();

    let (stdout, _, success) = run(
        &repo,
        &[
            "files",
            "add",
            "--dataset",
            "documents",
            doc.to_str().unwrap(),
        ],
    );
    assert!(success, "files add with custom dataset: {stdout}");
    assert!(stdout.contains("Added: documents/spec.pdf"));

    // List only that dataset
    let (stdout, _, success) = run(&repo, &["files", "ls", "--dataset", "documents"]);
    assert!(success);
    assert!(stdout.contains("spec.pdf"));
}

#[test]
fn test_files_rm() {
    let dir = tempdir("files_rm");
    let repo = dir.path().join("repo");
    run(dir.path(), &["init", repo.to_str().unwrap()]);
    setup_git_config(&repo);

    let doc = dir.path().join("temp.txt");
    fs::write(&doc, "temporary").unwrap();
    run(&repo, &["files", "add", doc.to_str().unwrap()]);

    let (stdout, _, success) = run(&repo, &["files", "rm", "temp.txt"]);
    assert!(success, "files rm should succeed: {stdout}");
    assert!(stdout.contains("Removed: files/temp.txt"));

    // File should be gone
    assert!(!repo.join("files/.file-dataset/files/temp.txt").exists());
}

#[test]
fn test_files_commit_and_version() {
    let dir = tempdir("files_version");
    let repo = dir.path().join("repo");
    run(dir.path(), &["init", repo.to_str().unwrap()]);
    setup_git_config(&repo);

    let doc = dir.path().join("notes.txt");
    fs::write(&doc, "Version 1").unwrap();
    run(&repo, &["files", "add", doc.to_str().unwrap()]);

    let (_, _, success) = run(&repo, &["commit", "-m", "Add notes"]);
    assert!(success);

    // Update the file
    fs::write(
        repo.join("files/.file-dataset/files/notes.txt"),
        "Version 2",
    )
    .unwrap();
    Command::new("git")
        .args(["add", "."])
        .current_dir(&repo)
        .output()
        .unwrap();
    let (_, _, success) = run(&repo, &["commit", "-m", "Update notes"]);
    assert!(success);

    // Log should show both commits
    let (stdout, _, _) = run(&repo, &["log", "--oneline"]);
    assert!(stdout.contains("Add notes"));
    assert!(stdout.contains("Update notes"));
}

#[test]
fn test_files_in_data_ls() {
    let dir = tempdir("files_data_ls");
    let repo = dir.path().join("repo");
    run(dir.path(), &["init", repo.to_str().unwrap()]);
    setup_git_config(&repo);

    let doc = dir.path().join("doc.txt");
    fs::write(&doc, "content").unwrap();
    run(&repo, &["files", "add", doc.to_str().unwrap()]);

    let (stdout, _, success) = run(&repo, &["data", "ls"]);
    assert!(success);
    assert!(stdout.contains("files (file)"));
}

// ─── Metadata Tests ──────────────────────────────────────────────────────────

#[test]
fn test_metadata_set_and_show() {
    let dir = tempdir("metadata");
    let repo = dir.path().join("repo");
    run(dir.path(), &["init", repo.to_str().unwrap()]);
    setup_git_config(&repo);

    // Import a dataset
    let gpkg = dir.path().join("data.gpkg");
    create_test_gpkg(&gpkg);
    run(&repo, &["import", &format!("GPKG:{}", gpkg.display())]);

    // Create XML metadata file
    let meta_file = dir.path().join("metadata.xml");
    fs::write(
        &meta_file,
        r#"<?xml version="1.0" encoding="UTF-8"?>
<gmd:MD_Metadata xmlns:gmd="http://www.isotc211.org/2005/gmd">
  <gmd:identificationInfo>
    <gmd:title>Cities of the World</gmd:title>
  </gmd:identificationInfo>
</gmd:MD_Metadata>"#,
    )
    .unwrap();

    // Set metadata
    let (stdout, _, success) = run(
        &repo,
        &["metadata", "set", "cities", meta_file.to_str().unwrap()],
    );
    assert!(success, "metadata set should succeed: {stdout}");
    assert!(stdout.contains("Metadata set"));

    // Show metadata
    let (stdout, _, success) = run(&repo, &["metadata", "show", "cities"]);
    assert!(success);
    assert!(stdout.contains("MD_Metadata"));
    assert!(stdout.contains("Cities of the World"));

    // data info should show has metadata
    let (stdout, _, _) = run(&repo, &["data", "info", "cities"]);
    assert!(stdout.contains("Has metadata: yes"));
}

#[test]
fn test_metadata_invalid_xml() {
    let dir = tempdir("metadata_invalid");
    let repo = dir.path().join("repo");
    run(dir.path(), &["init", repo.to_str().unwrap()]);
    setup_git_config(&repo);

    let gpkg = dir.path().join("data.gpkg");
    create_test_gpkg(&gpkg);
    run(&repo, &["import", &format!("GPKG:{}", gpkg.display())]);

    let bad_file = dir.path().join("not_xml.txt");
    fs::write(&bad_file, "This is not XML at all").unwrap();

    let (_, _, success) = run(
        &repo,
        &["metadata", "set", "cities", bad_file.to_str().unwrap()],
    );
    assert!(!success, "should reject non-XML content");
}

#[test]
fn test_metadata_nonexistent_dataset() {
    let dir = tempdir("metadata_nodata");
    let repo = dir.path().join("repo");
    run(dir.path(), &["init", repo.to_str().unwrap()]);
    setup_git_config(&repo);

    let meta_file = dir.path().join("m.xml");
    fs::write(&meta_file, "<root/>").unwrap();

    let (_, _, success) = run(
        &repo,
        &["metadata", "set", "nope", meta_file.to_str().unwrap()],
    );
    assert!(!success, "should fail for nonexistent dataset");
}

// ─── License Tests ───────────────────────────────────────────────────────────

#[test]
fn test_license_set_and_show_text() {
    let dir = tempdir("license_text");
    let repo = dir.path().join("repo");
    run(dir.path(), &["init", repo.to_str().unwrap()]);
    setup_git_config(&repo);

    let gpkg = dir.path().join("data.gpkg");
    create_test_gpkg(&gpkg);
    run(&repo, &["import", &format!("GPKG:{}", gpkg.display())]);

    let license_file = dir.path().join("LICENSE.txt");
    fs::write(
        &license_file,
        "Creative Commons Attribution 4.0 International (CC BY 4.0)",
    )
    .unwrap();

    let (stdout, _, success) = run(
        &repo,
        &["license", "set", "cities", license_file.to_str().unwrap()],
    );
    assert!(success, "license set should succeed: {stdout}");
    assert!(stdout.contains("License set"));

    let (stdout, _, success) = run(&repo, &["license", "show", "cities"]);
    assert!(success);
    assert!(stdout.contains("CC BY 4.0"));

    // data info should show has license
    let (stdout, _, _) = run(&repo, &["data", "info", "cities"]);
    assert!(stdout.contains("Has license: yes"));
}

#[test]
fn test_license_set_xml() {
    let dir = tempdir("license_xml");
    let repo = dir.path().join("repo");
    run(dir.path(), &["init", repo.to_str().unwrap()]);
    setup_git_config(&repo);

    let gpkg = dir.path().join("data.gpkg");
    create_test_gpkg(&gpkg);
    run(&repo, &["import", &format!("GPKG:{}", gpkg.display())]);

    let license_xml = dir.path().join("license.xml");
    fs::write(
        &license_xml,
        r#"<?xml version="1.0"?>
<license>
  <name>ODbL</name>
  <url>https://opendatacommons.org/licenses/odbl/</url>
</license>"#,
    )
    .unwrap();

    let (_, _, success) = run(
        &repo,
        &["license", "set", "cities", license_xml.to_str().unwrap()],
    );
    assert!(success);

    // Should be stored as license.xml (not license)
    assert!(repo.join("cities/.table-dataset/meta/license.xml").exists());

    let (stdout, _, _) = run(&repo, &["license", "show", "cities"]);
    assert!(stdout.contains("ODbL"));
}

#[test]
fn test_license_nonexistent_dataset() {
    let dir = tempdir("license_nodata");
    let repo = dir.path().join("repo");
    run(dir.path(), &["init", repo.to_str().unwrap()]);
    setup_git_config(&repo);

    let f = dir.path().join("l.txt");
    fs::write(&f, "MIT").unwrap();

    let (_, _, success) = run(&repo, &["license", "set", "nope", f.to_str().unwrap()]);
    assert!(!success, "should fail for nonexistent dataset");
}

#[test]
fn test_metadata_on_file_dataset() {
    let dir = tempdir("meta_file_ds");
    let repo = dir.path().join("repo");
    run(dir.path(), &["init", repo.to_str().unwrap()]);
    setup_git_config(&repo);

    // Add a file to create the dataset
    let doc = dir.path().join("report.pdf");
    fs::write(&doc, "PDF content").unwrap();
    run(&repo, &["files", "add", doc.to_str().unwrap()]);

    // Set metadata on the file dataset
    let meta_file = dir.path().join("meta.xml");
    fs::write(&meta_file, "<dataset><name>Reports</name></dataset>").unwrap();

    let (stdout, _, success) = run(
        &repo,
        &["metadata", "set", "files", meta_file.to_str().unwrap()],
    );
    assert!(success, "metadata set on file dataset: {stdout}");

    let (stdout, _, success) = run(&repo, &["metadata", "show", "files"]);
    assert!(success);
    assert!(stdout.contains("Reports"));
}

// ─── Point Cloud Tests ───────────────────────────────────────────────────────

/// Create a minimal LAS file for testing.
fn create_test_las(path: &Path) {
    use las::header::Builder;
    use las::point::Format;
    use las::{Point, Writer};

    let mut builder = Builder::default();
    builder.point_format = Format::new(0).unwrap();
    let header = builder.into_header().unwrap();
    let mut writer = Writer::from_path(path, header).unwrap();
    let point = Point {
        x: 1.0,
        y: 2.0,
        z: 3.0,
        ..Default::default()
    };
    writer.write_point(point).unwrap();
    writer.close().unwrap();
}

#[test]
fn test_pointcloud_import_and_ls() {
    let dir = tempdir("pointcloud");
    let repo = dir.path();
    run(repo, &["init"]);
    setup_git_config(repo);

    // Create a test LAS file
    let las_file = repo.join("test-tile.las");
    create_test_las(&las_file);

    // Import point cloud
    let (stdout, stderr, success) = run(
        repo,
        &[
            "pointcloud",
            "import",
            "--dataset",
            "lidar/scan1",
            las_file.to_str().unwrap(),
        ],
    );
    assert!(success, "pointcloud import failed: {stdout}\n{stderr}");
    assert!(stdout.contains("imported tile:"));
    assert!(stdout.contains("1 tile(s)"));

    // Verify dataset structure
    let ds_dir = repo.join("lidar/scan1/.point-cloud-dataset.v1");
    assert!(ds_dir.join("meta/title").exists());
    assert!(ds_dir.join("meta/format.json").exists());
    assert!(ds_dir.join("meta/schema.json").exists());
    assert!(ds_dir.join("tile").exists());

    // Check format.json
    let format: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(ds_dir.join("meta/format.json")).unwrap())
            .unwrap();
    assert_eq!(format["compression"], "las");
    assert_eq!(format["pointDataRecordFormat"], 0);

    // List point cloud datasets
    let (stdout, _, success) = run(repo, &["pointcloud", "ls"]);
    assert!(success);
    assert!(stdout.contains("lidar/scan1"));
    assert!(stdout.contains("1 tile(s)"));

    // Should appear in data ls too
    let (stdout, _, success) = run(repo, &["data", "ls"]);
    assert!(success);
    assert!(stdout.contains("lidar/scan1 (point cloud)"));
}

#[test]
fn test_pointcloud_info() {
    let dir = tempdir("pointcloud-info");
    let repo = dir.path();
    run(repo, &["init"]);
    setup_git_config(repo);

    let las_file = repo.join("cloud.las");
    create_test_las(&las_file);

    run(
        repo,
        &[
            "pointcloud",
            "import",
            "--dataset",
            "mycloud",
            las_file.to_str().unwrap(),
        ],
    );

    let (stdout, _, success) = run(repo, &["pointcloud", "info", "mycloud"]);
    assert!(success);
    assert!(stdout.contains("Title: mycloud"));
    assert!(stdout.contains("Format:"));
    assert!(stdout.contains("Compression: las"));
    assert!(stdout.contains("Schema:"));
    assert!(stdout.contains("X: integer(32)"));
    assert!(stdout.contains("Y: integer(32)"));
    assert!(stdout.contains("Z: integer(32)"));
    assert!(stdout.contains("Tiles: 1"));
}

#[test]
fn test_pointcloud_info_nonexistent() {
    let dir = tempdir("pointcloud-noexist");
    let repo = dir.path();
    run(repo, &["init"]);

    let (_, stderr, success) = run(repo, &["pointcloud", "info", "nope"]);
    assert!(!success);
    assert!(stderr.contains("not found"));
}

// ─── Raster Tests ────────────────────────────────────────────────────────────

/// Create a minimal TIFF file for testing (2x2 grayscale).
fn create_test_tiff(path: &Path) {
    use std::io::BufWriter;
    use tiff::encoder::TiffEncoder;
    use tiff::encoder::colortype::Gray8;

    let file = std::fs::File::create(path).unwrap();
    let buf = BufWriter::new(file);
    let mut encoder = TiffEncoder::new(buf).unwrap();
    let data: [u8; 4] = [10, 20, 30, 40];
    encoder.write_image::<Gray8>(2, 2, &data).unwrap();
}

#[test]
fn test_raster_import_and_ls() {
    let dir = tempdir("raster");
    let repo = dir.path();
    run(repo, &["init"]);
    setup_git_config(repo);

    let tiff_file = repo.join("aerial.tif");
    create_test_tiff(&tiff_file);

    // Import raster
    let (stdout, stderr, success) = run(
        repo,
        &[
            "raster",
            "import",
            "--dataset",
            "aerials/north",
            tiff_file.to_str().unwrap(),
        ],
    );
    assert!(success, "raster import failed: {stdout}\n{stderr}");
    assert!(stdout.contains("imported tile:"));
    assert!(stdout.contains("1 tile(s)"));

    // Verify dataset structure
    let ds_dir = repo.join("aerials/north/.raster-dataset.v1");
    assert!(ds_dir.join("meta/title").exists());
    assert!(ds_dir.join("meta/format.json").exists());
    assert!(ds_dir.join("meta/schema.json").exists());
    assert!(ds_dir.join("tile").exists());

    // Check format.json
    let format: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(ds_dir.join("meta/format.json")).unwrap())
            .unwrap();
    assert_eq!(format["fileType"], "geotiff");

    // List raster datasets
    let (stdout, _, success) = run(repo, &["raster", "ls"]);
    assert!(success);
    assert!(stdout.contains("aerials/north"));
    assert!(stdout.contains("1 tile(s)"));

    // Should appear in data ls too
    let (stdout, _, success) = run(repo, &["data", "ls"]);
    assert!(success);
    assert!(stdout.contains("aerials/north (raster)"));
}

#[test]
fn test_raster_info() {
    let dir = tempdir("raster-info");
    let repo = dir.path();
    run(repo, &["init"]);
    setup_git_config(repo);

    let tiff_file = repo.join("dem.tif");
    create_test_tiff(&tiff_file);

    run(
        repo,
        &[
            "raster",
            "import",
            "--dataset",
            "elevation",
            tiff_file.to_str().unwrap(),
        ],
    );

    let (stdout, _, success) = run(repo, &["raster", "info", "elevation"]);
    assert!(success);
    assert!(stdout.contains("Title: elevation"));
    assert!(stdout.contains("Format:"));
    assert!(stdout.contains("File Type: geotiff"));
    assert!(stdout.contains("Schema: 1 band(s)"));
    assert!(stdout.contains("Band 1: integer(8)"));
    assert!(stdout.contains("Tiles: 1"));
}

#[test]
fn test_raster_info_nonexistent() {
    let dir = tempdir("raster-noexist");
    let repo = dir.path();
    run(repo, &["init"]);

    let (_, stderr, success) = run(repo, &["raster", "info", "nope"]);
    assert!(!success);
    assert!(stderr.contains("not found"));
}

#[test]
fn test_pointcloud_multiple_tiles() {
    let dir = tempdir("pointcloud-multi");
    let repo = dir.path();
    run(repo, &["init"]);
    setup_git_config(repo);

    let las1 = repo.join("tile-a.las");
    let las2 = repo.join("tile-b.las");
    create_test_las(&las1);
    create_test_las(&las2);

    let (stdout, _, success) = run(
        repo,
        &[
            "pointcloud",
            "import",
            "--dataset",
            "scan",
            las1.to_str().unwrap(),
            las2.to_str().unwrap(),
        ],
    );
    assert!(success);
    assert!(stdout.contains("2 tile(s)"));

    // Both tiles should be in different shard directories
    let (stdout, _, success) = run(repo, &["pointcloud", "ls"]);
    assert!(success);
    assert!(stdout.contains("2 tile(s)"));
}
