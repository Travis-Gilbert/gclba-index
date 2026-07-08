use crate::adapters::parse_adapter_bytes;
use crate::models::{AdapterKind, IndexPayload, NormalizedRecord};
use crate::resolution::build_index;
use anyhow::{bail, Context, Result};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Debug)]
pub struct ExportStore {
    root: PathBuf,
}

impl ExportStore {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn save_export(&self, adapter: AdapterKind, bytes: &[u8]) -> Result<PathBuf> {
        if bytes.is_empty() {
            bail!("export body is empty");
        }
        let records = parse_adapter_bytes(adapter, bytes)?;
        if records.is_empty() {
            bail!("export parsed zero records");
        }

        let dir = self.root.join(adapter.as_str());
        fs::create_dir_all(&dir)
            .with_context(|| format!("failed to create export directory {}", dir.display()))?;
        let path = self.next_path(adapter, bytes);
        fs::write(&path, bytes)
            .with_context(|| format!("failed to write export {}", path.display()))?;
        Ok(path)
    }

    pub fn rebuild_index(&self) -> Result<IndexPayload> {
        Ok(build_index(self.load_records()?))
    }

    pub fn load_records(&self) -> Result<Vec<NormalizedRecord>> {
        let mut records = Vec::new();
        for (adapter, path) in self.export_files()? {
            let bytes = fs::read(&path)
                .with_context(|| format!("failed to read export {}", path.display()))?;
            records.extend(parse_adapter_bytes(adapter, &bytes).with_context(|| {
                format!(
                    "failed to parse {} export {}",
                    adapter.as_str(),
                    path.display()
                )
            })?);
        }
        Ok(records)
    }

    fn export_files(&self) -> Result<Vec<(AdapterKind, PathBuf)>> {
        if !self.root.exists() {
            return Ok(Vec::new());
        }

        let mut adapter_dirs = sorted_paths(&self.root)?;
        adapter_dirs.retain(|path| path.is_dir());

        let mut files = Vec::new();
        for dir in adapter_dirs {
            let adapter_name = dir
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or_default();
            let adapter: AdapterKind = adapter_name
                .parse()
                .map_err(|err: String| anyhow::anyhow!("{err}"))?;
            let mut adapter_files = sorted_paths(&dir)?;
            adapter_files.retain(|path| path.is_file());
            files.extend(adapter_files.into_iter().map(|path| (adapter, path)));
        }
        Ok(files)
    }

    fn next_path(&self, adapter: AdapterKind, bytes: &[u8]) -> PathBuf {
        let dir = self.root.join(adapter.as_str());
        let extension = export_extension(bytes);
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_millis())
            .unwrap_or_default();
        for index in 0.. {
            let suffix = if index == 0 {
                String::new()
            } else {
                format!("-{index}")
            };
            let path = dir.join(format!(
                "{stamp}-{}{}.{extension}",
                std::process::id(),
                suffix
            ));
            if !path.exists() {
                return path;
            }
        }
        unreachable!("unbounded export filename search must return");
    }
}

fn sorted_paths(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut paths = Vec::new();
    for entry in
        fs::read_dir(dir).with_context(|| format!("failed to read directory {}", dir.display()))?
    {
        paths.push(entry?.path());
    }
    paths.sort();
    Ok(paths)
}

fn export_extension(bytes: &[u8]) -> &'static str {
    let trimmed = bytes
        .iter()
        .copied()
        .find(|byte| !byte.is_ascii_whitespace());
    match trimmed {
        Some(b'{') | Some(b'[') => "json",
        _ => "csv",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn export_store_replays_multiple_saved_sources() {
        let root = test_dir("export-store-replay");
        let store = ExportStore::new(&root);
        let portal = "parcel_id,address,owner,gclba_owned,property_class\n\
                      40-12-345-067,123 North Saginaw Street,GCLBA,true,Residential";
        let filemaker = "parcel_id,address,program\n\
                         40-12-345-067,123 North Saginaw Street,Homebuyer";

        store
            .save_export(AdapterKind::SiteControlExport, portal.as_bytes())
            .unwrap();
        store
            .save_export(AdapterKind::FilemakerCsv, filemaker.as_bytes())
            .unwrap();

        let payload = store.rebuild_index().unwrap();
        assert_eq!(payload.coverage.parcels_indexed, 1);
        assert_eq!(
            payload.coverage.per_source.get("site_control_export"),
            Some(&1)
        );
        assert_eq!(payload.coverage.per_source.get("filemaker_csv"), Some(&1));

        fs::remove_dir_all(root).ok();
    }

    fn test_dir(name: &str) -> PathBuf {
        let mut path = std::env::temp_dir();
        path.push(format!(
            "gclba-index-{name}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        path
    }
}
