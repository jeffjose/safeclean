mod cleaner;
mod projects;
mod scanner;
mod selector;

use clap::Parser;
use colored::Colorize;
use indicatif::{ProgressBar, ProgressStyle};
use projects::ProjectType;
use scanner::FoundDir;
use selector::GroupedSelector;
use std::collections::HashSet;
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "safeclean")]
#[command(about = "Safely clean up build artifacts and dependency caches to reclaim disk space")]
#[command(version)]
struct Cli {
    /// Directory to scan (defaults to current directory)
    #[arg(default_value = ".")]
    path: PathBuf,

    /// Show what would be deleted without deleting
    #[arg(short = 'n', long)]
    dry_run: bool,

    /// Clean Rust target/ directories
    #[arg(long)]
    rust: bool,

    /// Clean Node.js node_modules/
    #[arg(long, visible_alias = "js", visible_alias = "npm")]
    node: bool,

    /// Clean Python venvs and caches
    #[arg(long, visible_alias = "py")]
    python: bool,

    /// Clean Java Maven target/
    #[arg(long, visible_alias = "maven")]
    java: bool,

    /// Clean Gradle build/ directories
    #[arg(long)]
    gradle: bool,

    /// Clean .NET bin/ and obj/ directories
    #[arg(long, visible_alias = "csharp")]
    dotnet: bool,

    /// Clean Next.js .next/ directories
    #[arg(long)]
    next: bool,

    /// Clean Nuxt.js .nuxt/ directories
    #[arg(long)]
    nuxt: bool,

    /// Skip confirmation prompt
    #[arg(short = 'y', long)]
    yes: bool,

    /// Demo mode - show UI with simulated data (nothing is deleted)
    #[arg(long)]
    demo: bool,
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

fn get_enabled_types(cli: &Cli) -> HashSet<ProjectType> {
    let any_specified =
        cli.rust || cli.node || cli.python || cli.java || cli.gradle || cli.dotnet || cli.next || cli.nuxt;

    if !any_specified {
        return ProjectType::all().into_iter().collect();
    }

    let mut types = HashSet::new();
    if cli.rust {
        types.insert(ProjectType::Rust);
    }
    if cli.node {
        types.insert(ProjectType::Node);
    }
    if cli.python {
        types.insert(ProjectType::Python);
    }
    if cli.java {
        types.insert(ProjectType::JavaMaven);
    }
    if cli.gradle {
        types.insert(ProjectType::Gradle);
    }
    if cli.dotnet {
        types.insert(ProjectType::DotNet);
    }
    if cli.next {
        types.insert(ProjectType::NextJs);
    }
    if cli.nuxt {
        types.insert(ProjectType::NuxtJs);
    }
    types
}

fn generate_demo_data() -> Vec<FoundDir> {
    vec![
        // Rust projects
        FoundDir {
            path: "/home/user/projects/api-server/target".into(),
            project_type: ProjectType::Rust,
            size_bytes: 1_892_000_000, // 1.9 GB
        },
        FoundDir {
            path: "/home/user/projects/cli-tool/target".into(),
            project_type: ProjectType::Rust,
            size_bytes: 456_000_000, // 456 MB
        },
        FoundDir {
            path: "/home/user/projects/utils/target".into(),
            project_type: ProjectType::Rust,
            size_bytes: 234_000_000, // 234 MB
        },
        // Node.js projects
        FoundDir {
            path: "/home/user/projects/webapp/node_modules".into(),
            project_type: ProjectType::Node,
            size_bytes: 892_000_000, // 892 MB
        },
        FoundDir {
            path: "/home/user/projects/dashboard/node_modules".into(),
            project_type: ProjectType::Node,
            size_bytes: 654_000_000, // 654 MB
        },
        FoundDir {
            path: "/home/user/projects/blog/node_modules".into(),
            project_type: ProjectType::Node,
            size_bytes: 423_000_000, // 423 MB
        },
        FoundDir {
            path: "/home/user/projects/portfolio/node_modules".into(),
            project_type: ProjectType::Node,
            size_bytes: 312_000_000, // 312 MB
        },
        // Python projects
        FoundDir {
            path: "/home/user/projects/ml-pipeline/.venv".into(),
            project_type: ProjectType::Python,
            size_bytes: 1_234_000_000, // 1.2 GB
        },
        FoundDir {
            path: "/home/user/projects/data-analysis/.venv".into(),
            project_type: ProjectType::Python,
            size_bytes: 567_000_000, // 567 MB
        },
        FoundDir {
            path: "/home/user/projects/scripts/__pycache__".into(),
            project_type: ProjectType::Python,
            size_bytes: 12_000_000, // 12 MB
        },
        // Next.js
        FoundDir {
            path: "/home/user/projects/webapp/.next".into(),
            project_type: ProjectType::NextJs,
            size_bytes: 345_000_000, // 345 MB
        },
        // Gradle
        FoundDir {
            path: "/home/user/projects/android-app/build".into(),
            project_type: ProjectType::Gradle,
            size_bytes: 789_000_000, // 789 MB
        },
        FoundDir {
            path: "/home/user/projects/android-app/.gradle".into(),
            project_type: ProjectType::Gradle,
            size_bytes: 234_000_000, // 234 MB
        },
    ]
}

fn group_by_type(dirs: &[FoundDir]) -> Vec<(ProjectType, Vec<&FoundDir>)> {
    let mut grouped: std::collections::HashMap<ProjectType, Vec<&FoundDir>> =
        std::collections::HashMap::new();

    for dir in dirs {
        grouped.entry(dir.project_type).or_default().push(dir);
    }

    let type_order = ProjectType::all();
    let mut result: Vec<(ProjectType, Vec<&FoundDir>)> = Vec::new();

    for pt in type_order {
        if let Some(dirs) = grouped.remove(&pt) {
            result.push((pt, dirs));
        }
    }

    result
}

fn main() {
    let cli = Cli::parse();

    let found = if cli.demo {
        println!(
            "{} {}\n",
            "Demo mode".yellow().bold(),
            "(simulated data - nothing will be deleted)".dimmed()
        );
        generate_demo_data()
    } else {
        let path = cli.path.canonicalize().unwrap_or_else(|_| {
            eprintln!("{} Invalid path: {}", "error:".red().bold(), cli.path.display());
            std::process::exit(1);
        });

        let spinner = ProgressBar::new_spinner();
        spinner.set_style(
            ProgressStyle::default_spinner()
                .template("{spinner:.cyan} {msg}")
                .unwrap()
                .tick_chars("⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏"),
        );
        spinner.set_message(format!("Searching for build artifacts in {}", path.display()));
        spinner.enable_steady_tick(std::time::Duration::from_millis(80));

        let enabled_types = get_enabled_types(&cli);
        let result = scanner::scan(&path, &enabled_types);

        spinner.finish_and_clear();
        result
    };

    if found.is_empty() {
        println!("{}", "No cleanable directories found.".yellow());
        return;
    }

    let total_size = scanner::total_size(&found);
    println!(
        "Found {} cleanable directories ({})\n",
        found.len().to_string().green().bold(),
        format_size(total_size).green().bold()
    );

    if cli.dry_run {
        println!("{}", "Dry run - nothing will be deleted:\n".yellow());
        let grouped = group_by_type(&found);
        for (project_type, dirs) in &grouped {
            let group_size: u64 = dirs.iter().map(|d| d.size_bytes).sum();
            println!(
                "{} {} ({} items, {})",
                "▼".dimmed(),
                project_type.name().bold(),
                dirs.len(),
                format_size(group_size).green()
            );
            for dir in dirs {
                println!(
                    "    {}  {:>10}",
                    dir.path.display(),
                    dir.size_human()
                );
            }
            println!();
        }
        println!(
            "{} {}",
            "Total:".bold(),
            format_size(total_size).green().bold()
        );
        return;
    }

    let to_delete = if cli.yes {
        found
    } else {
        let selector = GroupedSelector::new(found);
        match selector.run() {
            Ok(selected) => selected,
            Err(_) => {
                println!("{}", "Cancelled.".yellow());
                return;
            }
        }
    };

    if to_delete.is_empty() {
        println!("{}", "Nothing selected.".yellow());
        return;
    }

    if cli.demo {
        // Demo mode - just show what would be deleted
        let total: u64 = to_delete.iter().map(|d| d.size_bytes).sum();
        println!(
            "\n{} Would delete {} directories ({}):",
            "Demo:".yellow().bold(),
            to_delete.len().to_string().green(),
            format_size(total).green().bold()
        );
        let grouped = group_by_type(&to_delete);
        for (project_type, dirs) in &grouped {
            let group_size: u64 = dirs.iter().map(|d| d.size_bytes).sum();
            println!(
                "\n  {} ({} items, {})",
                project_type.name().bold(),
                dirs.len(),
                format_size(group_size).green()
            );
            for dir in dirs {
                println!("    {} {:>10}", dir.path.display(), dir.size_human());
            }
        }
        println!(
            "\n{} {}",
            "Nothing was deleted - this is a demo.".yellow(),
            "Run without --demo to actually clean.".dimmed()
        );
        return;
    }

    println!("\n{} {} directories...", "Deleting".red().bold(), to_delete.len());

    let result = cleaner::clean(to_delete);

    if !result.failed.is_empty() {
        println!("\n{}", "Failed to delete:".red());
        for (dir, err) in &result.failed {
            println!("  {} - {}", dir.path.display(), err);
        }
    }

    if !result.deleted.is_empty() {
        println!(
            "\n{} Cleaned {} in {} directories",
            "Done!".green().bold(),
            format_size(result.total_cleaned()).green().bold(),
            result.deleted.len().to_string().green()
        );
    }
}
