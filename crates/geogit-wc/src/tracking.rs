use std::collections::HashMap;

use geogit_encoding::value::ColumnValue;
use rusqlite::Connection;

use crate::gpkg::read_sqlite_value;

/// Change tracking table management for GeoPackage working copies.
///
/// GeoGit installs triggers on each dataset table that record
/// INSERT/UPDATE/DELETE operations in a tracking table. This allows
/// efficient detection of changes without scanning all rows.
///
/// Updates and deletes also copy the pre-edit row into a second table,
/// since the working copy itself no longer holds those values.
pub struct ChangeTracker {
    tracking_table: String,
    old_values_table: String,
}

impl Default for ChangeTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl ChangeTracker {
    pub fn new() -> Self {
        Self {
            tracking_table: "_geogit_track".to_string(),
            old_values_table: "_geogit_track_old".to_string(),
        }
    }

    /// Create the change tracking tables if they don't exist.
    pub fn init(&self, conn: &Connection) -> rusqlite::Result<()> {
        conn.execute_batch(&format!(
            "CREATE TABLE IF NOT EXISTS {track} (
                table_name TEXT NOT NULL,
                pk TEXT NOT NULL,
                change_type TEXT NOT NULL CHECK(change_type IN ('I', 'U', 'D')),
                changed_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
            );
            CREATE TABLE IF NOT EXISTS {old} (
                table_name TEXT NOT NULL,
                pk TEXT NOT NULL,
                column_name TEXT NOT NULL,
                -- untyped so each value keeps its sqlite storage class
                \"value\",
                PRIMARY KEY (table_name, pk, column_name)
            )",
            track = self.tracking_table,
            old = self.old_values_table,
        ))?;
        Ok(())
    }

    /// Install INSERT/UPDATE/DELETE triggers on a table.
    pub fn install_triggers(
        &self,
        conn: &Connection,
        table_name: &str,
        pk_column: &str,
    ) -> rusqlite::Result<()> {
        // INSERT trigger
        conn.execute_batch(&format!(
            "CREATE TRIGGER IF NOT EXISTS _geogit_ins_{table_name}
             AFTER INSERT ON \"{table_name}\"
             BEGIN
                 INSERT INTO {track} (table_name, pk, change_type)
                 VALUES ('{table_name}', CAST(NEW.\"{pk_column}\" AS TEXT), 'I');
             END",
            track = self.tracking_table,
        ))?;

        // UPDATE trigger: key the old row by the new pk, matching the tracking row
        let save_old_on_update = self.save_old_row(conn, table_name, pk_column, "NEW")?;
        conn.execute_batch(&format!(
            "CREATE TRIGGER IF NOT EXISTS _geogit_upd_{table_name}
             AFTER UPDATE ON \"{table_name}\"
             BEGIN
                 {save_old_on_update}
                 INSERT INTO {track} (table_name, pk, change_type)
                 VALUES ('{table_name}', CAST(NEW.\"{pk_column}\" AS TEXT), 'U');
             END",
            track = self.tracking_table,
        ))?;

        // DELETE trigger
        let save_old_on_delete = self.save_old_row(conn, table_name, pk_column, "OLD")?;
        conn.execute_batch(&format!(
            "CREATE TRIGGER IF NOT EXISTS _geogit_del_{table_name}
             AFTER DELETE ON \"{table_name}\"
             BEGIN
                 {save_old_on_delete}
                 INSERT INTO {track} (table_name, pk, change_type)
                 VALUES ('{table_name}', CAST(OLD.\"{pk_column}\" AS TEXT), 'D');
             END",
            track = self.tracking_table,
        ))?;

        Ok(())
    }

    /// Trigger body that copies every OLD column into the old-values table.
    ///
    /// `pk_source` is NEW or OLD: the row the tracking entry is keyed by.
    /// OR IGNORE keeps the first value seen, so repeated edits still report
    /// the state the working copy was checked out with.
    fn save_old_row(
        &self,
        conn: &Connection,
        table_name: &str,
        pk_column: &str,
        pk_source: &str,
    ) -> rusqlite::Result<String> {
        let mut stmt = conn.prepare(&format!("PRAGMA table_info(\"{table_name}\")"))?;
        let columns: Vec<String> = stmt
            .query_map([], |row| row.get::<_, String>(1))?
            .collect::<rusqlite::Result<_>>()?;

        Ok(columns
            .iter()
            .map(|col| {
                format!(
                    "INSERT OR IGNORE INTO {old} (table_name, pk, column_name, \"value\")
                     VALUES ('{table_name}', CAST({pk_source}.\"{pk_column}\" AS TEXT), '{col}', OLD.\"{col}\");",
                    old = self.old_values_table,
                )
            })
            .collect::<Vec<_>>()
            .join("\n"))
    }

    /// Get all tracked changes for a table.
    pub fn get_changes(
        &self,
        conn: &Connection,
        table_name: &str,
    ) -> rusqlite::Result<Vec<TrackedChange>> {
        let mut stmt = conn.prepare(&format!(
            "SELECT pk, change_type FROM {} WHERE table_name = ?1 ORDER BY rowid",
            self.tracking_table
        ))?;
        let rows = stmt.query_map([table_name], |row| {
            Ok(TrackedChange {
                pk: row.get(0)?,
                change_type: row.get(1)?,
            })
        })?;
        rows.collect()
    }

    /// Get the pre-edit values of a tracked row, by column name.
    ///
    /// Empty for inserts, which have no earlier state.
    pub fn old_values(
        &self,
        conn: &Connection,
        table_name: &str,
        pk: &str,
    ) -> rusqlite::Result<HashMap<String, ColumnValue>> {
        let mut stmt = conn.prepare(&format!(
            "SELECT column_name, \"value\" FROM {} WHERE table_name = ?1 AND pk = ?2",
            self.old_values_table
        ))?;
        let rows = stmt.query_map([table_name, pk], |row| {
            Ok((row.get::<_, String>(0)?, read_sqlite_value(row, 1)?))
        })?;
        rows.collect()
    }

    /// Clear tracked changes for a table (after commit).
    pub fn clear(&self, conn: &Connection, table_name: &str) -> rusqlite::Result<()> {
        conn.execute(
            &format!("DELETE FROM {} WHERE table_name = ?1", self.tracking_table),
            [table_name],
        )?;
        conn.execute(
            &format!(
                "DELETE FROM {} WHERE table_name = ?1",
                self.old_values_table
            ),
            [table_name],
        )?;
        Ok(())
    }
}

/// A tracked change from the tracking table.
#[derive(Debug, Clone)]
pub struct TrackedChange {
    /// Primary key value as text.
    pub pk: String,
    /// Change type: 'I' (insert), 'U' (update), 'D' (delete).
    pub change_type: String,
}

impl TrackedChange {
    pub fn is_insert(&self) -> bool {
        self.change_type == "I"
    }

    pub fn is_update(&self) -> bool {
        self.change_type == "U"
    }

    pub fn is_delete(&self) -> bool {
        self.change_type == "D"
    }
}
