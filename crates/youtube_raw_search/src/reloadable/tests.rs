use super::*;
use crate::build_index;
use std::{fs::File, io::Write};

fn engine(root: &tempfile::TempDir, name: &str, token: &str) -> RawSearchEngine {
    let chunks = root.path().join(format!("{name}.jsonl"));
    let channels = root.path().join(format!("{name}-channels.jsonl"));
    let index = root.path().join(name);
    writeln!(
        File::create(&chunks).unwrap(),
        "{}",
        serde_json::json!({
            "id": format!("{name}-0"),
            "video_id": name,
            "youtuber_id": "creator",
            "youtuber_handle": "@creator",
            "title": name,
            "text": token,
            "t_start_ms": 0
        })
    )
    .unwrap();
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
