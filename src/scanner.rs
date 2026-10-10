use crate::projects::{self, ProjectType};
use rayon::prelude::*;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Mutex;

#[derive(Debug, Clone)]
pub struct Found {
    pub path: PathBuf,
    /// Path relative to the scan root, used for display.
    pub rel: PathBuf,
    pub kind: ProjectType,
    pub size: u64,
}

/// Live counters a caller can poll while a scan runs on another thread.
#[derive(Default)]
pub struct Progress {
    pub found: AtomicUsize,
    pub bytes: AtomicU64,
}

pub fn human(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["KB", "MB", "GB", "TB"];
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut value = bytes as f64 / 1024.0;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    format!("{value:.1} {}", UNITS[unit])
}

/// Space actually occupied on disk, like `du`, rather than apparent length.
#[cfg(unix)]
fn disk_usage(meta: &std::fs::Metadata) -> u64 {
    use std::os::unix::fs::MetadataExt;
    meta.blocks() * 512
}

#[cfg(not(unix))]
fn disk_usage(meta: &std::fs::Metadata) -> u64 {
    meta.len()
}

/// With `--fast`, directories holding more files than this have only this
/// many measured; the rest are assumed to average the same.
const FILE_SAMPLE: usize = 64;

/// What a directory entry itself occupies, near enough, without a stat.
const DIR_BLOCK: u64 = 4096;

/// Disk usage of everything under `path`, walked in parallel.
fn dir_size(path: &Path, fast: bool) -> u64 {
    let Ok(entries) = std::fs::read_dir(path) else {
        return 0;
    };
    let mut files = Vec::new();
    let mut dirs = Vec::new();
    for entry in entries.flatten() {
        match entry.file_type() {
            Ok(kind) if kind.is_dir() => dirs.push(entry.path()),
            Ok(_) => files.push(entry),
            Err(_) => {}
        }
    }

    let measure = |file: &std::fs::DirEntry| file.metadata().map_or(0, |m| disk_usage(&m));
    let own = if fast && files.len() > FILE_SAMPLE {
        // Listing order is effectively random, so evenly spaced picks are a
        // fair sample.
        let step = files.len() as f64 / FILE_SAMPLE as f64;
        let sum: u64 = (0..FILE_SAMPLE).map(|i| measure(&files[(i as f64 * step) as usize])).sum();
        (sum as f64 * step) as u64
    } else {
        files.iter().map(measure).sum()
    };
    DIR_BLOCK + own + dirs.par_iter().map(|dir| dir_size(dir, fast)).sum::<u64>()
}

#[cfg(unix)]
fn device(path: &Path) -> Option<u64> {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata(path).ok().map(|m| m.dev())
}

#[cfg(not(unix))]
fn device(_: &Path) -> Option<u64> {
    None
}

struct Search<'a> {
    enabled: &'a HashSet<ProjectType>,
    device: Option<u64>,
    progress: &'a Progress,
    hits: Mutex<Vec<(PathBuf, ProjectType)>>,
}

impl Search<'_> {
    fn walk(&self, dir: &Path) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        let mut descend = Vec::new();
        for entry in entries.flatten() {
            if !entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                continue;
            }
            let name = entry.file_name();
            let Some(name) = name.to_str() else { continue };
            if name == ".git" {
                continue;
            }
            let path = entry.path();
            // Never look for artifacts inside an artifact, wanted or not.
            match projects::classify(&path, name) {
                Some(kind) if self.enabled.contains(&kind) => {
                    self.progress.found.fetch_add(1, Ordering::Relaxed);
                    self.hits.lock().unwrap().push((path, kind));
                }
                Some(_) => {}
                None => descend.push(path),
            }
        }
        descend
            .par_iter()
            // Stay on the filesystem the scan started on.
            .filter(|path| self.device.is_none() || device(path) == self.device)
            .for_each(|path| self.walk(path));
    }
}

pub fn scan(root: &Path, enabled: &HashSet<ProjectType>, fast: bool, progress: &Progress) -> Vec<Found> {
    let search = Search { enabled, device: device(root), progress, hits: Mutex::new(Vec::new()) };
    search.walk(root);

    let mut found: Vec<Found> = search
        .hits
        .into_inner()
        .unwrap()
        .into_par_iter()
        .map(|(path, kind)| {
            let size = dir_size(&path, fast);
            progress.bytes.fetch_add(size, Ordering::Relaxed);
            let rel = path.strip_prefix(root).unwrap_or(&path).to_path_buf();
            Found { path, rel, kind, size }
        })
        .collect();

    found.sort_by(|a, b| b.size.cmp(&a.size).then_with(|| a.path.cmp(&b.path)));
    found
}

/// Space on the filesystem holding the scan root.
#[derive(Clone, Copy)]
pub struct Disk {
    pub free: u64,
    pub total: u64,
}

impl Disk {
    pub fn of(path: &Path) -> Option<Disk> {
        Some(Disk { free: fs4::available_space(path).ok()?, total: fs4::total_space(path).ok()? })
    }
}
