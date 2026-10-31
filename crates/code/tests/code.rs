use std::collections::HashSet;

use engram_code::{Import, Lang, Resolver, parse_file};

fn names(src: &str, lang: Lang) -> Vec<String> {
    parse_file(lang, src).unwrap().symbols.iter().map(|s| format!("{}:{}{}", s.kind, s.name, if s.top_level { "" } else { "*" })).collect()
}

#[test]
fn languages_are_detected_by_extension() {
    assert_eq!(Lang::from_path("a/b.rs"), Some(Lang::Rust));
    assert_eq!(Lang::from_path("ui/App.tsx"), Some(Lang::Tsx));
    assert_eq!(Lang::from_path("x.mjs"), Some(Lang::Tsx));
    assert_eq!(Lang::from_path("api.ts"), Some(Lang::TypeScript));
    assert_eq!(Lang::from_path("README.md"), None);
}

#[test]
fn rust_symbols_and_uses() {
    let src = r#"
use std::path::Path;
use crate::db::{new_id, now};
use engram_core::memory::{Memory, search_memories as search};
use super::graph::*;
mod helpers;
pub struct Store { conn: u8 }
pub enum Mode { A }
pub trait Embedder { fn embed(&self); }
impl Store {
    pub fn open() -> Self { todo!() }
    fn close(&self) {}
}
pub fn run() {}
mod inline { pub fn nested() {} }
"#;
    assert_eq!(
        names(src, Lang::Rust),
        ["struct:Store", "enum:Mode", "trait:Embedder", "method:Embedder::embed*", "impl:impl Store", "method:Store::open*", "method:Store::close*", "function:run", "module:inline", "function:nested*"]
    );
    let imports = parse_file(Lang::Rust, src).unwrap().imports;
    let paths: Vec<String> = imports.iter().map(|i| match i {
        Import::Rust { path, is_mod } => format!("{}{}", if *is_mod { "mod " } else { "" }, path.join("::")),
        _ => unreachable!(),
    }).collect();
    assert_eq!(paths, ["std::path::Path", "crate::db::new_id", "crate::db::now", "engram_core::memory::Memory", "engram_core::memory::search_memories", "super::graph", "mod helpers"]);
}

#[test]
fn typescript_symbols_and_imports() {
    let src = r#"
import { useState } from "react";
import { projectApi } from "./api/client.ts";
import type { Graph } from "../types";
export { helper } from "./helpers";
export interface Props { a: string }
export type Mode = "a" | "b";
export enum Kind { A }
export function App() { return null; }
export const Panel = ({ x }: Props) => null;
const lazy = () => import("./lazy");
class Store { load() {} save() {} }
"#;
    assert_eq!(
        names(src, Lang::Tsx),
        ["interface:Props", "type:Mode", "enum:Kind", "function:App", "function:Panel", "function:lazy", "class:Store", "method:Store.load*", "method:Store.save*"]
    );
    let specs: Vec<String> = parse_file(Lang::Tsx, src).unwrap().imports.iter().map(|i| match i { Import::Js { specifier } => specifier.clone(), _ => unreachable!() }).collect();
    assert_eq!(specs, ["react", "./api/client.ts", "../types", "./helpers", "./lazy"]);
}

#[test]
fn python_keeps_decorators_in_the_symbol_span() {
    let idx = parse_file(Lang::Python, "import os\n\n@dataclass\nclass A:\n    def f(self): pass\n").unwrap();
    assert_eq!((idx.symbols[0].name.as_str(), idx.symbols[0].start_line, idx.symbols[0].end_line), ("A", 3, 5));
    assert_eq!(idx.symbols[1].name, "A.f");
}

#[test]
fn imports_resolve_to_project_files() {
    let files: HashSet<String> = [
        "crates/core/src/lib.rs", "crates/core/src/db.rs", "crates/core/src/memory.rs",
        "crates/serve/src/lib.rs", "crates/serve/src/ops.rs", "crates/serve/src/graph/mod.rs", "crates/serve/src/graph/draw.rs",
        "ui/src/App.tsx", "ui/src/api/client.ts", "ui/src/types.ts", "ui/src/components/index.ts",
        "perception/lidar.py", "perception/fusion.py",
    ].map(String::from).into();
    let manifests = vec![
        ("crates/core/Cargo.toml".to_string(), "[package]\nname = \"engram-core\"\nversion = \"0.1.0\"\n\n[dependencies]\nname-like = \"1\"\n".to_string()),
        ("crates/serve/Cargo.toml".to_string(), "[package]\nname = \"engram-serve\"\n".to_string()),
    ];
    let r = Resolver::new(&files, &manifests);
    let rust = |from: &str, path: &str| r.resolve(from, &Import::Rust { path: path.split("::").map(String::from).collect(), is_mod: false });
    assert_eq!(rust("crates/serve/src/ops.rs", "engram_core::memory::Memory").as_deref(), Some("crates/core/src/memory.rs"));
    assert_eq!(rust("crates/serve/src/ops.rs", "engram_core::Connection").as_deref(), Some("crates/core/src/lib.rs"), "re-exported at the root");
    assert_eq!(rust("crates/core/src/memory.rs", "crate::db::now").as_deref(), Some("crates/core/src/db.rs"));
    assert_eq!(rust("crates/serve/src/graph/mod.rs", "super::ops::x").as_deref(), Some("crates/serve/src/ops.rs"));
    assert_eq!(rust("crates/serve/src/graph/draw.rs", "super::super::ops::x").as_deref(), Some("crates/serve/src/ops.rs"));
    assert_eq!(rust("crates/serve/src/graph/draw.rs", "super::ops::x"), None, "graph::ops does not exist");
    assert_eq!(rust("crates/serve/src/ops.rs", "serde_json::json"), None, "external crates are skipped");
    assert_eq!(r.resolve("crates/serve/src/lib.rs", &Import::Rust { path: vec!["graph".into()], is_mod: true }).as_deref(), Some("crates/serve/src/graph/mod.rs"));

    let js = |from: &str, spec: &str| r.resolve(from, &Import::Js { specifier: spec.into() });
    assert_eq!(js("ui/src/App.tsx", "./api/client.ts").as_deref(), Some("ui/src/api/client.ts"));
    assert_eq!(js("ui/src/api/client.ts", "../types").as_deref(), Some("ui/src/types.ts"));
    assert_eq!(js("ui/src/App.tsx", "./components").as_deref(), Some("ui/src/components/index.ts"));
    assert_eq!(js("ui/src/App.tsx", "react"), None);

    assert_eq!(r.resolve("perception/fusion.py", &Import::Python { module: "lidar".into(), level: 1 }).as_deref(), Some("perception/lidar.py"));
}

#[test]
fn python_absolute_imports_resolve_in_src_and_subproject_layouts() {
    let files: HashSet<String> = [
        "src/parallax/__init__.py", "src/parallax/engine.py", "src/parallax/io/frames.py", "tests/test_engine.py",
        "backend/app/main.py", "backend/app/models.py", "backend/app/routers/jobs.py", "backend/tests/test_jobs.py",
        "worker/app/models.py",
    ].map(String::from).into();
    let r = Resolver::new(&files, &[]);
    let py = |from: &str, module: &str| r.resolve(from, &Import::Python { module: module.into(), level: 0 });
    assert_eq!(py("tests/test_engine.py", "parallax.engine").as_deref(), Some("src/parallax/engine.py"));
    assert_eq!(py("src/parallax/engine.py", "parallax.io.frames").as_deref(), Some("src/parallax/io/frames.py"));
    assert_eq!(py("tests/test_engine.py", "parallax").as_deref(), Some("src/parallax/__init__.py"));
    assert_eq!(py("backend/app/routers/jobs.py", "app.models").as_deref(), Some("backend/app/models.py"), "same sub-project wins");
    assert_eq!(py("worker/app/main.py", "app.models").as_deref(), Some("worker/app/models.py"));
    assert_eq!(py("backend/app/main.py", "numpy"), None);
}
