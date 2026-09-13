use super::*;
use crate::build_index;
use std::{fs::File, io::Write};

fn engine(root: &tempfile::TempDir, name: &str, token: &str) -> RawSearchEngine {
    engine_with_documents(root, name, token, 1)
}

fn engine_with_documents(
    root: &tempfile::TempDir,
    name: &str,
    token: &str,
    documents: usize,
) -> RawSearchEngine {
    let chunks = root.path().join(format!("{name}.jsonl"));
    let channels = root.path().join(format!("{name}-channels.jsonl"));
    let index = root.path().join(name);
    let mut output = File::create(&chunks).unwrap();
    for id in 0..documents {
        writeln!(
            output,
            "{}",
            serde_json::json!({
                "id": format!("{name}-{id}"),
                "video_id": format!("{name}-{id}"),
                "youtuber_id": "creator",
                "youtuber_handle": "@creator",
                "title": name,
                "text": token,
                "t_start_ms": 0
            })
        )
        .unwrap();
    }
    File::create(&channels).unwrap();
    build_index(&chunks, &channels, &index).unwrap();
    RawSearchEngine::open(&index).unwrap()
}

#[test]
fn replacement_changes_reader_and_cache_generation_atomically() {
    let root = tempfile::tempdir().unwrap();
    let reloadable = ReloadableEngine::new(engine(&root, "old", "legacytoken"), 0);
    let params = SearchParams {
        q: "legacytoken".to_string(),
        ..SearchParams::default()
    };
    assert_eq!(reloadable.search(&params).unwrap()["found"], 1);

    reloadable
        .replace(engine(&root, "new", "currenttoken"), 7)
        .unwrap();

    assert_eq!(reloadable.cache_generation(), 7);
    assert_eq!(reloadable.search(&params).unwrap()["found"], 0);
}

#[test]
fn pinned_reader_keeps_pages_and_counts_on_one_generation_after_activation() {
    let root = tempfile::tempdir().unwrap();
    let reloadable = ReloadableEngine::new(engine_with_documents(&root, "old", "orbit", 3), 41);
    let (pinned, generation) = reloadable.snapshot().unwrap();
    let lease = Arc::downgrade(&pinned);
    let mut params = SearchParams {
        q: "orbit".into(),
        per_page: Some(1),
        ..SearchParams::default()
    };
    let before: Vec<_> = (1..=3)
        .map(|page| {
            params.page = Some(page);
            pinned.search(&params).unwrap()["hits"].clone()
        })
        .collect();
    assert!(
        before
            .iter()
            .all(|hits| hits.as_array().unwrap().len() == 1)
    );
    let ids: std::collections::BTreeSet<_> = before
        .iter()
        .map(|hits| hits[0]["document"]["id"].as_str().unwrap())
        .collect();
    assert_eq!(
        ids,
        std::collections::BTreeSet::from(["old-0", "old-1", "old-2"])
    );

    reloadable
        .replace(engine_with_documents(&root, "new", "orbit", 2), 42)
        .unwrap();

    assert_eq!(generation, 41);
    assert_eq!(reloadable.snapshot().unwrap().1, 42);
    assert_eq!(reloadable.search(&params).unwrap()["found"], 2);
    params.page = Some(1);
    let current = reloadable.search(&params).unwrap();
    assert!(
        current["hits"][0]["document"]["id"]
            .as_str()
            .unwrap()
            .starts_with("new-")
    );
    for page in 1..=3 {
        params.page = Some(page);
        let response = pinned.search(&params).unwrap();
        assert_eq!(response["found"], 3);
        assert_eq!(response["hits"], before[page as usize - 1]);
    }
    assert!(lease.upgrade().is_some());
    drop(pinned);
    assert!(
        lease.upgrade().is_none(),
        "retired reader survives only while leased"
    );
}

#[test]
fn snapshot_pairing_remains_consistent_in_activation_race_smoke_test() {
    let root = tempfile::tempdir().unwrap();
    let old = engine_with_documents(&root, "old", "orbit", 1);
    let next = engine_with_documents(&root, "new", "orbit", 2);
    let reloadable = ReloadableEngine::new(old, 1);
    let barrier = std::sync::Barrier::new(2);
    std::thread::scope(|scope| {
        scope.spawn(|| {
            barrier.wait();
            reloadable.replace(next, 2).unwrap();
        });
        barrier.wait();
        for _ in 0..1_000 {
            let (reader, generation) = reloadable.snapshot().unwrap();
            assert_eq!(reader.document_count(), generation);
        }
    });
    assert_eq!(reloadable.snapshot().unwrap().1, 2);
}
