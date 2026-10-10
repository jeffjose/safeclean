use clap::ValueEnum;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, ValueEnum)]
pub enum ProjectType {
    Rust,
    #[value(alias = "js", alias = "npm")]
    Node,
    /// Framework build caches: .next, .nuxt, .svelte-kit, .turbo, ...
    Web,
    #[value(alias = "py")]
    Python,
    #[value(alias = "java")]
    Maven,
    Gradle,
    #[value(alias = "csharp")]
    Dotnet,
    Zig,
    Elixir,
    Swift,
    Haskell,
    Dart,
    Terraform,
    /// Everything in ~/.cache
    Cache,
    /// Package manager download caches: npm, cargo, gradle, maven, ...
    Packages,
    /// Your files in /tmp
    #[value(alias = "tmp")]
    Temp,
    Trash,
}

impl ProjectType {
    pub const ALL: &[ProjectType] = &[
        ProjectType::Rust,
        ProjectType::Node,
        ProjectType::Web,
        ProjectType::Python,
        ProjectType::Maven,
        ProjectType::Gradle,
        ProjectType::Dotnet,
        ProjectType::Zig,
        ProjectType::Elixir,
        ProjectType::Swift,
        ProjectType::Haskell,
        ProjectType::Dart,
        ProjectType::Terraform,
        ProjectType::Cache,
        ProjectType::Packages,
        ProjectType::Temp,
        ProjectType::Trash,
    ];

    pub fn name(self) -> &'static str {
        match self {
            ProjectType::Rust => "Rust",
            ProjectType::Node => "Node.js",
            ProjectType::Web => "Web caches",
            ProjectType::Python => "Python",
            ProjectType::Maven => "Java (Maven)",
            ProjectType::Gradle => "Gradle",
            ProjectType::Dotnet => ".NET",
            ProjectType::Zig => "Zig",
            ProjectType::Elixir => "Elixir",
            ProjectType::Swift => "Swift",
            ProjectType::Haskell => "Haskell",
            ProjectType::Dart => "Dart",
            ProjectType::Terraform => "Terraform",
            ProjectType::Cache => "User cache",
            ProjectType::Packages => "Package caches",
            ProjectType::Temp => "Temp files",
            ProjectType::Trash => "Trash",
        }
    }

    /// Kinds that come from well-known locations on the machine rather than
    /// from projects found by walking a directory.
    pub fn is_system(self) -> bool {
        matches!(self, ProjectType::Cache | ProjectType::Packages | ProjectType::Temp | ProjectType::Trash)
    }
}

/// What has to be true for a directory with a matching name to count.
enum Marker {
    /// The name alone is unambiguous.
    Always,
    /// One of these files sits next to the directory. A leading or trailing
    /// `*` matches any suffix/prefix.
    Sibling(&'static [&'static str]),
    /// The directory itself contains this file.
    Contains(&'static str),
}

struct Rule {
    dir: &'static str,
    kind: ProjectType,
    marker: Marker,
}

const fn rule(dir: &'static str, kind: ProjectType, marker: Marker) -> Rule {
    Rule { dir, kind, marker }
}

use Marker::{Always, Contains, Sibling};
use ProjectType::*;

// Order matters where a name is shared (target, build): first match wins.
const RULES: &[Rule] = &[
    rule("target", Rust, Sibling(&["Cargo.toml"])),
    rule("node_modules", Node, Always),
    rule(".next", Web, Sibling(&["next.config*", "package.json"])),
    rule(".nuxt", Web, Sibling(&["nuxt.config*", "package.json"])),
    rule(".svelte-kit", Web, Sibling(&["svelte.config*", "package.json"])),
    rule(".turbo", Web, Sibling(&["package.json"])),
    rule(".parcel-cache", Web, Always),
    rule(".angular", Web, Sibling(&["angular.json"])),
    rule(".venv", Python, Contains("pyvenv.cfg")),
    rule("venv", Python, Contains("pyvenv.cfg")),
    rule("__pycache__", Python, Always),
    rule(".pytest_cache", Python, Always),
    rule(".mypy_cache", Python, Always),
    rule(".ruff_cache", Python, Always),
    rule(".tox", Python, Always),
    rule("target", Maven, Sibling(&["pom.xml"])),
    rule("build", Gradle, Sibling(&["build.gradle", "build.gradle.kts"])),
    rule(
        ".gradle",
        Gradle,
        Sibling(&["build.gradle", "build.gradle.kts", "settings.gradle", "settings.gradle.kts"]),
    ),
    rule("bin", Dotnet, Sibling(&["*.csproj", "*.fsproj", "*.vbproj"])),
    rule("obj", Dotnet, Sibling(&["*.csproj", "*.fsproj", "*.vbproj"])),
    rule(".zig-cache", Zig, Sibling(&["build.zig"])),
    rule("zig-cache", Zig, Sibling(&["build.zig"])),
    rule("zig-out", Zig, Sibling(&["build.zig"])),
    rule("_build", Elixir, Sibling(&["mix.exs"])),
    rule("deps", Elixir, Sibling(&["mix.exs"])),
    rule(".build", Swift, Sibling(&["Package.swift"])),
    rule("dist-newstyle", Haskell, Always),
    rule(".stack-work", Haskell, Always),
    rule(".dart_tool", Dart, Sibling(&["pubspec.yaml"])),
    rule("build", Dart, Sibling(&["pubspec.yaml"])),
    rule(".terraform", Terraform, Always),
];

fn has_sibling(dir: &Path, patterns: &[&str]) -> bool {
    let Some(parent) = dir.parent() else {
        return false;
    };
    let (globs, exact): (Vec<&str>, Vec<&str>) = patterns.iter().partition(|p| p.contains('*'));
    if exact.iter().any(|name| parent.join(name).exists()) {
        return true;
    }
    if globs.is_empty() {
        return false;
    }
    let Ok(entries) = std::fs::read_dir(parent) else {
        return false;
    };
    entries.flatten().any(|entry| {
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            return false;
        };
        globs.iter().any(|glob| match glob.strip_prefix('*') {
            Some(suffix) => name.ends_with(suffix),
            None => name.starts_with(glob.trim_end_matches('*')),
        })
    })
}

/// Returns the project type `path` is a cleanable artifact of, if any.
pub fn classify(path: &Path, name: &str) -> Option<ProjectType> {
    RULES
        .iter()
        .filter(|rule| rule.dir == name)
        .find(|rule| match rule.marker {
            Always => true,
            Sibling(patterns) => has_sibling(path, patterns),
            Contains(file) => path.join(file).exists(),
        })
        .map(|rule| rule.kind)
}
