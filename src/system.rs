use crate::projects::ProjectType;
use std::collections::HashSet;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

/// Download caches that package managers keep outside `~/.cache`. Everything
/// here is re-fetched on demand.
const PACKAGE_CACHES: &[&str] = &[
    ".npm/_cacache",
    ".bun/install/cache",
    ".local/share/pnpm/store",
    ".cargo/registry",
    ".cargo/git",
    ".gradle/caches",
    ".m2/repository",
    ".nuget/packages",
    ".pub-cache",
    "go/pkg/mod",
];

/// A well-known place on the machine that holds disposable data.
struct Location {
    path: PathBuf,
    kind: ProjectType,
    /// Offer each entry inside separately rather than the location as a whole.
    each_child: bool,
}

fn home() -> Option<PathBuf> {
    env::var_os("HOME").map(PathBuf::from).filter(|home| home.is_absolute())
}

fn locations() -> Vec<Location> {
    let mut all = Vec::new();
    let mut add = |path: PathBuf, kind, each_child| {
        // Resolve symlinks so /tmp and a $TMPDIR pointing at it are one place.
        if let Ok(path) = path.canonicalize()
            && path.is_dir()
            && !all.iter().any(|known: &Location| known.path == path)
        {
            all.push(Location { path, kind, each_child });
        }
    };

    if let Some(home) = home() {
        let xdg = env::var_os("XDG_CACHE_HOME").map(PathBuf::from).filter(|dir| dir.is_absolute());
        add(xdg.unwrap_or_else(|| home.join(".cache")), ProjectType::Cache, true);
        add(home.join("Library/Caches"), ProjectType::Cache, true);
        for cache in PACKAGE_CACHES {
            add(home.join(cache), ProjectType::Packages, false);
        }
        add(home.join(".local/share/Trash"), ProjectType::Trash, false);
        add(home.join(".Trash"), ProjectType::Trash, true);
    }
    add(PathBuf::from("/tmp"), ProjectType::Temp, true);
    add(PathBuf::from("/var/tmp"), ProjectType::Temp, true);
    add(env::temp_dir(), ProjectType::Temp, true);
    all
}

#[cfg(unix)]
fn owner(meta: &fs::Metadata) -> Option<u32> {
    use std::os::unix::fs::MetadataExt;
    Some(meta.uid())
}

#[cfg(not(unix))]
fn owner(_: &fs::Metadata) -> Option<u32> {
    None
}

/// Directories the project scan should stay out of, because their contents
/// are offered here instead.
pub fn roots() -> Vec<PathBuf> {
    locations().into_iter().map(|location| location.path).collect()
}

/// Everything deletable in the well-known locations, for the enabled kinds.
pub fn candidates(enabled: &HashSet<ProjectType>) -> Vec<(PathBuf, ProjectType)> {
    // The home directory's owner stands in for "me" without needing libc.
    let me = home().and_then(|home| fs::metadata(home).ok()).and_then(|meta| owner(&meta));
    let mut found = Vec::new();

    for location in locations().into_iter().filter(|location| enabled.contains(&location.kind)) {
        if !location.each_child {
            found.push((location.path, location.kind));
            continue;
        }
        let Ok(entries) = fs::read_dir(&location.path) else {
            continue;
        };
        for entry in entries.flatten() {
            // Plain files and directories only: sockets, pipes and symlinks
            // in a temp directory belong to something that is running.
            let Ok(meta) = entry.metadata() else { continue };
            if !(meta.is_dir() || meta.is_file()) {
                continue;
            }
            // Shared temp directories hold other users' files too.
            if location.kind == ProjectType::Temp && owner(&meta) != me {
                continue;
            }
            found.push((entry.path(), location.kind));
        }
    }
    found
}

/// Whether `path` is one of the well-known locations or inside one.
pub fn covers(roots: &[PathBuf], path: &Path) -> bool {
    roots.iter().any(|root| path.starts_with(root))
}
