//! Export project source to a zip archive or a folder. Dependency folders, build output,
//! VCS metadata and secret files (`.env`, keys) are excluded.

use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::Path;

use crate::error::{GeniusError, Result};
use crate::workspace::{is_secret_name, Workspace, IGNORED_DIRS};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportResult {
    pub destination: String,
    pub files: usize,
    pub bytes: u64,
    pub skipped: Vec<String>,
}

fn collect(root: &Path) -> (Vec<(String, std::path::PathBuf)>, Vec<String>) {
    let mut files = Vec::new();
    let mut skipped = Vec::new();
    let walker = walkdir::WalkDir::new(root).follow_links(false).sort_by_file_name().into_iter().filter_entry(|e| {
        !(e.file_type().is_dir() && e.depth() > 0 && IGNORED_DIRS.contains(&e.file_name().to_string_lossy().as_ref()))
    });
    for e in walker.flatten() {
        if !e.file_type().is_file() {
            continue;
        }
        let Ok(rel) = e.path().strip_prefix(root) else { continue };
        let rel = crate::workspace::to_slash(rel);
        let name = rel.rsplit('/').next().unwrap_or(&rel);
        if is_secret_name(name) {
            skipped.push(rel);
            continue;
        }
        files.push((rel, e.path().to_path_buf()));
    }
    (files, skipped)
}

pub fn export_zip(ws: &Workspace, dest: &Path, top_dir: &str) -> Result<ExportResult> {
    if dest.starts_with(ws.root()) {
        return Err(GeniusError::Validation("Choose a destination outside the project folder.".into()));
    }
    let (files, skipped) = collect(ws.root());
    let tmp = dest.with_extension("zip.partial");
    let f = std::fs::File::create(&tmp)?;
    let mut zip = zip::ZipWriter::new(f);
    let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    let mut bytes = 0u64;
    let result = (|| -> Result<()> {
        for (rel, abs) in &files {
            let data = std::fs::read(abs)?;
            bytes += data.len() as u64;
            zip.start_file(format!("{top_dir}/{rel}"), opts).map_err(|e| GeniusError::Internal(e.to_string()))?;
            zip.write_all(&data)?;
        }
        zip.finish().map_err(|e| GeniusError::Internal(e.to_string()))?;
        Ok(())
    })();
    if let Err(e) = result {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    std::fs::rename(&tmp, dest)?;
    Ok(ExportResult { destination: dest.to_string_lossy().into(), files: files.len(), bytes, skipped })
}

pub fn export_folder(ws: &Workspace, dest: &Path) -> Result<ExportResult> {
    if dest.starts_with(ws.root()) {
        return Err(GeniusError::Validation("Choose a destination outside the project folder.".into()));
    }
    if dest.exists() && std::fs::read_dir(dest)?.next().is_some() {
        return Err(GeniusError::Conflict("The destination folder is not empty. Choose an empty or new folder.".into()));
    }
    let (files, skipped) = collect(ws.root());
    let mut bytes = 0;
    for (rel, abs) in &files {
        let target = dest.join(rel);
        if let Some(p) = target.parent() {
            std::fs::create_dir_all(p)?;
        }
        bytes += std::fs::copy(abs, &target)?;
    }
    Ok(ExportResult { destination: dest.to_string_lossy().into(), files: files.len(), bytes, skipped })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zip_excludes_deps_and_secrets() {
        let d = tempfile::tempdir().unwrap();
        let ws = Workspace::new(d.path().join("p")).unwrap();
        ws.write_text("src/App.tsx", "x").unwrap();
        ws.write_text("package.json", "{}").unwrap();
        std::fs::create_dir_all(ws.root().join("node_modules/a")).unwrap();
        std::fs::write(ws.root().join("node_modules/a/i.js"), "x").unwrap();
        std::fs::write(ws.root().join(".env"), "K=1").unwrap();
        let dest = d.path().join("out.zip");
        let r = export_zip(&ws, &dest, "p").unwrap();
        assert_eq!(r.files, 2);
        assert_eq!(r.skipped, vec![".env"]);
        let mut z = zip::ZipArchive::new(std::fs::File::open(&dest).unwrap()).unwrap();
        let mut names: Vec<String> = (0..z.len()).map(|i| z.by_index(i).unwrap().name().to_string()).collect();
        names.sort();
        assert_eq!(names, vec!["p/package.json", "p/src/App.tsx"]);
    }

    #[test]
    fn folder_export_refuses_non_empty_or_inside() {
        let d = tempfile::tempdir().unwrap();
        let ws = Workspace::new(d.path().join("p")).unwrap();
        ws.write_text("a.txt", "x").unwrap();
        assert!(export_folder(&ws, &ws.root().join("sub")).is_err());
        let out = d.path().join("out");
        assert_eq!(export_folder(&ws, &out).unwrap().files, 1);
        assert!(export_folder(&ws, &out).is_err(), "now non-empty");
    }
}
