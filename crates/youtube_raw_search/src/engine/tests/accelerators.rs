//! Schema-9 sidecars: their contents equal what the reader would compute at
//! run time, and a schema-9 generation answers exactly as its schema-7 source.

use super::skips::build_skip_entries;
use super::*;
use crate::engine::accelerators::{
    AcceleratorConfig, BlockBound, score_factor, text_streams, upgrade_to_schema_nine,
};

// Small thresholds so a few hundred documents exercise every sidecar part.
const TEST_CONFIG: AcceleratorConfig = AcceleratorConfig {
    min_postings: 8,
    block_postings: 4,
    text_bitmap_budget_bytes: 64,
    title_bitmap_min_per_mille: 100,
};

const WORDS: [&str; 12] = [
    "the", "and", "is", "this", "end", "of", "fire", "react", "problem", "that", "world", "code",
];

/// Builds the same skewed corpus twice: a schema-7 generation and a copy
/// upgraded to schema 9.
fn schema_pair() -> (TempDir, RawSearchEngine, RawSearchEngine) {
    let root = TempDir::new().expect("temp dir");
    let chunks_path = root.path().join("chunks.jsonl");
    let channels_path = root.path().join("channels.jsonl");
    let mut chunks = File::create(&chunks_path).expect("chunks");
    let mut state = 0x2545_f491_u64;
    let mut next = |bound: usize| {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1);
        (state >> 33) as usize % bound
    };
    for index in 0..400 {
        let video = index / 7;
        // Low word indices are far more common, as in real transcripts.
        let length = 3 + next(12);
        let text = (0..length)
            .map(|_| WORDS[next(WORDS.len()).min(next(WORDS.len()))])
            .collect::<Vec<_>>()
            .join(" ");
        let title = format!("{} {}", WORDS[video % 5], WORDS[4 + video % 8]);
        let start = (index % 7) as u64 * 15_000;
        let document = json!({
            "id": format!("v{video}-{index}"), "video_id": format!("v{video}"),
            "title": title, "text": text, "youtuber_handle": format!("@c{}", video % 3),
            "t_start_ms": start, "t_end_ms": start + 1000,
            "published_at_timestamp": video, "view_count": video * 13 % 97,
        });
        writeln!(chunks, "{document}").expect("write chunk");
    }
    File::create(&channels_path).expect("channels");
    let seven = root.path().join("seven");
    let nine = root.path().join("nine");
    build_index(&chunks_path, &channels_path, &seven).expect("build schema 7");
    build_index(&chunks_path, &channels_path, &nine).expect("build schema 9 source");
    upgrade_to_schema_nine(&nine, TEST_CONFIG).expect("add accelerators");
    let seven = RawSearchEngine::open(&seven).expect("open schema 7");
    let nine = RawSearchEngine::open(&nine).expect("open schema 9");
    (root, seven, nine)
}

fn accelerated_terms(engine: &RawSearchEngine) -> Vec<TermEntry> {
    (0..engine.inner.lexicon.count)
        .map(|index| {
            engine
                .inner
                .lexicon
                .entry_at(index)
                .expect("lexicon entry")
                .1
        })
        .filter(|entry| entry.text_frequency >= TEST_CONFIG.min_postings)
        .collect()
}

#[test]
fn schema_nine_answers_exactly_as_schema_seven() {
    let (_root, seven, nine) = schema_pair();
    assert_eq!(nine.inner.manifest.schema_version, 9);
    assert!(nine.inner.accelerators.is_some() && nine.inner.bitmaps.is_some());
    let mut queries = vec![
        "the",
        "and the",
        "this is the end",
        "\"this is the\"",
        "\"the end\"",
        "fire code",
        "the problem is that",
        "react -the",
        "wor",
        "end of the world",
        "\"and the\" code",
    ]
    .into_iter()
    .map(grouped_query)
    .collect::<Vec<_>>();
    for query in ["the end", "\"is the\"", "code that"] {
        queries.push(SearchParams {
            group_by: None,
            ..grouped_query(query)
        });
        queries.push(SearchParams {
            sort_by: Some("published_at_timestamp:desc".to_string()),
            ..grouped_query(query)
        });
        queries.push(SearchParams {
            filter_by: Some("youtuber_handle:=@c1".to_string()),
            page: Some(2),
            per_page: Some(3),
            offset: None,
            limit: None,
            ..grouped_query(query)
        });
    }
    for params in &queries {
        assert_eq!(
            seven.search(params).expect("schema 7 search"),
            nine.search(params).expect("schema 9 search"),
            "schema 9 diverged for {:?}",
            params.q
        );
    }
}

#[test]
fn sidecar_checkpoints_equal_runtime_checkpoints() {
    let (_root, _seven, nine) = schema_pair();
    let sidecar = nine.inner.accelerators.as_ref().expect("accelerators");
    let terms = accelerated_terms(&nine);
    assert!(terms.len() >= 5, "fixture must accelerate several terms");
    for entry in terms {
        let streams = text_streams(&nine, entry).expect("streams");
        let mut expected = build_skip_entries(streams, TEST_CONFIG.block_postings).expect("build");
        expected.retain(|checkpoint| checkpoint.decoded < entry.text_frequency);
        let stored = sidecar
            .checkpoints(entry.term, streams)
            .expect("valid checkpoints")
            .expect("accelerated term");
        assert_eq!(&*stored, expected.as_slice(), "term {}", entry.term);
    }
}

#[test]
fn block_bounds_are_the_maximum_factor_of_each_block() {
    let (_root, _seven, nine) = schema_pair();
    let sidecar = nine.inner.accelerators.as_ref().expect("accelerators");
    let average = nine.inner.manifest.average_document_tokens;
    for entry in accelerated_terms(&nine) {
        // Independent decode of (document, frequency, first position).
        let streams = text_streams(&nine, entry).expect("streams");
        let (mut cursor, mut first_cursor, mut document) = (0, 0, 0_u32);
        let mut postings = Vec::new();
        while cursor < streams.postings.len() {
            document += read_varint(streams.postings, &mut cursor, streams.postings.len()).unwrap();
            let frequency =
                read_varint(streams.postings, &mut cursor, streams.postings.len()).unwrap();
            let first = read_varint(
                streams.first_positions,
                &mut first_cursor,
                streams.first_positions.len(),
            )
            .unwrap();
            let tokens = nine
                .inner
                .metadata
                .search_document(document)
                .unwrap()
                .tokens;
            postings.push((document, score_factor(frequency, tokens, average, first)));
        }
        let expected = postings
            .chunks(TEST_CONFIG.block_postings as usize)
            .map(|block| BlockBound {
                last_document: block.last().unwrap().0,
                max_factor: block
                    .iter()
                    .map(|(_, factor)| *factor)
                    .fold(f32::MIN, f32::max),
            })
            .collect::<Vec<_>>();
        assert_eq!(sidecar.blocks(entry.term).unwrap().unwrap(), expected);
    }
}

#[test]
fn bitmaps_equal_posting_sets() {
    let (_root, _seven, nine) = schema_pair();
    let bitmaps = nine.inner.bitmaps.as_ref().expect("bitmaps");
    let documents = nine.inner.manifest.documents as usize;
    let mut text_bitmaps = 0;
    let mut title_bitmaps = 0;
    for index in 0..nine.inner.lexicon.count {
        let (_, entry) = nine.inner.lexicon.entry_at(index).unwrap();
        for field in [IndexedField::Text, IndexedField::Title] {
            let Some(words) = bitmaps.words(entry.term, field) else {
                continue;
            };
            let (offset, length) = match field {
                IndexedField::Text => (entry.text_offset, entry.text_length),
                IndexedField::Title => (entry.title_offset, entry.title_length),
            };
            let bytes = nine.postings_range(offset, length).unwrap();
            let (mut cursor, mut id, mut expected) = (0, 0_u32, Vec::new());
            while cursor < bytes.len() {
                id += read_varint(bytes, &mut cursor, bytes.len()).unwrap();
                read_varint(bytes, &mut cursor, bytes.len()).unwrap();
                expected.push(id);
            }
            let actual = words
                .enumerate()
                .flat_map(|(word, bits)| {
                    (0..64)
                        .filter(move |bit| bits & (1 << bit) != 0)
                        .map(move |bit| (word * 64 + bit) as u32)
                })
                .collect::<Vec<_>>();
            assert_eq!(actual, expected, "term {} {field:?}", entry.term);
            match field {
                IndexedField::Text => text_bitmaps += 1,
                IndexedField::Title => title_bitmaps += 1,
            }
        }
    }
    // A 64-byte budget holds exactly one 400-document bitmap (56 bytes).
    assert_eq!(documents.div_ceil(64) * 8, 56);
    assert_eq!(text_bitmaps, 1);
    assert!(title_bitmaps > 0);
}

#[test]
fn corrupt_sidecars_are_rejected() {
    let (root, _seven, nine) = schema_pair();
    drop(nine);
    let index = root.path().join("nine");
    let path = index.join(crate::engine::accelerators::ACCELERATORS_FILE);
    let mut bytes = fs::read(&path).unwrap();
    // The first directory entry's block count.
    bytes[HEADER_BYTES + 8 + 4] ^= 1;
    fs::write(&path, &bytes).unwrap();
    assert!(RawSearchEngine::open(&index).is_err(), "checksum must fail");
    assert!(
        crate::engine::accelerators::Accelerators::open(&path, usize::MAX).is_err(),
        "structural validation must fail without the checksum"
    );
}

#[test]
fn only_schema_seven_generations_are_upgraded() {
    let (root, _seven, nine) = schema_pair();
    drop(nine);
    let error = upgrade_to_schema_nine(&root.path().join("nine"), TEST_CONFIG)
        .expect_err("schema 9 must not be upgraded twice");
    assert!(error.to_string().contains("found schema 9"), "{error}");
}
