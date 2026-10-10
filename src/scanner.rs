use crate::projects::{self, ProjectType};
use crate::system;
use rayon::prelude::*;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

#[derive(Debug, Clone)]
pub struct Found {
    pub path: PathBuf,
    /// Path relative to the scan root, used for display.
    pub rel: PathBuf,
    pub kind: ProjectType,
    pub size: u64,
    /// When the project (or, outside projects, the entry) last changed.
    pub modified: Option<SystemTime>,
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

/// How long ago `modified` was, in the largest sensible unit.
pub fn age(modified: Option<SystemTime>) -> String {
    const HOUR: u64 = 3600;
    const DAY: u64 = 24 * HOUR;
    let Some(modified) = modified else {
        return String::new();
    };
    let secs = modified.elapsed().unwrap_or(Duration::ZERO).as_secs();
    match secs {
        s if s < HOUR => "now".to_string(),
        s if s < DAY => format!("{}h", s / HOUR),
        s if s < 14 * DAY => format!("{}d", s / DAY),
        s if s < 60 * DAY => format!("{}w", s / (7 * DAY)),
        s if s < 730 * DAY => format!("{}mo", s / (30 * DAY)),
        s => format!("{}y", s / (365 * DAY)),
    }
}

/// Most entries looked at per directory when dating something.
const DATING_LIMIT: usize = 256;

/// Newest modification time of `path` and what it holds, `depth` levels down.
fn newest(path: &Path, depth: usize) -> Option<SystemTime> {
    let meta = std::fs::symlink_metadata(path).ok()?;
    let mut newest = meta.modified().ok();
    if depth > 0 && meta.is_dir() {
        for entry in std::fs::read_dir(path).ok()?.flatten().take(DATING_LIMIT) {
            newest = newest.max(self::newest(&entry.path(), depth - 1));
        }
    }
    newest
}

/// When something was last worked on. For a project artifact that is the
/// project around it: its files, its top-level directories and what is in
/// them (which takes in `.git/index` and the artifact's own contents), so
/// both an edit and a build count. Elsewhere it is just the entry itself.
fn last_touched(path: &Path, kind: ProjectType) -> Option<SystemTime> {
    match path.parent() {
        Some(project) if !kind.is_system() => newest(project, 2),
        _ => newest(path, 1),
    }
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

/// Disk usage of a found entry, which outside of projects may be a file.
fn entry_size(path: &Path, fast: bool) -> u64 {
    match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.is_dir() => dir_size(path, fast),
        Ok(meta) => disk_usage(&meta),
        Err(_) => 0,
    }
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
    /// Directories handled elsewhere, not to be entered.
    skip: &'a [PathBuf],
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
            if self.skip.contains(&path) {
                continue;
            }
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

/// `path` with the home directory abbreviated to `~`.
pub fn tilde(path: &Path) -> PathBuf {
    match std::env::var_os("HOME") {
        Some(home) if !home.is_empty() => match path.strip_prefix(&home) {
            Ok(rest) => Path::new("~").join(rest),
            Err(_) => path.to_path_buf(),
        },
        _ => path.to_path_buf(),
    }
}

/// Finds project artifacts under `root` and, if `system` is set, disposable
/// data in the machine's well-known cache and temp locations.
pub fn scan(
    root: &Path,
    system: bool,
    enabled: &HashSet<ProjectType>,
    fast: bool,
    progress: &Progress,
) -> Vec<Found> {
    let skip = if system { system::roots() } else { Vec::new() };
    let mut hits = if system { system::candidates(enabled) } else { Vec::new() };
    progress.found.fetch_add(hits.len(), Ordering::Relaxed);

    // A scan started inside one of those locations has nothing to add.
    if !system::covers(&skip, root) {
        let search = Search { enabled, skip: &skip, device: device(root), progress, hits: Mutex::new(Vec::new()) };
        search.walk(root);
        hits.extend(search.hits.into_inner().unwrap());
    }

    let mut found: Vec<Found> = hits
        .into_par_iter()
        .map(|(path, kind)| {
            let size = entry_size(&path, fast);
            let modified = last_touched(&path, kind);
            progress.bytes.fetch_add(size, Ordering::Relaxed);
            // Project artifacts read best relative to where the scan began;
            // anything else needs its full path to make sense.
            let rel = match path.strip_prefix(root) {
                Ok(rel) if !kind.is_system() => rel.to_path_buf(),
                _ => tilde(&path),
            };
            Found { path, rel, kind, size, modified }
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
