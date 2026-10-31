//! Embedding throughput for a local model: `cargo run --release -p engram-embed --example speed -- bge-small 1 256 2000`
//! (model, instances, texts, characters per text).
use std::time::Instant;

use engram_embed::{Embedder, LocalEmbedder, LocalModel};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let model = LocalModel::parse(args.first().map_or("bge-small", String::as_str)).expect("model");
    let (instances, n, chars) = (args.get(1).map_or(1, |a| a.parse().unwrap()), args.get(2).map_or(256, |a| a.parse().unwrap()), args.get(3).map_or(2000, |a| a.parse().unwrap()));
    let dir = std::path::PathBuf::from(std::env::var("HOME").unwrap()).join("Library/Application Support/Engram/models");
    let t = Instant::now();
    let e = LocalEmbedder::load(model, dir, instances).unwrap();
    println!("load {:.1}s", t.elapsed().as_secs_f64());
    let texts: Vec<String> = (0..n).map(|i| format!("document {i} ").repeat(chars / 12)).collect();
    let t = Instant::now();
    let v = e.embed(&texts).unwrap();
    let s = t.elapsed().as_secs_f64();
    println!("{} texts x {} chars: {:.1}s, {:.0} texts/s, dim {}", n, chars, s, n as f64 / s, v[0].len());
}
