use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProjectType {
    Rust,
    Node,
    Python,
    JavaMaven,
    Gradle,
    DotNet,
    NextJs,
    NuxtJs,
}

impl ProjectType {
    pub fn name(&self) -> &'static str {
        match self {
            ProjectType::Rust => "Rust",
            ProjectType::Node => "Node.js",
            ProjectType::Python => "Python",
            ProjectType::JavaMaven => "Java (Maven)",
            ProjectType::Gradle => "Gradle",
            ProjectType::DotNet => ".NET",
            ProjectType::NextJs => "Next.js",
            ProjectType::NuxtJs => "Nuxt.js",
        }
    }

    pub fn all() -> Vec<ProjectType> {
        vec![
            ProjectType::Rust,
            ProjectType::Node,
            ProjectType::Python,
            ProjectType::JavaMaven,
            ProjectType::Gradle,
            ProjectType::DotNet,
            ProjectType::NextJs,
            ProjectType::NuxtJs,
        ]
    }
}

pub struct CleanableDir {
    pub dir_name: &'static str,
    pub project_type: ProjectType,
    pub validator: fn(&Path) -> bool,
}

fn has_sibling(path: &Path, filename: &str) -> bool {
    path.parent()
        .map(|p| p.join(filename).exists())
        .unwrap_or(false)
}

fn has_sibling_matching(path: &Path, pattern: &str) -> bool {
    let Some(parent) = path.parent() else {
        return false;
    };
    let Ok(entries) = std::fs::read_dir(parent) else {
        return false;
    };
    entries.filter_map(|e| e.ok()).any(|entry| {
        entry
            .file_name()
            .to_str()
            .map(|name| name.starts_with(pattern))
            .unwrap_or(false)
    })
}

fn always_valid(_: &Path) -> bool {
    true
}

fn validate_rust(path: &Path) -> bool {
    has_sibling(path, "Cargo.toml")
}

fn validate_node(path: &Path) -> bool {
    has_sibling(path, "package.json")
}

fn validate_maven(path: &Path) -> bool {
    has_sibling(path, "pom.xml")
}

fn validate_gradle(path: &Path) -> bool {
    has_sibling(path, "build.gradle") || has_sibling(path, "build.gradle.kts")
}

fn validate_dotnet(path: &Path) -> bool {
    has_sibling_matching(path, ".csproj")
        || has_sibling_matching(path, ".fsproj")
        || has_sibling_matching(path, ".sln")
}

fn validate_nextjs(path: &Path) -> bool {
    has_sibling_matching(path, "next.config")
}

fn validate_nuxtjs(path: &Path) -> bool {
    has_sibling_matching(path, "nuxt.config")
}

pub fn get_cleanable_dirs() -> Vec<CleanableDir> {
    vec![
        // Rust
        CleanableDir {
            dir_name: "target",
            project_type: ProjectType::Rust,
            validator: validate_rust,
        },
        // Node.js
        CleanableDir {
            dir_name: "node_modules",
            project_type: ProjectType::Node,
            validator: validate_node,
        },
        // Python
        CleanableDir {
            dir_name: ".venv",
            project_type: ProjectType::Python,
            validator: always_valid,
        },
        CleanableDir {
            dir_name: "venv",
            project_type: ProjectType::Python,
            validator: always_valid,
        },
        CleanableDir {
            dir_name: "__pycache__",
            project_type: ProjectType::Python,
            validator: always_valid,
        },
        CleanableDir {
            dir_name: ".pytest_cache",
            project_type: ProjectType::Python,
            validator: always_valid,
        },
        CleanableDir {
            dir_name: ".mypy_cache",
            project_type: ProjectType::Python,
            validator: always_valid,
        },
        CleanableDir {
            dir_name: ".ruff_cache",
            project_type: ProjectType::Python,
            validator: always_valid,
        },
        CleanableDir {
            dir_name: ".tox",
            project_type: ProjectType::Python,
            validator: always_valid,
        },
        // Java (Maven)
        CleanableDir {
            dir_name: "target",
            project_type: ProjectType::JavaMaven,
            validator: validate_maven,
        },
        // Gradle
        CleanableDir {
            dir_name: "build",
            project_type: ProjectType::Gradle,
            validator: validate_gradle,
        },
        CleanableDir {
            dir_name: ".gradle",
            project_type: ProjectType::Gradle,
            validator: validate_gradle,
        },
        // .NET
        CleanableDir {
            dir_name: "bin",
            project_type: ProjectType::DotNet,
            validator: validate_dotnet,
        },
        CleanableDir {
            dir_name: "obj",
            project_type: ProjectType::DotNet,
            validator: validate_dotnet,
        },
        // Next.js
        CleanableDir {
            dir_name: ".next",
            project_type: ProjectType::NextJs,
            validator: validate_nextjs,
        },
        // Nuxt.js
        CleanableDir {
            dir_name: ".nuxt",
            project_type: ProjectType::NuxtJs,
            validator: validate_nuxtjs,
        },
    ]
}
