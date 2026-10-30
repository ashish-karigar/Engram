use std::path::{Path, PathBuf};

/// The list of project folders Engram watches, remembered across restarts.
pub struct Registry {
    file: PathBuf,
}

impl Registry {
    pub fn new(base: &Path) -> Self {
        Registry { file: base.join("projects.json") }
    }

    pub fn load(&self) -> Vec<PathBuf> {
        std::fs::read_to_string(&self.file)
            .ok()
            .and_then(|text| serde_json::from_str::<Vec<PathBuf>>(&text).ok())
            .unwrap_or_default()
    }

    pub fn add(&self, root: &Path) {
        let mut roots = self.load();
        if !roots.iter().any(|r| r == root) {
            roots.push(root.to_path_buf());
            self.save(&roots);
        }
    }

    pub fn remove(&self, root: &Path) {
        let roots: Vec<PathBuf> = self.load().into_iter().filter(|r| r != root).collect();
        self.save(&roots);
    }

    fn save(&self, roots: &[PathBuf]) {
        if let Some(dir) = self.file.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(&self.file, serde_json::to_string_pretty(roots).unwrap_or_default());
    }
}
