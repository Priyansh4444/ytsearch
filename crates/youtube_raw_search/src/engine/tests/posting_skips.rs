use super::*;

// "the" appears in every document with a varying frequency and first
// position, so the positional streams differ per posting.
fn skip_fixture() -> (TempDir, RawSearchEngine) {
    let root = TempDir::new().expect("temp dir");
    let chunks_path = root.path().join("chunks.jsonl");
    let channels_path = root.path().join("channels.jsonl");
    let index_path = root.path().join("index");
    let mut chunks = File::create(&chunks_path).expect("chunks");
    for index in 0..200 {
        let mut words = vec!["filler"; index % 4];
        words.extend(std::iter::repeat_n("the", 1 + index % 3));
        words.push(if index % 5 == 0 { "end" } else { "word" });
        let document = json!({
            "id": format!("video-{}-{index}", index / 7),
            "video_id": format!("video-{}", index / 7),
            "title": "notes",
            "text": words.join(" "),
            "youtuber_handle": "@skips",
        });
        writeln!(chunks, "{document}").expect("write chunk");
    }
    File::create(&channels_path).expect("channels");
    build_index(&chunks_path, &channels_path, &index_path).expect("build raw index");
    let engine = RawSearchEngine::open(&index_path).expect("open raw index");
    (root, engine)
}

fn text_streams<'a>(engine: &'a RawSearchEngine, term: &str) -> skips::PositionedStreams<'a> {
    let entry = engine
        .inner
        .lexicon
        .lookup(term)
        .expect("lookup")
        .expect("term");
    let full = &engine.inner.full_positions;
    skips::PositionedStreams {
        postings: engine
            .postings_range(entry.text_offset, entry.text_length)
            .expect("postings"),
        first_positions: engine
            .inner
            .positions
            .range(entry.term, IndexedField::Text)
            .expect("first positions"),
        remaining_positions: full
            .range(entry.term, IndexedField::Text)
            .expect("remaining"),
        expected: entry.text_frequency,
    }
}

fn cursor<'a>(
    streams: skips::PositionedStreams<'a>,
    skips: Option<Arc<[skips::SkipEntry]>>,
) -> PositionedPostingCursor<'a> {
    PositionedPostingCursor::new(
        streams.postings,
        streams.first_positions,
        streams.remaining_positions,
        streams.expected,
        skips,
    )
}

fn walk(mut cursor: PositionedPostingCursor<'_>, targets: &[u32]) -> Vec<(u32, Vec<u32>)> {
    let mut found = Vec::new();
    let mut positions = Vec::new();
    for &target in targets {
        match cursor.next_at_least(target, &mut positions).expect("seek") {
            Some(id) => found.push((id, positions.clone())),
            None => break,
        }
    }
    // Draining validates the complete counts after any checkpoint jump.
    while cursor
        .next_at_least(u32::MAX, &mut positions)
        .expect("drain")
        .is_some()
    {}
    found
}

#[test]
fn checkpoints_record_the_state_after_each_interval() {
    let (_root, engine) = skip_fixture();
    let streams = text_streams(&engine, "the");
    assert_eq!(streams.expected, 200);
    let entries = skips::build_skip_entries(streams, 3).expect("skips");
    assert_eq!(entries.len(), 66);

    let mut reference = cursor(streams, None);
    let mut positions = Vec::new();
    let mut next = 0;
    for entry in entries.iter() {
        while reference.decoded < entry.decoded {
            let id = reference
                .next_at_least(next, &mut positions)
                .expect("decode")
                .expect("posting");
            next = id + 1;
        }
        assert_eq!(reference.skip_entry(), *entry);
    }
}

#[test]
fn seeks_through_checkpoints_match_the_full_walk() {
    let (_root, engine) = skip_fixture();
    let streams = text_streams(&engine, "the");
    let entries: Arc<[skips::SkipEntry]> =
        skips::build_skip_entries(streams, 3).expect("skips").into();
    for stride in [1, 2, 3, 5, 17, 64, 199, 250] {
        for start in [0, 1, 4, 100] {
            let targets = (start..220).step_by(stride).collect::<Vec<u32>>();
            assert_eq!(
                walk(cursor(streams, Some(Arc::clone(&entries))), &targets),
                walk(cursor(streams, None), &targets),
                "stride={stride} start={start}"
            );
        }
    }
}

#[test]
fn short_lists_walk_without_checkpoints() {
    let (_root, engine) = skip_fixture();
    let streams = text_streams(&engine, "end");
    assert!(streams.expected < skips::SKIP_MIN_POSTINGS);
    let directory = skips::SkipDirectory::default();
    assert!(
        directory
            .get_or_build(0, IndexedField::Text, streams)
            .is_none()
    );
}
