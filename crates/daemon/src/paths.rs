use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

/// Engram's data lives outside the project, so there is nothing to gitignore or leak.
pub fn app_data_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("ENGRAM_HOME") {
        return PathBuf::from(dir);
    }
    let home = PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
    if cfg!(target_os = "macos") {
        home.join("Library/Application Support/Engram")
    } else {
        std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).unwrap_or_else(|| home.join(".local/share")).join("engram")
    }
}

#[derive(Debug, Clone)]
pub struct ProjectPaths {
    pub id: String,
    pub name: String,
    pub root: PathBuf,
    pub dir: PathBuf,
    pub db: PathBuf,
    pub tokens: PathBuf,
}

pub fn project_paths(root: &Path, base: &Path) -> std::io::Result<ProjectPaths> {
    let root = root.canonicalize()?;
    let id = hex::encode(Sha256::digest(root.to_string_lossy().as_bytes()))[..12].to_string();
    let dir = base.join("projects").join(&id);
    let name = root.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "project".into());
    Ok(ProjectPaths { db: dir.join("engram.sqlite"), tokens: dir.join("tokens.json"), id, name, root, dir })
}
