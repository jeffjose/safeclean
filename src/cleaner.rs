use crate::scanner::FoundDir;
use std::fs;
use std::io;

pub struct CleanResult {
    pub deleted: Vec<FoundDir>,
    pub failed: Vec<(FoundDir, io::Error)>,
}

impl CleanResult {
    pub fn total_cleaned(&self) -> u64 {
        self.deleted.iter().map(|d| d.size_bytes).sum()
    }
}

pub fn clean(dirs: Vec<FoundDir>) -> CleanResult {
    let mut deleted = Vec::new();
    let mut failed = Vec::new();

    for dir in dirs {
        match fs::remove_dir_all(&dir.path) {
            Ok(()) => deleted.push(dir),
            Err(e) => failed.push((dir, e)),
        }
    }

    CleanResult { deleted, failed }
}
