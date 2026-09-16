use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use rusqlite::Connection;

use geogit_core::dataset::DatasetMeta;
use geogit_core::diff::FeatureDelta;
use geogit_encoding::crs;
use geogit_encoding::geometry::gpkg_geometry_with_srs_id;
use geogit_encoding::schema::{Column, DataType};
use geogit_encoding::value::ColumnValue;

use crate::tracking::ChangeTracker;
use crate::traits::WorkingCopy;

const DEFAULT_SRS_ID: i32 = 4326;

/// GeoPackage working copy.
///
/// Stores datasets as tables in a GeoPackage (.gpkg) SQLite database.
/// Change tracking triggers record edits for efficient commit detection.
pub struct GeoPackageWorkingCopy {
    conn: Connection,
    path: PathBuf,
    tracker: ChangeTracker,
}

impl GeoPackageWorkingCopy {
    /// Open or create a GeoPackage working copy.
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path).context("failed to open GeoPackage")?;

        // Initialize GeoPackage metadata tables
        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS gpkg_spatial_ref_sys (
                srs_name TEXT NOT NULL,
                srs_id INTEGER NOT NULL PRIMARY KEY,
                organization TEXT NOT NULL,
                organization_coordsys_id INTEGER NOT NULL,
                definition TEXT NOT NULL,
                description TEXT
            );
            CREATE TABLE IF NOT EXISTS gpkg_contents (
                table_name TEXT NOT NULL PRIMARY KEY,
                data_type TEXT NOT NULL DEFAULT 'features',
                identifier TEXT UNIQUE,
                description TEXT DEFAULT '',
                last_change DATETIME DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
                min_x DOUBLE,
                min_y DOUBLE,
                max_x DOUBLE,
                max_y DOUBLE,
                srs_id INTEGER REFERENCES gpkg_spatial_ref_sys(srs_id)
            );
            CREATE TABLE IF NOT EXISTS gpkg_geometry_columns (
                table_name TEXT NOT NULL,
                column_name TEXT NOT NULL,
                geometry_type_name TEXT NOT NULL,
                srs_id INTEGER NOT NULL,
                z TINYINT NOT NULL,
                m TINYINT NOT NULL,
                CONSTRAINT pk_geom PRIMARY KEY (table_name, column_name),
                CONSTRAINT fk_gc_tn FOREIGN KEY (table_name) REFERENCES gpkg_contents(table_name)
            );
            ",
        )
        .context("failed to initialize GeoPackage tables")?;

        // Add default SRS (WGS84)
        conn.execute(
            "INSERT OR IGNORE INTO gpkg_spatial_ref_sys
             (srs_name, srs_id, organization, organization_coordsys_id, definition)
             VALUES ('WGS 84', 4326, 'EPSG', 4326,
             'GEOGCS[\"WGS 84\",DATUM[\"WGS_1984\",SPHEROID[\"WGS 84\",6378137,298.257223563]],PRIMEM[\"Greenwich\",0],UNIT[\"degree\",0.0174532925199433]]')",
            [],
        ).ok();

        let tracker = ChangeTracker::new();
        tracker
            .init(&conn)
            .context("failed to init change tracker")?;

        Ok(Self {
            conn,
            path: path.to_path_buf(),
            tracker,
        })
    }

    /// Get the file path of this GeoPackage.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Clear tracking data for a dataset (after syncing changes to tree).
    pub fn clear_tracking(&self, dataset_path: &str) -> Result<()> {
        let table_name = dataset_path.replace('/', "_");
        self.tracker
            .clear(&self.conn, &table_name)
            .context("failed to clear tracking")?;
        Ok(())
    }

    /// Map a GeoGit data type to a SQLite/GeoPackage column type.
    fn sql_type(col: &Column) -> &'static str {
        match col.data_type {
            DataType::Boolean => "BOOLEAN",
            DataType::Blob => "BLOB",
            DataType::Date => "DATE",
            DataType::Float => match col.size {
                Some(32) => "REAL",
                _ => "DOUBLE",
            },
            DataType::Geometry => "GEOMETRY",
            DataType::Integer => match col.size {
                Some(8) | Some(16) | Some(32) => "INTEGER",
                _ => "INTEGER",
            },
            DataType::Interval => "TEXT",
            DataType::Numeric => "TEXT",
            DataType::Text => "TEXT",
            DataType::Time => "TEXT",
            DataType::Timestamp => "DATETIME",
        }
    }

    /// Name of the table's primary key column.
    fn pk_column(&self, table_name: &str) -> Result<String> {
        let mut stmt = self
            .conn
            .prepare(&format!("PRAGMA table_info(\"{table_name}\")"))?;
        let mut rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(1)?, row.get::<_, i64>(5)?))
        })?;
        match rows.find_map(|r| match r {
            Ok((name, pk)) if pk != 0 => Some(Ok(name)),
            Ok(_) => None,
            Err(e) => Some(Err(e)),
        }) {
            Some(name) => Ok(name?),
            None => bail!("table {table_name} has no primary key column"),
        }
    }

    fn srs_id_is_registered(&self, srs_id: i32) -> Result<bool> {
        let count: i64 = self.conn.query_row(
            "SELECT count(*) FROM gpkg_spatial_ref_sys WHERE srs_id = ?1",
            [srs_id],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    }

    /// Read one row by its primary key, as it appears in the working copy.
    fn read_row(
        &self,
        table_name: &str,
        pk_column: &str,
        pk: &str,
    ) -> Result<Option<HashMap<String, ColumnValue>>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT * FROM \"{table_name}\" WHERE CAST(\"{pk_column}\" AS TEXT) = ?1"
        ))?;
        let names: Vec<String> = stmt.column_names().iter().map(|n| n.to_string()).collect();
        let mut rows = stmt.query([pk])?;
        let Some(row) = rows.next()? else {
            return Ok(None);
        };
        let mut values = HashMap::with_capacity(names.len());
        for (i, name) in names.iter().enumerate() {
            values.insert(name.clone(), read_sqlite_value(row, i)?);
        }
        Ok(Some(values))
    }
}

fn column_srs_id(column: &Column, crs_definitions: &BTreeMap<String, String>) -> i32 {
    let identifier = column.geometry_crs.as_deref();
    let epsg_code = identifier
        .and_then(|crs| crs.strip_prefix("EPSG:"))
        .and_then(|code| code.parse::<i32>().ok())
        .filter(|code| *code > 0);
    epsg_code
        .or_else(|| {
            identifier
                .and_then(|identifier| crs_definitions.get(identifier))
                .and_then(|definition| crs::crs_srs_id(definition))
        })
        .unwrap_or(DEFAULT_SRS_ID)
}

/// Convert a SQLite cell to a `ColumnValue`.
pub fn read_sqlite_value(row: &rusqlite::Row, idx: usize) -> rusqlite::Result<ColumnValue> {
    use rusqlite::types::ValueRef;
    Ok(match row.get_ref(idx)? {
        ValueRef::Null => ColumnValue::Null,
        ValueRef::Integer(v) => ColumnValue::Integer(v),
        ValueRef::Real(v) => ColumnValue::Float(v),
        ValueRef::Text(v) => ColumnValue::Text(String::from_utf8_lossy(v).to_string()),
        ValueRef::Blob(v) => ColumnValue::Blob(v.to_vec()),
    })
}

impl WorkingCopy for GeoPackageWorkingCopy {
    fn checkout(
        &mut self,
        dataset_path: &str,
        meta: &DatasetMeta,
        features: &[(Vec<ColumnValue>, HashMap<String, ColumnValue>)],
    ) -> Result<()> {
        let table_name = dataset_path.replace('/', "_");

        // Build CREATE TABLE
        let mut col_defs = Vec::new();
        let mut pk_cols = Vec::new();
        let mut geom_col = None;

        for col in &meta.schema.0 {
            if col.data_type == DataType::Geometry {
                geom_col = Some(col.clone());
                col_defs.push(format!("\"{}\" BLOB", col.name));
            } else {
                col_defs.push(format!("\"{}\" {}", col.name, Self::sql_type(col)));
            }
            if col.primary_key_index.is_some() {
                pk_cols.push(format!("\"{}\"", col.name));
            }
        }

        if !pk_cols.is_empty() {
            col_defs.push(format!("PRIMARY KEY ({})", pk_cols.join(", ")));
        }

        let create_sql = format!(
            "CREATE TABLE IF NOT EXISTS \"{}\" ({})",
            table_name,
            col_defs.join(", ")
        );
        self.conn
            .execute(&create_sql, [])
            .context("create dataset table")?;

        let geometry_srs_id = geom_col
            .as_ref()
            .map(|geom| column_srs_id(geom, &meta.crs_definitions));

        // Register the CRS, which gpkg_contents references
        if let Some((srs_id, definition)) = geometry_srs_id.zip(
            geom_col
                .as_ref()
                .and_then(|geom| geom.geometry_crs.as_deref())
                .and_then(|identifier| meta.crs_definitions.get(identifier)),
        ) {
            self.conn.execute(
                "INSERT OR REPLACE INTO gpkg_spatial_ref_sys
                 (srs_name, srs_id, organization, organization_coordsys_id, definition)
                 VALUES (?1, ?2, ?3, ?2, ?4)",
                rusqlite::params![
                    crs::crs_name(definition).unwrap_or(""),
                    srs_id,
                    crs::crs_organization(definition),
                    definition
                ],
            )?;
        }

        // gpkg_contents.srs_id is a foreign key, so only name an srs the file holds
        let contents_srs_id = match geometry_srs_id {
            Some(srs_id) if self.srs_id_is_registered(srs_id)? => Some(srs_id),
            _ => None,
        };
        self.conn.execute(
            "INSERT OR REPLACE INTO gpkg_contents (table_name, data_type, identifier, srs_id)
             VALUES (?1, 'features', ?2, ?3)",
            rusqlite::params![table_name, meta.title, contents_srs_id],
        )?;

        // Register geometry column
        if let Some(ref geom) = geom_col {
            let geom_type = geom.geometry_type.as_deref().unwrap_or("GEOMETRY");
            let srs_id = geometry_srs_id.unwrap_or(DEFAULT_SRS_ID);

            self.conn.execute(
                "INSERT OR REPLACE INTO gpkg_geometry_columns
                 (table_name, column_name, geometry_type_name, srs_id, z, m)
                 VALUES (?1, ?2, ?3, ?4, 0, 0)",
                rusqlite::params![table_name, geom.name, geom_type, srs_id],
            )?;
        }

        // Insert features
        if !features.is_empty() {
            let col_names: Vec<String> = meta
                .schema
                .0
                .iter()
                .map(|c| format!("\"{}\"", c.name))
                .collect();
            let placeholders: Vec<String> =
                (1..=col_names.len()).map(|i| format!("?{i}")).collect();
            let insert_sql = format!(
                "INSERT OR REPLACE INTO \"{}\" ({}) VALUES ({})",
                table_name,
                col_names.join(", "),
                placeholders.join(", ")
            );

            let tx = self.conn.transaction()?;
            {
                let mut stmt = tx.prepare(&insert_sql)?;
                for (_pk, values) in features {
                    let params: Vec<Box<dyn rusqlite::types::ToSql>> = meta
                        .schema
                        .0
                        .iter()
                        .map(|col| -> Result<Box<dyn rusqlite::types::ToSql>> {
                            Ok(match values.get(&col.name) {
                                Some(ColumnValue::Null) | None => Box::new(rusqlite::types::Null),
                                Some(ColumnValue::Bool(v)) => Box::new(*v),
                                Some(ColumnValue::Integer(v)) => Box::new(*v),
                                Some(ColumnValue::Float(v)) => Box::new(*v),
                                Some(ColumnValue::Text(v)) => Box::new(v.clone()),
                                Some(ColumnValue::Blob(v)) | Some(ColumnValue::Geometry(v)) => {
                                    match geometry_srs_id {
                                        Some(srs_id) if col.data_type == DataType::Geometry => {
                                            Box::new(gpkg_geometry_with_srs_id(v, srs_id).context(
                                                "stamp the working copy srs id into a geometry",
                                            )?)
                                        }
                                        _ => Box::new(v.clone()),
                                    }
                                }
                            })
                        })
                        .collect::<Result<Vec<_>>>()?;

                    let param_refs: Vec<&dyn rusqlite::types::ToSql> =
                        params.iter().map(|p| p.as_ref()).collect();
                    stmt.execute(param_refs.as_slice())?;
                }
            }
            tx.commit()?;
        }

        // Install change tracking triggers
        if let Some(pk_col) = meta.schema.primary_key_columns().first() {
            self.tracker
                .install_triggers(&self.conn, &table_name, &pk_col.name)?;
        }

        // Clear any existing tracking data (fresh checkout)
        self.tracker.clear(&self.conn, &table_name)?;

        Ok(())
    }

    fn status(&self, dataset_path: &str) -> Result<Vec<FeatureDelta>> {
        let table_name = dataset_path.replace('/', "_");
        let changes = self.tracker.get_changes(&self.conn, &table_name)?;
        if changes.is_empty() {
            return Ok(Vec::new());
        }
        let pk_column = self.pk_column(&table_name)?;

        let mut deltas = Vec::new();
        for change in changes {
            let new = self.read_row(&table_name, &pk_column, &change.pk)?;
            let old = self
                .tracker
                .old_values(&self.conn, &table_name, &change.pk)?;
            let pk = vec![
                new.as_ref()
                    .and_then(|row| row.get(&pk_column))
                    .or_else(|| old.get(&pk_column))
                    .cloned()
                    .unwrap_or(ColumnValue::Text(change.pk.clone())),
            ];
            match change.change_type.as_str() {
                "I" => {
                    deltas.push(FeatureDelta::Insert {
                        pk,
                        new: new.unwrap_or_default(),
                    });
                }
                "U" => {
                    let new = new.unwrap_or_default();
                    let mut changed_columns: Vec<String> = new
                        .iter()
                        .filter(|(name, value)| old.get(*name) != Some(*value))
                        .map(|(name, _)| name.clone())
                        .collect();
                    changed_columns.sort();
                    deltas.push(FeatureDelta::Update {
                        pk,
                        old,
                        new,
                        changed_columns,
                    });
                }
                "D" => {
                    deltas.push(FeatureDelta::Delete { pk, old });
                }
                _ => {}
            }
        }
        Ok(deltas)
    }

    fn reset(
        &mut self,
        dataset_path: &str,
        meta: &DatasetMeta,
        features: &[(Vec<ColumnValue>, HashMap<String, ColumnValue>)],
    ) -> Result<()> {
        let table_name = dataset_path.replace('/', "_");
        // Drop and recreate
        self.conn
            .execute(&format!("DROP TABLE IF EXISTS \"{table_name}\""), [])?;
        self.checkout(dataset_path, meta, features)
    }

    fn list_datasets(&self) -> Result<Vec<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT table_name FROM gpkg_contents WHERE data_type = 'features'")?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use geogit_encoding::path::PathStructure;
    use geogit_encoding::schema::{Column, DataType, Schema};
    use uuid::Uuid;

    fn test_schema() -> Schema {
        Schema(vec![
            Column {
                id: Uuid::new_v4(),
                name: "fid".into(),
                data_type: DataType::Integer,
                primary_key_index: Some(0),
                size: Some(64),
                geometry_type: None,
                geometry_crs: None,
                length: None,
                precision: None,
                scale: None,
                timezone: None,
            },
            Column {
                id: Uuid::new_v4(),
                name: "name".into(),
                data_type: DataType::Text,
                primary_key_index: None,
                size: None,
                geometry_type: None,
                geometry_crs: None,
                length: Some(250),
                precision: None,
                scale: None,
                timezone: None,
            },
            Column {
                id: Uuid::new_v4(),
                name: "population".into(),
                data_type: DataType::Integer,
                primary_key_index: None,
                size: Some(32),
                geometry_type: None,
                geometry_crs: None,
                length: None,
                precision: None,
                scale: None,
                timezone: None,
            },
        ])
    }

    fn test_features() -> Vec<(Vec<ColumnValue>, HashMap<String, ColumnValue>)> {
        vec![
            (
                vec![ColumnValue::Integer(1)],
                HashMap::from([
                    ("fid".into(), ColumnValue::Integer(1)),
                    ("name".into(), ColumnValue::Text("Tokyo".into())),
                    ("population".into(), ColumnValue::Integer(13_960_000)),
                ]),
            ),
            (
                vec![ColumnValue::Integer(2)],
                HashMap::from([
                    ("fid".into(), ColumnValue::Integer(2)),
                    ("name".into(), ColumnValue::Text("Delhi".into())),
                    ("population".into(), ColumnValue::Integer(11_034_555)),
                ]),
            ),
        ]
    }

    fn test_meta() -> DatasetMeta {
        DatasetMeta {
            title: "Cities".into(),
            description: "World cities".into(),
            schema: test_schema(),
            path_structure: PathStructure::default(),
            crs_definitions: Default::default(),
        }
    }

    fn geometry_meta(geometry_crs: &str, crs_definitions: BTreeMap<String, String>) -> DatasetMeta {
        let mut schema = test_schema();
        schema.0.push(Column {
            id: Uuid::new_v4(),
            name: "geom".into(),
            data_type: DataType::Geometry,
            primary_key_index: None,
            size: None,
            geometry_type: Some("POINT".into()),
            geometry_crs: Some(geometry_crs.to_string()),
            length: None,
            precision: None,
            scale: None,
            timezone: None,
        });
        DatasetMeta {
            title: "Cities".into(),
            description: "World cities".into(),
            schema,
            path_structure: PathStructure::default(),
            crs_definitions,
        }
    }

    #[test]
    fn test_checkout_without_a_crs_definition_leaves_contents_srs_unset() {
        let dir = std::env::temp_dir().join(format!("geogit-gpkg-no-crs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let mut wc = GeoPackageWorkingCopy::open(&dir.join("test.gpkg")).unwrap();
        let meta = geometry_meta("EPSG:2193", BTreeMap::new());
        wc.checkout("cities", &meta, &test_features()).unwrap();

        let contents_srs_id: Option<i32> = wc
            .conn
            .query_row(
                "SELECT srs_id FROM gpkg_contents WHERE table_name = 'cities'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(contents_srs_id, None);

        let column_srs_id: i32 = wc
            .conn
            .query_row(
                "SELECT srs_id FROM gpkg_geometry_columns WHERE table_name = 'cities'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(column_srs_id, 2193);
    }

    #[test]
    fn test_checkout_keeps_the_epsg_code_from_the_identifier() {
        let dir = std::env::temp_dir().join(format!("geogit-gpkg-epsg-crs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        // the definition has no authority, but the dataset names the CRS by its EPSG code
        let meta = geometry_meta(
            "EPSG:2193",
            BTreeMap::from([(
                "EPSG:2193".to_string(),
                "PROJCS[\"NZGD2000 / New Zealand Transverse Mercator 2000\"]".to_string(),
            )]),
        );

        let mut wc = GeoPackageWorkingCopy::open(&dir.join("test.gpkg")).unwrap();
        wc.checkout("cities", &meta, &test_features()).unwrap();

        let column_srs_id: i32 = wc
            .conn
            .query_row(
                "SELECT srs_id FROM gpkg_geometry_columns WHERE table_name = 'cities'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(column_srs_id, 2193);
    }

    #[test]
    fn test_checkout_registers_a_custom_crs() {
        let dir =
            std::env::temp_dir().join(format!("geogit-gpkg-custom-crs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let definition = "PROJCS[\"Lambert / Custom\",GEOGCS[\"Custom\"]]";
        let meta = geometry_meta(
            "Lambert _ Custom",
            BTreeMap::from([("Lambert _ Custom".to_string(), definition.to_string())]),
        );

        let mut wc = GeoPackageWorkingCopy::open(&dir.join("test.gpkg")).unwrap();
        wc.checkout("cities", &meta, &test_features()).unwrap();

        // kart's uint32hash of the crs name, inside its 200000 to 209199 custom range
        let expected_srs_id = 200_603;
        let (contents_srs_id, column_srs_id, srs_name, organization, stored_definition): (
            i32,
            i32,
            String,
            String,
            String,
        ) = wc
            .conn
            .query_row(
                "SELECT C.srs_id, G.srs_id, S.srs_name, S.organization, S.definition
                 FROM gpkg_contents C
                 JOIN gpkg_geometry_columns G ON G.table_name = C.table_name
                 JOIN gpkg_spatial_ref_sys S ON S.srs_id = C.srs_id
                 WHERE C.table_name = 'cities'",
                [],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                    ))
                },
            )
            .unwrap();
        assert_eq!(contents_srs_id, expected_srs_id);
        assert_eq!(column_srs_id, expected_srs_id);
        assert_eq!(srs_name, "Lambert / Custom");
        assert_eq!(organization, "NONE");
        assert_eq!(stored_definition, definition);
    }

    #[test]
    fn test_status_reports_feature_values() {
        let dir = std::env::temp_dir().join(format!("geogit-gpkg-status-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let mut wc = GeoPackageWorkingCopy::open(&dir.join("test.gpkg")).unwrap();
        wc.checkout("cities", &test_meta(), &test_features())
            .unwrap();

        wc.conn
            .execute_batch(
                "INSERT INTO cities (fid, name, population) VALUES (3, 'Shanghai', 24870895);
                 UPDATE cities SET population = 14000000 WHERE fid = 1;
                 DELETE FROM cities WHERE fid = 2;",
            )
            .unwrap();

        let changes = wc.status("cities").unwrap();
        assert_eq!(changes.len(), 3);

        match &changes[0] {
            FeatureDelta::Insert { pk, new } => {
                assert_eq!(pk, &[ColumnValue::Integer(3)]);
                assert_eq!(new["name"], ColumnValue::Text("Shanghai".into()));
                assert_eq!(new["population"], ColumnValue::Integer(24_870_895));
            }
            other => panic!("expected insert, got {other:?}"),
        }

        match &changes[1] {
            FeatureDelta::Update {
                pk,
                old,
                new,
                changed_columns,
            } => {
                assert_eq!(pk, &[ColumnValue::Integer(1)]);
                assert_eq!(old["population"], ColumnValue::Integer(13_960_000));
                assert_eq!(new["population"], ColumnValue::Integer(14_000_000));
                assert_eq!(old["name"], ColumnValue::Text("Tokyo".into()));
                assert_eq!(changed_columns, &["population"]);
            }
            other => panic!("expected update, got {other:?}"),
        }

        match &changes[2] {
            FeatureDelta::Delete { pk, old } => {
                assert_eq!(pk, &[ColumnValue::Integer(2)]);
                assert_eq!(old["name"], ColumnValue::Text("Delhi".into()));
                assert_eq!(old["population"], ColumnValue::Integer(11_034_555));
            }
            other => panic!("expected delete, got {other:?}"),
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_clear_tracking_drops_old_values() {
        let dir = std::env::temp_dir().join(format!("geogit-gpkg-clear-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let mut wc = GeoPackageWorkingCopy::open(&dir.join("test.gpkg")).unwrap();
        wc.checkout("cities", &test_meta(), &test_features())
            .unwrap();
        wc.conn
            .execute("DELETE FROM cities WHERE fid = 2", [])
            .unwrap();
        assert_eq!(wc.status("cities").unwrap().len(), 1);

        wc.clear_tracking("cities").unwrap();
        assert!(wc.status("cities").unwrap().is_empty());
        let leftover: i64 = wc
            .conn
            .query_row("SELECT COUNT(*) FROM _geogit_track_old", [], |r| r.get(0))
            .unwrap();
        assert_eq!(leftover, 0);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_gpkg_checkout_and_list() {
        let dir = std::env::temp_dir().join(format!("geogit-gpkg-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let gpkg_path = dir.join("test.gpkg");

        let mut wc = GeoPackageWorkingCopy::open(&gpkg_path).unwrap();
        wc.checkout("cities", &test_meta(), &test_features())
            .unwrap();

        // Verify datasets
        let datasets = wc.list_datasets().unwrap();
        assert_eq!(datasets, vec!["cities"]);

        // Verify rows were inserted
        let count: i64 = wc
            .conn
            .query_row("SELECT COUNT(*) FROM cities", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 2);

        // Verify status is clean (no changes since checkout)
        let changes = wc.status("cities").unwrap();
        assert!(changes.is_empty());

        // Make a change and verify tracking
        wc.conn
            .execute(
                "INSERT INTO cities (fid, name, population) VALUES (3, 'Shanghai', 24870895)",
                [],
            )
            .unwrap();
        let changes = wc.status("cities").unwrap();
        assert_eq!(changes.len(), 1);
        assert!(changes[0].is_insert());

        // Cleanup
        let _ = std::fs::remove_dir_all(&dir);
    }
}
