use crate::projects::{get_cleanable_dirs, ProjectType};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

#[derive(Debug, Clone)]
pub struct FoundDir {
    pub path: PathBuf,
    pub project_type: ProjectType,
    pub size_bytes: u64,
}

impl FoundDir {
    pub fn size_human(&self) -> String {
        format_size(self.size_bytes)
    }
}

fn format_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;

    if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else {
        format!("{} B", bytes)
    }
}

fn dir_size(path: &Path) -> u64 {
    WalkDir::new(path)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter_map(|e| e.metadata().ok())
        .filter(|m| m.is_file())
        .map(|m| m.len())
        .sum()
}

pub fn scan(root: &Path, enabled_types: &HashSet<ProjectType>) -> Vec<FoundDir> {
    let cleanable_dirs = get_cleanable_dirs();
    let mut found: Vec<FoundDir> = Vec::new();
    let mut skip_prefixes: Vec<PathBuf> = Vec::new();

    for entry in WalkDir::new(root).into_iter().filter_map(|e| e.ok()) {
        if !entry.file_type().is_dir() {
            continue;
        }

        let path = entry.path();

        // Skip if inside a previously found cleanable dir
        if skip_prefixes.iter().any(|prefix| path.starts_with(prefix)) {
            continue;
        }

        let Some(dir_name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };

        // Check against each cleanable directory pattern
        for cleanable in &cleanable_dirs {
            if !enabled_types.contains(&cleanable.project_type) {
                continue;
            }

            if dir_name == cleanable.dir_name && (cleanable.validator)(path) {
                let size_bytes = dir_size(path);
                found.push(FoundDir {
                    path: path.to_path_buf(),
                    project_type: cleanable.project_type,
                    size_bytes,
                });
                skip_prefixes.push(path.to_path_buf());
                break;
            }
        }
    }

    // Sort by size descending
    found.sort_by(|a, b| b.size_bytes.cmp(&a.size_bytes));
    found
}

pub fn total_size(dirs: &[FoundDir]) -> u64 {
    dirs.iter().map(|d| d.size_bytes).sum()
}
