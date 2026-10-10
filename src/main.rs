mod projects;
mod scanner;
mod system;
mod tree;
mod tui;

use clap::Parser;
use projects::ProjectType;
use scanner::{Disk, Found, Progress, age, human, tilde};
use std::collections::HashSet;
use std::fmt::Display;
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime};
use std::{env, fs, thread};

#[derive(Parser)]
#[command(name = "safeclean", version)]
#[command(about = "Find and delete build artifacts, caches and temp files to reclaim disk space")]
struct Cli {
    /// Directory to scan for projects. Without one, scans the current
    /// directory plus the machine's cache and temp locations
    path: Option<PathBuf>,

    /// Only look for these kinds (repeatable or comma-separated)
    #[arg(short = 't', long = "type", value_enum, value_delimiter = ',', value_name = "TYPE")]
    types: Vec<ProjectType>,

    /// Include ~/.cache, package caches, /tmp and the trash even when a
    /// directory or -y is given
    #[arg(short = 's', long)]
    system: bool,

    /// Leave the machine's cache and temp locations out
    #[arg(short = 'S', long, conflicts_with = "system")]
    no_system: bool,

    /// Go through the motions without deleting anything
    #[arg(short = 'n', long)]
    dry_run: bool,

    /// Delete everything found without asking
    #[arg(short = 'y', long, conflicts_with = "dry_run")]
    yes: bool,

    /// Estimate sizes by sampling large directories instead of measuring every file
    #[arg(long)]
    fast: bool,

    /// Show the UI with simulated data (nothing is deleted)
    #[arg(long)]
    demo: bool,
}

static COLOR: AtomicBool = AtomicBool::new(false);

const BOLD: &str = "1";
const DIM: &str = "2";
const RED: &str = "31";
const GREEN: &str = "32";

/// Wraps `text` in an SGR sequence when stdout is a colour terminal.
fn paint(code: &str, text: impl Display) -> String {
    if COLOR.load(Ordering::Relaxed) {
        format!("\x1b[{code}m{text}\x1b[0m")
    } else {
        text.to_string()
    }
}

fn plural(count: usize) -> &'static str {
    if count == 1 { "item" } else { "items" }
}

fn demo_data(enabled: &HashSet<ProjectType>, system: bool) -> (PathBuf, Vec<Found>) {
    use ProjectType::*;
    const MB: u64 = 1_000_000;
    const DAY: u64 = 24 * 3600;
    let root = PathBuf::from("/home/user/projects");
    let found = [
        ("api-server/target", Rust, 1_892 * MB, 0),
        ("cli-tool/target", Rust, 456 * MB, 64),
        ("experiments/parsers/json/target", Rust, 234 * MB, 410),
        ("monorepo/node_modules", Node, 892 * MB, 2),
        ("monorepo/apps/web/node_modules", Node, 654 * MB, 2),
        ("monorepo/apps/web/.next", Web, 345 * MB, 2),
        ("monorepo/apps/docs/node_modules", Node, 212 * MB, 19),
        ("monorepo/apps/docs/.svelte-kit", Web, 48 * MB, 19),
        ("monorepo/packages/ui/node_modules", Node, 97 * MB, 33),
        ("blog/node_modules", Node, 423 * MB, 240),
        ("ml-pipeline/.venv", Python, 1_234 * MB, 9),
        ("ml-pipeline/src/__pycache__", Python, 3 * MB, 9),
        ("ml-pipeline/src/models/__pycache__", Python, 9 * MB, 9),
        ("ml-pipeline/.pytest_cache", Python, MB / 4, 9),
        ("data-analysis/.venv", Python, 567 * MB, 130),
        ("android-app/build", Gradle, 789 * MB, 800),
        ("android-app/.gradle", Gradle, 234 * MB, 800),
        ("infra/staging/.terraform", Terraform, 310 * MB, 51),
        ("~/.cache/pip", Cache, 1_420 * MB, 9),
        ("~/.cache/uv", Cache, 2_310 * MB, 1),
        ("~/.cache/google-chrome", Cache, 880 * MB, 0),
        ("~/.cache/thumbnails", Cache, 96 * MB, 3),
        ("~/.npm/_cacache", Packages, 3_150 * MB, 2),
        ("~/.cargo/registry", Packages, 2_740 * MB, 0),
        ("~/.gradle/caches", Packages, 5_900 * MB, 800),
        ("/tmp/rust-analyzer-build", Temp, 610 * MB, 12),
        ("/tmp/download.iso", Temp, 1_900 * MB, 45),
        ("~/.local/share/Trash", Trash, 4_200 * MB, 6),
    ]
    .into_iter()
    .filter(|(_, kind, ..)| enabled.contains(kind) && (system || !kind.is_system()))
    .map(|(rel, kind, size, days)| Found {
        path: root.join(rel),
        rel: rel.into(),
        kind,
        size,
        modified: SystemTime::now().checked_sub(Duration::from_secs(days * DAY + 60)),
    })
    .collect();
    (root, found)
}

/// Runs the scan on a worker thread, showing live progress on stderr.
fn scan_with_spinner(root: &Path, system: bool, enabled: &HashSet<ProjectType>, fast: bool) -> Vec<Found> {
    const FRAMES: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
    let progress = Progress::default();
    let animate = io::stderr().is_terminal();

    thread::scope(|scope| {
        let worker = scope.spawn(|| scanner::scan(root, system, enabled, fast, &progress));
        let mut stderr = io::stderr();
        let mut tick = 0;
        while animate && !worker.is_finished() {
            let _ = write!(
                stderr,
                "\r\x1b[2K{} Scanning {} · {} found, {}",
                FRAMES[tick % FRAMES.len()],
                tilde(root).display(),
                progress.found.load(Ordering::Relaxed),
                human(progress.bytes.load(Ordering::Relaxed)),
            );
            let _ = stderr.flush();
            tick += 1;
            thread::sleep(Duration::from_millis(80));
        }
        if animate {
            let _ = write!(stderr, "\r\x1b[2K");
        }
        worker.join().expect("scan thread panicked")
    })
}

fn print_listing(found: &[Found]) {
    // One absurdly long name should not push every size off the screen.
    let width = found.iter().map(|f| f.rel.display().to_string().chars().count()).max().unwrap_or(0).min(60);
    let mut groups: Vec<(ProjectType, Vec<&Found>)> = ProjectType::ALL
        .iter()
        .map(|&kind| (kind, found.iter().filter(|f| f.kind == kind).collect::<Vec<_>>()))
        .filter(|(_, items)| !items.is_empty())
        .collect();
    groups.sort_by_key(|(_, items)| std::cmp::Reverse(items.iter().map(|f| f.size).sum::<u64>()));

    for (kind, items) in groups {
        let size: u64 = items.iter().map(|f| f.size).sum();
        println!(
            "{}  {}",
            paint(BOLD, kind.name()),
            paint(DIM, format!("{} {}, {}", items.len(), plural(items.len()), human(size)))
        );
        for item in items {
            println!(
                "  {:<width$}  {:>9}  {}",
                item.rel.display(),
                human(item.size),
                paint(DIM, format!("{:>4}", age(item.modified)))
            );
        }
        println!();
    }

    let total: u64 = found.iter().map(|f| f.size).sum();
    println!(
        "{} {} in {} {}",
        paint(BOLD, "Total:"),
        paint(GREEN, human(total)),
        found.len(),
        plural(found.len())
    );
}

/// Shows where the disk stands now: used, just reclaimed, and free.
fn print_disk(before: Disk, now_free: u64) {
    const WIDTH: usize = 32;
    let percent_used = |free: u64| 100.0 * before.total.saturating_sub(free) as f64 / before.total.max(1) as f64;
    let cells = |bytes: u64| (bytes as f64 / before.total.max(1) as f64 * WIDTH as f64).round() as usize;

    let gained = now_free.saturating_sub(before.free);
    // Always show at least a sliver for what was reclaimed.
    let reclaimed = cells(gained).max((gained > 0) as usize).min(WIDTH);
    let free = cells(before.free).min(WIDTH - reclaimed);
    let used = WIDTH - reclaimed - free;

    println!();
    println!(
        "  {}{}{}  {} disk",
        "█".repeat(used),
        paint(GREEN, "█".repeat(reclaimed)),
        paint(DIM, "░".repeat(free)),
        human(before.total)
    );
    println!("  free  {} → {}", human(before.free), paint(GREEN, paint(BOLD, human(now_free))));
    println!("  used  {:.0}% → {:.0}%", percent_used(before.free), percent_used(now_free));
}

/// Makes every directory under `path` writable by its owner. Some caches,
/// Go's module cache among them, are stored read-only.
#[cfg(unix)]
fn unlock(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let Ok(meta) = fs::symlink_metadata(path) else { return };
    if !meta.is_dir() {
        return;
    }
    let mut perms = meta.permissions();
    perms.set_mode(perms.mode() | 0o700);
    let _ = fs::set_permissions(path, perms);
    if let Ok(entries) = fs::read_dir(path) {
        for entry in entries.flatten() {
            unlock(&entry.path());
        }
    }
}

#[cfg(not(unix))]
fn unlock(_: &Path) {}

fn remove(path: &Path) -> io::Result<()> {
    // Whatever is there now is judged afresh: a symlink swapped in since the
    // scan must not be followed.
    let meta = fs::symlink_metadata(path)?;
    if meta.is_file() {
        return fs::remove_file(path);
    }
    if !meta.is_dir() {
        return Err(io::Error::other("no longer a file or directory"));
    }
    match fs::remove_dir_all(path) {
        Err(err) if err.kind() == io::ErrorKind::PermissionDenied => {
            unlock(path);
            fs::remove_dir_all(path)
        }
        result => result,
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let interactive = io::stdout().is_terminal() && io::stdin().is_terminal();
    COLOR.store(io::stdout().is_terminal() && env::var_os("NO_COLOR").is_none(), Ordering::Relaxed);

    let enabled: HashSet<ProjectType> = if cli.types.is_empty() {
        ProjectType::ALL.iter().copied().collect()
    } else {
        cli.types.iter().copied().collect()
    };
    // Pointing at a directory means that directory, and -y is too blunt to
    // be let loose on the whole machine unasked.
    let system = !cli.no_system && (cli.system || (cli.path.is_none() && !cli.yes));

    let (root, found) = if cli.demo {
        demo_data(&enabled, system)
    } else {
        let path = cli.path.clone().unwrap_or_else(|| PathBuf::from("."));
        let root = match path.canonicalize() {
            Ok(root) if root.is_dir() => root,
            _ => {
                eprintln!("{} not a directory: {}", paint(RED, "error:"), path.display());
                return ExitCode::FAILURE;
            }
        };
        let found = scan_with_spinner(&root, system, &enabled, cli.fast);
        (root, found)
    };

    let disk = if cli.demo { Some(Disk { free: 182 << 30, total: 1 << 40 }) } else { Disk::of(&root) };
    let dry = cli.dry_run || cli.demo;
    let scope = match system {
        true => format!("{} + system", tilde(&root).display()),
        false => tilde(&root).display().to_string(),
    };

    if found.is_empty() {
        println!("Nothing to clean in {scope}");
        return ExitCode::SUCCESS;
    }

    let chosen: Vec<usize> = if cli.yes {
        (0..found.len()).collect()
    } else if !interactive {
        print_listing(&found);
        if !dry {
            eprintln!("\nNot a terminal, so nothing was deleted. Pass -y to delete everything listed.");
        }
        return ExitCode::SUCCESS;
    } else {
        match tui::run(&found, &scope, disk, dry) {
            Ok(chosen) => chosen,
            Err(err) => {
                eprintln!("{} {err}", paint(RED, "error:"));
                return ExitCode::FAILURE;
            }
        }
    };

    if chosen.is_empty() {
        println!("Nothing deleted.");
        return ExitCode::SUCCESS;
    }

    // Per-item lines would scroll the result away, so show progress in
    // place and summarise by kind afterwards.
    let progress = io::stderr().is_terminal();
    let mut freed: Vec<(ProjectType, usize, u64)> = Vec::new();
    let mut failures: Vec<(&Found, io::Error)> = Vec::new();
    for (done, &i) in chosen.iter().enumerate() {
        let item = &found[i];
        if progress {
            let name: String = item.rel.display().to_string().chars().take(60).collect();
            eprint!("\r\x1b[2K{} {}/{}  {name}", paint(DIM, "Deleting"), done + 1, chosen.len());
        }
        match if dry { Ok(()) } else { remove(&item.path) } {
            Ok(()) => match freed.iter_mut().find(|(kind, ..)| *kind == item.kind) {
                Some((_, count, size)) => {
                    *count += 1;
                    *size += item.size;
                }
                None => freed.push((item.kind, 1, item.size)),
            },
            Err(err) => failures.push((item, err)),
        }
    }
    if progress {
        eprint!("\r\x1b[2K");
    }
    freed.sort_by_key(|&(_, _, size)| std::cmp::Reverse(size));

    let deleted: usize = freed.iter().map(|&(_, count, _)| count).sum();
    let total: u64 = freed.iter().map(|&(_, _, size)| size).sum();
    let now_free = match (dry, disk) {
        (true, Some(before)) => Some(before.free + total),
        (false, Some(_)) => Disk::of(&root).map(|after| after.free),
        _ => None,
    };

    let headline = if dry { "Dry run, would have freed" } else { "Freed" };
    println!("{headline} {} in {deleted} {}", paint(GREEN, paint(BOLD, human(total))), plural(deleted));
    if !freed.is_empty() {
        println!();
    }
    let width = freed.iter().map(|(kind, ..)| kind.name().len()).max().unwrap_or(0);
    for (kind, count, size) in &freed {
        println!(
            "  {:<width$}  {}  {:>9}",
            kind.name(),
            paint(DIM, format!("{count:>5} {:<5}", plural(*count))),
            human(*size)
        );
    }
    if let (Some(before), Some(now)) = (disk, now_free) {
        print_disk(before, now);
    }
    if !failures.is_empty() {
        println!();
        println!("{}", paint(RED, format!("Could not delete {} {}", failures.len(), plural(failures.len()))));
        for (item, err) in &failures {
            println!("  {}  {}", item.rel.display(), paint(DIM, err));
        }
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}
