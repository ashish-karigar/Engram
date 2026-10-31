//! Areas: the deployable or buildable parts of a repository (a frontend app, a backend service, a
//! crate, an iOS app), found from their manifests. Every file belongs to its deepest area, which
//! is how the system map groups a thousand files into a handful of parts a person (or an agent)
//! can reason about.

#[derive(Debug, Clone, PartialEq)]
pub struct Area {
    /// Folder relative to the repository root; "" for the root.
    pub path: String,
    /// Display name: the folder name, or the repository name for the root.
    pub name: String,
    /// What it is, from its manifest: "Angular app", "FastAPI service", "Rust crate", ...
    pub kind: String,
    pub manifest: String,
}

/// Dependency names that say what kind of part a manifest describes, most specific first.
const JS_KINDS: &[(&str, &str)] = &[
    ("@angular/core", "Angular app"),
    ("next", "Next.js app"),
    ("@tauri-apps/api", "Tauri UI"),
    ("react", "React app"),
    ("vue", "Vue app"),
    ("svelte", "Svelte app"),
    ("express", "Express server"),
    ("fastify", "Fastify server"),
    ("hono", "Hono server"),
];
const PY_KINDS: &[(&str, &str)] = &[("fastapi", "FastAPI service"), ("flask", "Flask service"), ("django", "Django service"), ("streamlit", "Streamlit app")];
const RS_KINDS: &[(&str, &str)] = &[("tauri", "Tauri app"), ("axum", "Rust server"), ("actix-web", "Rust server"), ("rocket", "Rust server")];

fn kind_from(text: &str, table: &[(&str, &str)], fallback: &str) -> String {
    let lower = text.to_lowercase();
    table
        .iter()
        .find(|(dep, _)| lower.contains(&format!("\"{dep}\"")) || lower.contains(&format!("{dep} ")) || lower.contains(&format!("{dep}=")) || lower.contains(&format!("{dep}>")) || lower.contains(&format!("\n{dep}\n")) || lower.contains(&format!("'{dep}")) || lower.contains(&format!("\"{dep}[")))
        .map_or(fallback.to_string(), |(_, k)| k.to_string())
}

/// Find areas from the repository's files. `read` returns a manifest's text.
pub fn detect(files: &[String], repo_name: &str, read: impl Fn(&str) -> Option<String>) -> Vec<Area> {
    let mut areas: Vec<Area> = vec![];
    for f in files {
        let (dir, name) = f.rsplit_once('/').map_or(("", f.as_str()), |(d, n)| (d, n));
        let kind = match name {
            // A workspace root lists packages; it is not one itself (like a Cargo workspace).
            "package.json" => read(f).filter(|t| !t.contains("\"workspaces\"")).map(|t| kind_from(&t, JS_KINDS, "JavaScript package")),
            "pyproject.toml" | "setup.py" | "requirements.txt" => read(f).map(|t| kind_from(&t, PY_KINDS, "Python package")),
            // A workspace-only Cargo.toml (no [package]) is a container, not a part.
            "Cargo.toml" => read(f).filter(|t| t.contains("[package]")).map(|t| kind_from(&t, RS_KINDS, "Rust crate")),
            "go.mod" => Some("Go module".into()),
            "Package.swift" => Some("Swift package".into()),
            "pom.xml" | "build.gradle" | "build.gradle.kts" => Some("JVM project".into()),
            _ if f.contains(".xcodeproj/") && name == "project.pbxproj" => {
                // The app is the folder holding `X.xcodeproj`.
                let proj_dir = f.split(".xcodeproj/").next().unwrap_or("");
                let app_dir = proj_dir.rsplit_once('/').map_or("", |(d, _)| d).to_string();
                if !areas.iter().any(|a| a.path == app_dir) {
                    areas.push(Area { name: display(&app_dir, repo_name), path: app_dir, kind: "iOS / macOS app".into(), manifest: f.clone() });
                }
                None
            }
            _ => None,
        };
        let Some(kind) = kind else { continue };
        match areas.iter_mut().find(|a| a.path == dir) {
            // Several manifests in one folder (pyproject + requirements): keep the more specific kind.
            Some(existing) if existing.kind.ends_with("package") => existing.kind = kind,
            Some(_) => {}
            None => areas.push(Area { path: dir.to_string(), name: display(dir, repo_name), kind, manifest: f.clone() }),
        }
    }
    areas.sort_by(|a, b| a.path.cmp(&b.path));
    areas
}

fn display(dir: &str, repo_name: &str) -> String {
    if dir.is_empty() { repo_name.to_string() } else { dir.rsplit('/').next().unwrap_or(dir).to_string() }
}

/// The deepest area containing `file`; None when the repository has no manifests at all.
pub fn area_of<'a>(areas: &'a [Area], file: &str) -> Option<&'a Area> {
    areas
        .iter()
        .filter(|a| a.path.is_empty() || file == a.path || file.starts_with(&format!("{}/", a.path)))
        .max_by_key(|a| a.path.len())
}
