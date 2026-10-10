mod projects;
mod scanner;
mod tree;
mod tui;

use clap::Parser;
use projects::ProjectType;
use scanner::{Disk, Found, Progress, human};
use std::collections::HashSet;
use std::fmt::Display;
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use std::{env, fs, thread};

#[derive(Parser)]
#[command(name = "safeclean", version)]
#[command(about = "Find and delete build artifacts and dependency caches to reclaim disk space")]
struct Cli {
    /// Directory to scan
    #[arg(default_value = ".")]
    path: PathBuf,

    /// Only look for these project types (repeatable or comma-separated)
    #[arg(short = 't', long = "type", value_enum, value_delimiter = ',', value_name = "TYPE")]
    types: Vec<ProjectType>,

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
    if count == 1 { "directory" } else { "directories" }
}

fn tilde(path: &Path) -> String {
    if let Some(home) = env::var_os("HOME")
        && let Ok(rest) = path.strip_prefix(&home)
    {
        return Path::new("~").join(rest).display().to_string();
    }
    path.display().to_string()
}

fn demo_data(enabled: &HashSet<ProjectType>) -> (PathBuf, Vec<Found>) {
    use ProjectType::*;
    const MB: u64 = 1_000_000;
    let root = PathBuf::from("/home/user/projects");
    let found = [
        ("api-server/target", Rust, 1_892 * MB),
        ("cli-tool/target", Rust, 456 * MB),
        ("experiments/parsers/json/target", Rust, 234 * MB),
        ("monorepo/node_modules", Node, 892 * MB),
        ("monorepo/apps/web/node_modules", Node, 654 * MB),
        ("monorepo/apps/web/.next", Web, 345 * MB),
        ("monorepo/apps/docs/node_modules", Node, 212 * MB),
        ("monorepo/apps/docs/.svelte-kit", Web, 48 * MB),
        ("monorepo/packages/ui/node_modules", Node, 97 * MB),
        ("blog/node_modules", Node, 423 * MB),
        ("ml-pipeline/.venv", Python, 1_234 * MB),
        ("ml-pipeline/src/__pycache__", Python, 3 * MB),
        ("ml-pipeline/src/models/__pycache__", Python, 9 * MB),
        ("ml-pipeline/.pytest_cache", Python, MB / 4),
        ("data-analysis/.venv", Python, 567 * MB),
        ("android-app/build", Gradle, 789 * MB),
        ("android-app/.gradle", Gradle, 234 * MB),
        ("infra/staging/.terraform", Terraform, 310 * MB),
    ]
    .into_iter()
    .filter(|(_, kind, _)| enabled.contains(kind))
    .map(|(rel, kind, size)| Found { path: root.join(rel), rel: rel.into(), kind, size })
    .collect();
    (root, found)
}

/// Runs the scan on a worker thread, showing live progress on stderr.
fn scan_with_spinner(root: &Path, enabled: &HashSet<ProjectType>, fast: bool) -> Vec<Found> {
    const FRAMES: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
    let progress = Progress::default();
    let animate = io::stderr().is_terminal();

    thread::scope(|scope| {
        let worker = scope.spawn(|| scanner::scan(root, enabled, fast, &progress));
        let mut stderr = io::stderr();
        let mut tick = 0;
        while animate && !worker.is_finished() {
            let _ = write!(
                stderr,
                "\r\x1b[2K{} Scanning {} · {} found, {}",
                FRAMES[tick % FRAMES.len()],
                tilde(root),
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
    let width = found.iter().map(|f| f.rel.display().to_string().chars().count()).max().unwrap_or(0);
    let mut groups: Vec<(ProjectType, Vec<&Found>)> = ProjectType::ALL
        .iter()
        .map(|&kind| (kind, found.iter().filter(|f| f.kind == kind).collect::<Vec<_>>()))
        .filter(|(_, dirs)| !dirs.is_empty())
        .collect();
    groups.sort_by_key(|(_, dirs)| std::cmp::Reverse(dirs.iter().map(|f| f.size).sum::<u64>()));

    for (kind, dirs) in groups {
        let size: u64 = dirs.iter().map(|f| f.size).sum();
        println!(
            "{}  {}",
            paint(BOLD, kind.name()),
            paint(DIM, format!("{} {}, {}", dirs.len(), plural(dirs.len()), human(size)))
        );
        for dir in dirs {
            println!("  {:<width$}  {:>9}", dir.rel.display(), human(dir.size));
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

fn remove(path: &Path) -> io::Result<()> {
    // A symlink swapped in since the scan must not be followed.
    if !fs::symlink_metadata(path)?.is_dir() {
        return Err(io::Error::other("no longer a directory"));
    }
    fs::remove_dir_all(path)
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

    let (root, found) = if cli.demo {
        demo_data(&enabled)
    } else {
        let root = match cli.path.canonicalize() {
            Ok(root) if root.is_dir() => root,
            _ => {
                eprintln!("{} not a directory: {}", paint(RED, "error:"), cli.path.display());
                return ExitCode::FAILURE;
            }
        };
        let found = scan_with_spinner(&root, &enabled, cli.fast);
        (root, found)
    };

    let disk = if cli.demo { Some(Disk { free: 182 << 30, total: 1 << 40 }) } else { Disk::of(&root) };
    let dry = cli.dry_run || cli.demo;

    if found.is_empty() {
        println!("No cleanable directories found in {}", tilde(&root));
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
        match tui::run(&found, &tilde(&root), disk, dry) {
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

    let width = chosen.iter().map(|&i| found[i].rel.display().to_string().chars().count()).max().unwrap_or(0);
    let mut freed: u64 = 0;
    let mut deleted = 0;
    let mut failed = 0;
    for &i in &chosen {
        let dir = &found[i];
        match if dry { Ok(()) } else { remove(&dir.path) } {
            Ok(()) => {
                freed += dir.size;
                deleted += 1;
                println!("{} {:<width$}  {:>9}", paint(GREEN, "✓"), dir.rel.display(), human(dir.size));
            }
            Err(err) => {
                failed += 1;
                println!("{} {:<width$}  {}", paint(RED, "✗"), dir.rel.display(), paint(RED, err));
            }
        }
    }

    // What the filesystem reports is the truth; the per-directory sizes are
    // only what we expected to get back.
    let now_free = match (dry, disk) {
        (true, Some(before)) => Some(before.free + freed),
        (false, Some(_)) => Disk::of(&root).map(|after| after.free),
        _ => None,
    };
    if let (Some(before), Some(now)) = (disk, now_free)
        && now > before.free
    {
        freed = now - before.free;
    }

    println!();
    let headline = if dry { "Dry run, would have freed" } else { "Freed" };
    println!("{headline} {} in {deleted} {}", paint(GREEN, paint(BOLD, human(freed))), plural(deleted));
    if let (Some(before), Some(now)) = (disk, now_free) {
        print_disk(before, now);
    }
    if failed > 0 {
        println!("{}", paint(RED, format!("Failed to delete {failed} {}", plural(failed))));
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}
