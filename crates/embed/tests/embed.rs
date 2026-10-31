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
