//! Invariants that tie every generation-format consumer to one definition.
//! Each guards a failure seen in production: the reader advertising one
//! format while publishers built another, and file lists drifting apart.

use super::*;
use std::collections::BTreeSet;

fn names(files: &[&str]) -> BTreeSet<String> {
    files.iter().map(|name| name.to_string()).collect()
}

/// The schema number encoded in `raw-index-schema-<n>-v<m>`.
fn format_schema(format: &str) -> u8 {
    format
        .strip_prefix("raw-index-schema-")
        .and_then(|rest| rest.split('-').next())
        .and_then(|number| number.parse().ok())
        .expect("generation format names its schema")
}

#[test]
fn published_files_match_the_reader_schema_file_list() {
    assert_eq!(
        names(&GENERATION_FILES_SCHEMA_9),
        names(&youtube_index_store::RAW_GENERATION_FILES),
        "publication must upload exactly the files the reader validates"
    );
}

#[test]
fn schema_nine_is_schema_seven_plus_sidecars() {
    let mut expected = names(GENERATION_FILES_SCHEMA_7);
    expected.insert(crate::engine::accelerators::ACCELERATORS_FILE.to_string());
    expected.insert(crate::engine::accelerators::BITMAPS_FILE.to_string());
    expected.insert(crate::engine::plan_bounds::PLAN_BOUNDS_FILE.to_string());
    assert_eq!(names(&GENERATION_FILES_SCHEMA_9), expected);
}

#[test]
fn every_hot_file_belongs_to_every_readable_schema() {
    for schema in [6, 7, 9] {
        let files = names(generation_files(schema).expect("readable schema"));
        for hot in HOT_MAPPED_FILES {
            assert!(files.contains(hot), "{hot} missing from schema {schema}");
        }
    }
}

#[test]
fn the_advertised_format_is_the_schema_publication_builds() {
    let root = TempDir::new().expect("temp dir");
    let index = root.path().join("candidate");
    let chunks = [
        json!({"id":"a-1","video_id":"a","title":"Rust performance","text":"a compact search index","youtuber_handle":"@one","t_start_ms":0,"t_end_ms":1000}),
        json!({"id":"b-1","video_id":"b","title":"Runtimes","text":"the search runtime","youtuber_handle":"@two","t_start_ms":0,"t_end_ms":1000}),
    ]
    .map(|chunk| Ok(chunk.to_string()));
    build_publication_candidate(chunks.into_iter(), Vec::new(), &index).expect("publication build");
    let engine = RawSearchEngine::open(&index).expect("open candidate");
    assert_eq!(
        engine.inner.manifest.schema_version,
        format_schema(crate::RAW_GENERATION_FORMAT),
        "RAW_GENERATION_FORMAT must name the schema publication builds"
    );
    assert_eq!(
        names(generation_files(engine.inner.manifest.schema_version).expect("readable")),
        names(&youtube_index_store::RAW_GENERATION_FILES),
        "publication must upload exactly the files its schema lists"
    );
}
