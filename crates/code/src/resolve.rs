use std::collections::{HashMap, HashSet};

use crate::lang::Import;

/// Maps imports to files that exist in the project. External packages resolve to None.
pub struct Resolver<'a> {
    files: &'a HashSet<String>,
    /// Rust crate identifier (`engram_core`) → its `src` directory (`crates/core/src`).
    crates: HashMap<String, String>,
}

fn dirname(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(d, _)| d)
}

/// Join and normalise `a/b` + `../c` → `a/c`.
fn join(base: &str, rel: &str) -> String {
    let mut parts: Vec<&str> = base.split('/').filter(|p| !p.is_empty()).collect();
    for seg in rel.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            s => parts.push(s),
        }
    }
    parts.join("/")
}

/// The `name = "..."` of a Cargo.toml's `[package]` section.
fn package_name(manifest: &str) -> Option<String> {
    let mut in_package = false;
    for line in manifest.lines().map(str::trim) {
        if line.starts_with('[') {
            in_package = line == "[package]";
        } else if in_package {
            if let Some(v) = line.strip_prefix("name").map(str::trim_start).and_then(|r| r.strip_prefix('=')) {
                return Some(v.trim().trim_matches('"').to_string());
            }
        }
    }
    None
}

impl<'a> Resolver<'a> {
    /// `manifests` are (path of Cargo.toml, its contents), used to resolve workspace crates.
    pub fn new(files: &'a HashSet<String>, manifests: &[(String, String)]) -> Self {
        let crates = manifests
            .iter()
            .filter_map(|(path, body)| {
                let name = package_name(body)?.replace('-', "_");
                let dir = dirname(path);
                Some((name, if dir.is_empty() { "src".to_string() } else { format!("{dir}/src") }))
            })
            .collect();
        Resolver { files, crates }
    }

    fn first(&self, candidates: impl IntoIterator<Item = String>, from: &str) -> Option<String> {
        candidates.into_iter().find(|c| c != from && self.files.contains(c))
    }

    pub fn resolve(&self, from: &str, import: &Import) -> Option<String> {
        match import {
            Import::Python { module, level } => self.python(from, module, *level),
            Import::Js { specifier } => self.js(from, specifier),
            Import::Rust { path, is_mod } => self.rust(from, path, *is_mod),
        }
    }

    fn python(&self, from: &str, module: &str, level: usize) -> Option<String> {
        let mut parts: Vec<&str> = if level > 0 {
            let dir: Vec<&str> = from.split('/').collect();
            dir[..dir.len().saturating_sub(level)].to_vec()
        } else {
            vec![]
        };
        parts.extend(module.split('.').filter(|p| !p.is_empty()));
        let stem = parts.join("/");
        self.first([format!("{stem}.py"), format!("{stem}/__init__.py")], from)
    }

    fn js(&self, from: &str, spec: &str) -> Option<String> {
        if !spec.starts_with('.') {
            return None;
        }
        let base = join(dirname(from), spec);
        let stem = base.trim_end_matches(".js").trim_end_matches(".jsx").to_string();
        let exts = ["ts", "tsx", "js", "jsx", "mjs", "cjs", "d.ts"];
        let mut candidates = vec![base.clone()];
        candidates.extend(exts.iter().map(|e| format!("{stem}.{e}")));
        candidates.extend(["index.ts", "index.tsx", "index.js", "index.jsx"].iter().map(|i| format!("{base}/{i}")));
        self.first(candidates, from)
    }

    /// The directory holding a Rust file's child modules: `src/a.rs` → `src/a`, `src/a/mod.rs` → `src/a`.
    fn module_dir(from: &str) -> String {
        let file = from.rsplit('/').next().unwrap_or(from);
        if matches!(file, "mod.rs" | "lib.rs" | "main.rs") {
            dirname(from).to_string()
        } else {
            from.trim_end_matches(".rs").to_string()
        }
    }

    fn crate_src(&self, from: &str) -> Option<&String> {
        self.crates.values().filter(|src| from.starts_with(&format!("{src}/"))).max_by_key(|s| s.len())
    }

    fn rust(&self, from: &str, path: &[String], is_mod: bool) -> Option<String> {
        if is_mod {
            let dir = Self::module_dir(from);
            let name = path.first()?;
            return self.first([format!("{dir}/{name}.rs"), format!("{dir}/{name}/mod.rs")], from);
        }
        let (base, rest): (String, &[String]) = match path.first()?.as_str() {
            "crate" => (self.crate_src(from)?.clone(), &path[1..]),
            "self" => (Self::module_dir(from), &path[1..]),
            "super" => {
                // Each `super` climbs one module: `super::super::x` from `a/b/c.rs` looks in `a`.
                let ups = path.iter().take_while(|s| *s == "super").count();
                let mut dir = Self::module_dir(from);
                for _ in 0..ups {
                    dir = dirname(&dir).to_string();
                }
                (dir, &path[ups..])
            }
            first => (self.crates.get(first)?.clone(), &path[1..]),
        };
        // The longest prefix of the path that names a module file is the target (the rest are
        // items). Items re-exported at the crate root resolve to lib.rs / main.rs.
        (1..=rest.len())
            .rev()
            .find_map(|k| {
                let stem = format!("{base}/{}", rest[..k].join("/"));
                self.first([format!("{stem}.rs"), format!("{stem}/mod.rs")], from)
            })
            .or_else(|| self.first([format!("{base}/lib.rs"), format!("{base}/main.rs")], from))
    }
}
