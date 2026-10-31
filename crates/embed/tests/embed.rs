use engram_core::db::open_memory;
use engram_embed::embedder::{Embedder, HashEmbedder};
use engram_embed::fuse::rrf;
use engram_embed::store::{dot, embed_and_store, memories_missing, top_k};
use engram_core::memory::{NewMemory, create_memory};

#[test]
fn hash_embeddings_are_normalized_deterministic_and_lexically_meaningful() {
    let e = HashEmbedder::default();
    let v = e.embed(&["lidar voxel filter".into(), "lidar voxel filter".into(), "billing stripe webhook".into(), "voxelized lidar filtering".into()]).unwrap();
    assert!((dot(&v[0], &v[0]) - 1.0).abs() < 1e-4);
    assert_eq!(v[0], v[1]);
    assert!(dot(&v[0], &v[3]) > dot(&v[0], &v[2]), "sub-word overlap ranks above an unrelated text");
}

#[test]
fn top_k_returns_the_nearest_vectors_in_order() {
    let db = open_memory().unwrap();
    let e = HashEmbedder::default();
    let docs = [("c1", "camera calibration intrinsics"), ("c2", "lidar point cloud downsampling with voxels"), ("c3", "stripe billing webhooks")];
    let items: Vec<(String, String)> = docs.iter().map(|(i, t)| (i.to_string(), t.to_string())).collect();
    assert_eq!(embed_and_store(&db, &e, "chunk", &items).unwrap(), 3);
    let q = e.embed(&["how do we downsample the lidar cloud".into()]).unwrap().remove(0);
    let hits = top_k(&db, &e.model_id(), "chunk", &q, 2).unwrap();
    assert_eq!(hits.len(), 2);
    assert_eq!(hits[0].id, "c2");
    assert!(hits[0].score >= hits[1].score);
}

#[test]
fn rrf_rewards_items_ranked_well_by_several_lists() {
    let fused = rrf(&[vec!["a".into(), "b".into(), "c".into()], vec!["b".into(), "c".into()], vec!["b".into()]], 60.0);
    assert_eq!(fused[0].0, "b");
    assert_eq!(fused.len(), 3);
}

#[test]
fn memories_missing_lists_only_unembedded_current_memories() {
    let db = open_memory().unwrap();
    let e = HashEmbedder::default();
    let m = create_memory(&db, NewMemory::new("fact", "Lidar rate", "10 Hz")).unwrap();
    assert_eq!(memories_missing(&db, &e.model_id(), 10).unwrap().len(), 1);
    embed_and_store(&db, &e, "memory", &[(m.id, "Lidar rate. 10 Hz".into())]).unwrap();
    assert!(memories_missing(&db, &e.model_id(), 10).unwrap().is_empty());
}

/// Deterministic pseudo-random unit vectors (no `rand` dependency).
fn unit_vectors(n: usize, dim: usize, seed: u64) -> Vec<Vec<f32>> {
    let mut s = seed;
    (0..n)
        .map(|_| {
            let v: Vec<f32> = (0..dim)
                .map(|_| {
                    s = s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                    ((s >> 33) as f32 / (1u64 << 31) as f32) - 0.5
                })
                .collect();
            let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
            v.into_iter().map(|x| x / norm).collect()
        })
        .collect()
}

#[test]
fn nearest_neighbour_index_agrees_with_exact_search_and_tracks_changes() {
    use engram_embed::ann::top_k_with;
    use engram_embed::store::{delete_owners, exact_top_k, put};
    let dir = tempfile::tempdir().unwrap();
    let db = engram_core::db::open(&dir.path().join("engram.sqlite")).unwrap();
    let vectors = unit_vectors(3000, 64, 7);
    for (i, v) in vectors.iter().enumerate() {
        put(&db, &format!("chk_{i}"), "chunk", "m", v).unwrap();
    }
    let queries = unit_vectors(20, 64, 99);
    let mut overlap = 0;
    for q in &queries {
        let exact: Vec<String> = exact_top_k(&db, "m", "chunk", q, 10).unwrap().into_iter().map(|h| h.id).collect();
        let approx = top_k_with(&db, "m", "chunk", q, 10, 100).unwrap();
        overlap += approx.iter().filter(|h| exact.contains(&h.id)).count();
    }
    assert!(overlap as f64 / 200.0 >= 0.9, "recall@10 vs exact: {}", overlap as f64 / 200.0);

    // A deleted vector never comes back; a replaced one is found by its new value.
    let target = &queries[0];
    let best = top_k_with(&db, "m", "chunk", target, 1, 100).unwrap()[0].id.clone();
    delete_owners(&db, &[best.clone()]).unwrap();
    assert!(top_k_with(&db, "m", "chunk", target, 10, 100).unwrap().iter().all(|h| h.id != best));
    put(&db, "chk_5", "chunk", "m", target).unwrap();
    assert_eq!(top_k_with(&db, "m", "chunk", target, 1, 100).unwrap()[0].id, "chk_5");
}
