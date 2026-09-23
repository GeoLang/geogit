use anyhow::{Context, Result};
use geogit_core::dataset::DatasetMeta;
use geogit_encoding::feature::StoredFeature;
use geogit_encoding::legend::Legend;
use geogit_encoding::value::ColumnValue;

use crate::Repository;

/// Builder for constructing dataset tree structures on disk.
pub struct TreeBuilder<'a> {
    repo: &'a Repository,
}

impl<'a> TreeBuilder<'a> {
    pub fn new(repo: &'a Repository) -> Self {
        Self { repo }
    }

    /// Write a dataset's tree structure to the working directory.
    ///
    /// Creates the .table-dataset layout with meta/ and feature/ subtrees.
    pub fn import_dataset(
        &self,
        dataset_path: &str,
        meta: &DatasetMeta,
        features: &[(Vec<ColumnValue>, StoredFeature)],
    ) -> Result<()> {
        let base = self.repo.workdir.join(dataset_path).join(".table-dataset");
        let meta_dir = base.join("meta");
        let feature_dir = base.join("feature");
        let legend_dir = meta_dir.join("legend");

        std::fs::create_dir_all(&legend_dir).context("create legend dir")?;
        std::fs::create_dir_all(&feature_dir).context("create feature dir")?;

        // Write meta/title
        std::fs::write(meta_dir.join("title"), &meta.title).context("write title")?;

        // Write meta/description
        std::fs::write(meta_dir.join("description"), &meta.description)
            .context("write description")?;

        // Write meta/schema.json
        let schema_json = serde_json::to_string_pretty(&meta.schema).context("serialize schema")?;
        std::fs::write(meta_dir.join("schema.json"), schema_json).context("write schema")?;

        for (identifier, definition) in &meta.crs_definitions {
            let crs_dir = meta_dir.join("crs");
            std::fs::create_dir_all(&crs_dir).context("create crs dir")?;
            let stem = geogit_encoding::crs::crs_file_stem(identifier);
            std::fs::write(crs_dir.join(format!("{stem}.wkt")), definition)
                .with_context(|| format!("write crs {identifier}"))?;
        }

        // Write meta/path-structure.json
        let ps_json = serde_json::to_string_pretty(&meta.path_structure)
            .context("serialize path structure")?;
        std::fs::write(meta_dir.join("path-structure.json"), ps_json)
            .context("write path structure")?;

        // Build legend from non-PK columns (stored features only contain non-PK values)
        let non_pk_ids: Vec<uuid::Uuid> = meta
            .schema
            .0
            .iter()
            .filter(|c| c.primary_key_index.is_none())
            .map(|c| c.id)
            .collect();
        let legend = Legend::new(non_pk_ids);
        let legend_hash = legend.hash();
        std::fs::write(legend_dir.join(&legend_hash), legend.to_msgpack())
            .context("write legend")?;

        // Write features
        for (pk, feature) in features {
            let rel_path = meta.path_structure.feature_path(pk);
            let full_path = feature_dir.join(&rel_path);
            if let Some(parent) = full_path.parent() {
                std::fs::create_dir_all(parent)
                    .with_context(|| format!("create dir for feature {rel_path}"))?;
            }
            std::fs::write(&full_path, feature.to_msgpack())
                .with_context(|| format!("write feature {rel_path}"))?;
        }

        Ok(())
    }
}
