use std::collections::BTreeMap;

use geogit_encoding::path::PathStructure;
use geogit_encoding::schema::Schema;
use serde::{Deserialize, Serialize};

/// Metadata for a table dataset (stored in .table-dataset/meta/).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatasetMeta {
    pub title: String,
    pub description: String,
    pub schema: Schema,
    pub path_structure: PathStructure,
    // identifier such as "EPSG:4326" to the well-known-text definition, written to meta/crs/
    #[serde(default)]
    pub crs_definitions: BTreeMap<String, String>,
}

/// Information about a dataset within a repository.
#[derive(Debug, Clone)]
pub struct DatasetInfo {
    /// Path within the repo tree, e.g. "parcels/city"
    pub path: String,
    /// Dataset metadata
    pub meta: DatasetMeta,
}

/// Recognized dataset types.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DatasetType {
    /// Vector/tabular data (.table-dataset)
    Table,
    /// Raster tiles (.raster-dataset) — future
    Raster,
    /// Point cloud (.pointcloud-dataset) — future
    PointCloud,
}

impl DatasetType {
    /// The directory name marker for this dataset type.
    pub fn dir_name(&self) -> &'static str {
        match self {
            DatasetType::Table => ".table-dataset",
            DatasetType::Raster => ".raster-dataset",
            DatasetType::PointCloud => ".pointcloud-dataset",
        }
    }

    /// Detect dataset type from a directory name.
    pub fn from_dir_name(name: &str) -> Option<Self> {
        match name {
            ".table-dataset" => Some(DatasetType::Table),
            ".raster-dataset" => Some(DatasetType::Raster),
            ".pointcloud-dataset" => Some(DatasetType::PointCloud),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dataset_type_roundtrip() {
        assert_eq!(
            DatasetType::from_dir_name(DatasetType::Table.dir_name()),
            Some(DatasetType::Table)
        );
        assert_eq!(DatasetType::from_dir_name("random"), None);
    }
}
