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
        names(&GENERATION_FILES_SCHEMA_7),
        names(&youtube_index_store::RAW_GENERATION_FILES),
        "publication must upload exactly the files the reader validates"
    );
}

#[test]
fn schema_nine_is_schema_seven_plus_sidecars() {
    let mut expected = names(&GENERATION_FILES_SCHEMA_7);
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
fn the_advertised_format_is_the_schema_the_builder_writes() {
    let (root, engine) = test_engine();
    drop(engine);
    let manifest: Manifest = serde_json::from_reader(
        File::open(root.path().join("index").join(MANIFEST_FILE)).expect("manifest"),
    )
    .expect("manifest JSON");
    assert_eq!(
        manifest.schema_version,
        format_schema(crate::RAW_GENERATION_FORMAT),
        "RAW_GENERATION_FORMAT must name the schema build_index writes"
    );
    assert!(generation_files(manifest.schema_version).is_ok());
}
