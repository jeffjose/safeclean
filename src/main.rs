mod cleaner;
mod projects;
mod scanner;

use clap::Parser;
use colored::Colorize;
use dialoguer::MultiSelect;
use projects::ProjectType;
use scanner::FoundDir;
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

fn build_display_item(dir: &FoundDir, max_path_len: usize) -> String {
    let path_str = dir.path.display().to_string();
    let size_str = dir.size_human();
    let type_str = format!("[{}]", dir.project_type.name());
    format!(
        "{:<width$}  {:>10}  {}",
        path_str,
        size_str,
        type_str,
        width = max_path_len
    )
}

fn main() {
    let cli = Cli::parse();

    let path = cli.path.canonicalize().unwrap_or_else(|_| {
        eprintln!("{} Invalid path: {}", "error:".red().bold(), cli.path.display());
        std::process::exit(1);
    });

    println!(
        "{} {}...\n",
        "Scanning".cyan().bold(),
        path.display()
    );

    let enabled_types = get_enabled_types(&cli);
    let found = scanner::scan(&path, &enabled_types);

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
        for dir in &found {
            println!(
                "  {}  {:>10}  {}",
                dir.path.display(),
                dir.size_human(),
                format!("[{}]", dir.project_type.name()).dimmed()
            );
        }
        println!(
            "\n{} {}",
            "Total:".bold(),
            format_size(total_size).green().bold()
        );
        return;
    }

    // Build items for multi-select
    let max_path_len = found
        .iter()
        .map(|d| d.path.display().to_string().len())
        .max()
        .unwrap_or(50);

    let items: Vec<String> = found.iter().map(|d| build_display_item(d, max_path_len)).collect();

    // All selected by default
    let defaults: Vec<bool> = vec![true; found.len()];

    let selected_indices = if cli.yes {
        // Skip confirmation, select all
        (0..found.len()).collect()
    } else {
        println!("Use {} to toggle, {} to confirm:\n", "Space".cyan(), "Enter".cyan());

        match MultiSelect::new()
            .items(&items)
            .defaults(&defaults)
            .interact()
        {
            Ok(indices) => indices,
            Err(_) => {
                println!("\n{}", "Cancelled.".yellow());
                return;
            }
        }
    };

    if selected_indices.is_empty() {
        println!("\n{}", "Nothing selected.".yellow());
        return;
    }

    let to_delete: Vec<FoundDir> = selected_indices
        .iter()
        .map(|&i| found[i].clone())
        .collect();

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
