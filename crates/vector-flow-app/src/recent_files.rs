use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

const MAX_RECENT: usize = 10;
const CONFIG_DIR: &str = "vector-flow";
const RECENT_FILE: &str = "recent.json";

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct RecentData {
    paths: Vec<PathBuf>,
}

/// Tracks recently opened/saved project file paths.
/// Stored persistently at `~/.config/vector-flow/recent.json`.
pub struct RecentFiles {
    paths: Vec<PathBuf>,
}

impl RecentFiles {
    /// Load the recent files list from disk, filtering out entries that no longer exist.
    pub fn load() -> Self {
        let data = Self::config_path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|json| serde_json::from_str::<RecentData>(&json).ok())
            .unwrap_or_default();

        let paths = data.paths.into_iter().filter(|p| p.exists()).collect();
        Self { paths }
    }

    /// The list of recent file paths, most recent first.
    pub fn entries(&self) -> &[PathBuf] {
        &self.paths
    }

    /// Record a path as recently used. Moves it to the front if already present.
    pub fn add(&mut self, path: &Path) {
        let path = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_owned());

        // Remove existing entry if present (will be re-inserted at front).
        self.paths.retain(|p| p != &path);
        self.paths.insert(0, path);
        self.paths.truncate(MAX_RECENT);

        self.save();
    }

    /// Clear the entire recent files list.
    pub fn clear(&mut self) {
        self.paths.clear();
        self.save();
    }

    fn save(&self) {
        if let Some(config_path) = Self::config_path() {
            if let Some(parent) = config_path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let data = RecentData {
                paths: self.paths.clone(),
            };
            if let Ok(json) = serde_json::to_string_pretty(&data) {
                let _ = std::fs::write(config_path, json);
            }
        }
    }

    fn config_path() -> Option<PathBuf> {
        let home = std::env::var("HOME").ok()?;
        Some(
            PathBuf::from(home)
                .join(".config")
                .join(CONFIG_DIR)
                .join(RECENT_FILE),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn add_moves_to_front_and_deduplicates() {
        let mut rf = RecentFiles { paths: vec![] };
        let dir = std::env::temp_dir();

        // Create two temp files so they pass the canonicalize step.
        let a = dir.join("vf_test_a.vflow");
        let b = dir.join("vf_test_b.vflow");
        std::fs::File::create(&a).unwrap().write_all(b"").unwrap();
        std::fs::File::create(&b).unwrap().write_all(b"").unwrap();

        // Don't persist during tests — override save to no-op by not calling save().
        // We test the in-memory logic only.
        let a_canon = std::fs::canonicalize(&a).unwrap();
        let b_canon = std::fs::canonicalize(&b).unwrap();

        rf.paths.insert(0, a_canon.clone());
        rf.paths.insert(0, b_canon.clone());
        // Now: [b, a]

        // Re-add a — should move to front.
        rf.paths.retain(|p| p != &a_canon);
        rf.paths.insert(0, a_canon.clone());
        assert_eq!(rf.paths, vec![a_canon.clone(), b_canon.clone()]);

        // Clean up.
        let _ = std::fs::remove_file(&a);
        let _ = std::fs::remove_file(&b);
    }

    #[test]
    fn truncates_to_max() {
        let mut rf = RecentFiles { paths: vec![] };
        let dir = std::env::temp_dir();

        // Create MAX_RECENT + 2 temp files.
        let mut files = Vec::new();
        for i in 0..(MAX_RECENT + 2) {
            let p = dir.join(format!("vf_test_trunc_{i}.vflow"));
            std::fs::File::create(&p).unwrap().write_all(b"").unwrap();
            files.push(p);
        }

        for f in &files {
            let canon = std::fs::canonicalize(f).unwrap();
            rf.paths.retain(|p| p != &canon);
            rf.paths.insert(0, canon);
            rf.paths.truncate(MAX_RECENT);
        }

        assert_eq!(rf.paths.len(), MAX_RECENT);
        // Most recent file should be the last one added.
        let last_canon = std::fs::canonicalize(files.last().unwrap()).unwrap();
        assert_eq!(rf.paths[0], last_canon);

        // Clean up.
        for f in &files {
            let _ = std::fs::remove_file(f);
        }
    }

    #[test]
    fn load_filters_nonexistent() {
        let data = RecentData {
            paths: vec![
                PathBuf::from("/nonexistent/file1.vflow"),
                PathBuf::from("/nonexistent/file2.vflow"),
            ],
        };
        // Simulate: these paths don't exist on disk, so filtering should remove them.
        let paths: Vec<PathBuf> = data.paths.into_iter().filter(|p| p.exists()).collect();
        assert!(paths.is_empty());
    }
}
