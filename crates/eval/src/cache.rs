//! An on-disk embedding cache for evaluation runs, so repeated runs (tuning on dev) do not re-embed
//! the same ~30k texts. Keyed by sha256 of (query or document, text); vectors are exactly what the
//! model returned, so cached and fresh runs score identically.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use engram_core::Result;
use engram_embed::Embedder;
use sha2::{Digest, Sha256};

pub struct CachedEmbedder {
    inner: Arc<dyn Embedder>,
    file: PathBuf,
    vectors: Mutex<HashMap<[u8; 32], Vec<f32>>>,
    fresh: Mutex<usize>,
}

fn key(kind: u8, text: &str) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update([kind]);
    h.update(text.as_bytes());
    h.finalize().into()
}

impl CachedEmbedder {
    /// Load `file` if it exists. Layout: repeated [32-byte key][u32 dim][dim × f32], little-endian.
    pub fn open(inner: Arc<dyn Embedder>, file: PathBuf) -> CachedEmbedder {
        let mut vectors = HashMap::new();
        if let Ok(mut f) = std::fs::File::open(&file) {
            let mut bytes = vec![];
            let _ = f.read_to_end(&mut bytes);
            let mut at = 0;
            while at + 36 <= bytes.len() {
                let k: [u8; 32] = bytes[at..at + 32].try_into().unwrap();
                let dim = u32::from_le_bytes(bytes[at + 32..at + 36].try_into().unwrap()) as usize;
                let end = at + 36 + dim * 4;
                if end > bytes.len() {
                    break;
                }
                let v = bytes[at + 36..end].chunks_exact(4).map(|b| f32::from_le_bytes(b.try_into().unwrap())).collect();
                vectors.insert(k, v);
                at = end;
            }
        }
        CachedEmbedder { inner, file, vectors: Mutex::new(vectors), fresh: Mutex::new(0) }
    }

    pub fn cached(&self) -> usize {
        self.vectors.lock().unwrap().len()
    }

    /// Append everything new to the file (whole-file rewrite; the file is tens of MB).
    pub fn save(&self) -> std::io::Result<()> {
        if *self.fresh.lock().unwrap() == 0 {
            return Ok(());
        }
        if let Some(dir) = self.file.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let mut f = std::io::BufWriter::new(std::fs::File::create(&self.file)?);
        for (k, v) in self.vectors.lock().unwrap().iter() {
            f.write_all(k)?;
            f.write_all(&(v.len() as u32).to_le_bytes())?;
            for x in v {
                f.write_all(&x.to_le_bytes())?;
            }
        }
        f.flush()
    }

    fn get_or_embed(&self, kind: u8, texts: &[String], embed: impl Fn(&[String]) -> Result<Vec<Vec<f32>>>) -> Result<Vec<Vec<f32>>> {
        let keys: Vec<[u8; 32]> = texts.iter().map(|t| key(kind, t)).collect();
        let missing: Vec<usize> = {
            let cache = self.vectors.lock().unwrap();
            (0..texts.len()).filter(|&i| !cache.contains_key(&keys[i])).collect()
        };
        if !missing.is_empty() {
            let todo: Vec<String> = missing.iter().map(|&i| texts[i].clone()).collect();
            let fresh = embed(&todo)?;
            let mut cache = self.vectors.lock().unwrap();
            for (&i, v) in missing.iter().zip(fresh) {
                cache.insert(keys[i], v);
            }
            *self.fresh.lock().unwrap() += missing.len();
        }
        let cache = self.vectors.lock().unwrap();
        Ok(keys.iter().map(|k| cache[k].clone()).collect())
    }
}

impl Embedder for CachedEmbedder {
    fn model_id(&self) -> String {
        self.inner.model_id()
    }
    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        self.get_or_embed(0, texts, |t| self.inner.embed(t))
    }
    fn embed_query(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        self.get_or_embed(1, texts, |t| self.inner.embed_query(t))
    }
}
