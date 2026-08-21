use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Column data types supported by GeoGit (Kart-compatible).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DataType {
    Boolean,
    Blob,
    Date,
    Float,
    Geometry,
    Integer,
    Interval,
    Numeric,
    Text,
    Time,
    Timestamp,
}

/// A single column definition in a dataset schema.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Column {
    /// Stable UUID that persists across renames.
    pub id: Uuid,
    /// Column name as used in SQL.
    pub name: String,
    /// Data type.
    pub data_type: DataType,
    /// Position in composite primary key, or None if not a PK column.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub primary_key_index: Option<u32>,

    // -- Extra type info (type-specific) --
    /// For geometry: e.g. "MULTIPOLYGON ZM"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub geometry_type: Option<String>,
    /// For geometry: e.g. "EPSG:4326"
    #[serde(rename = "geometryCRS", skip_serializing_if = "Option::is_none")]
    pub geometry_crs: Option<String>,
    /// For integer/float: bit size (8, 16, 32, 64)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<u32>,
    /// For text: max character length
    #[serde(skip_serializing_if = "Option::is_none")]
    pub length: Option<u64>,
    /// For numeric: total digits
    #[serde(skip_serializing_if = "Option::is_none")]
    pub precision: Option<u32>,
    /// For numeric: digits right of decimal
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scale: Option<u32>,
    /// For timestamp: "UTC" or null
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timezone: Option<String>,
}

/// A dataset schema: ordered list of columns.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Schema(pub Vec<Column>);

impl Schema {
    /// Get columns that are primary keys, ordered by primary_key_index.
    pub fn primary_key_columns(&self) -> Vec<&Column> {
        let mut pks: Vec<&Column> = self
            .0
            .iter()
            .filter(|c| c.primary_key_index.is_some())
            .collect();
        pks.sort_by_key(|c| c.primary_key_index.unwrap());
        pks
    }

    /// Get non-primary-key columns in order.
    pub fn value_columns(&self) -> Vec<&Column> {
        self.0
            .iter()
            .filter(|c| c.primary_key_index.is_none())
            .collect()
    }

    /// Get column IDs in order (used for legend generation).
    pub fn column_ids(&self) -> Vec<Uuid> {
        self.0.iter().map(|c| c.id).collect()
    }

    /// Find a column by its stable UUID.
    pub fn column_by_id(&self, id: &Uuid) -> Option<&Column> {
        self.0.iter().find(|c| &c.id == id)
    }

    /// Find a column by name.
    pub fn column_by_name(&self, name: &str) -> Option<&Column> {
        self.0.iter().find(|c| c.name == name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_schema_roundtrip() {
        let schema = Schema(vec![
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
                name: "geom".into(),
                data_type: DataType::Geometry,
                primary_key_index: None,
                size: None,
                geometry_type: Some("MULTIPOLYGON".into()),
                geometry_crs: Some("EPSG:4326".into()),
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
        ]);

        let json = serde_json::to_string_pretty(&schema).unwrap();
        let parsed: Schema = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.0.len(), 3);
        assert_eq!(parsed.primary_key_columns().len(), 1);
        assert_eq!(parsed.primary_key_columns()[0].name, "fid");
    }
}
