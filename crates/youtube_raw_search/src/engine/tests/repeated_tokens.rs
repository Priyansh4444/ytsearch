use super::*;

#[test]
fn parser_keeps_the_repeated_token_sequence_for_phrase_evidence() {
    let parsed = parse_query("the end of the world").expect("parse query");
    assert_eq!(parsed.terms, ["the", "end", "of", "world"]);
    assert_eq!(parsed.sequence, ["the", "end", "of", "the", "world"]);

    let mixed = parse_query("at the end -of the day").expect("parse query");
    assert_eq!(mixed.terms, ["at", "the", "end", "day"]);
    assert_eq!(mixed.sequence, ["at", "the", "end", "the", "day"]);
}

#[test]
fn positional_summary_orders_by_the_full_sequence() {
    let terms = ["the", "end", "of", "world"].map(String::from);
    let sequence = ["the", "end", "of", "the", "world"].map(String::from);
    assert!(positional_summary("at the end of the world", &terms, &sequence).ordered);
    assert!(!positional_summary("the end of world", &terms, &sequence).ordered);
}

// A deduplicated sequence once rewarded "the end of world" as the ordered
// phrase for "the end of the world", above transcripts with the real phrase.
#[test]
fn a_sentence_with_a_repeated_word_ranks_its_exact_phrase_first() {
    let root = TempDir::new().expect("temp dir");
    let chunks_path = root.path().join("chunks.jsonl");
    let channels_path = root.path().join("channels.jsonl");
    let index_path = root.path().join("index");
    let mut chunks = File::create(&chunks_path).expect("chunks");
    for (index, (video, text)) in [
        ("literal", "the end of world the end of world"),
        (
            "phrase",
            "we talked about what happens at the end of the world today",
        ),
        ("scattered", "the world will end of course"),
        ("other", "nothing relevant here"),
    ]
    .into_iter()
    .enumerate()
    {
        let document = json!({
            "id": format!("{video}-{index}"), "video_id": video,
            "title": "Talk", "text": text, "youtuber_handle": "@test",
            "t_start_ms": 0, "t_end_ms": 1000,
            "published_at_timestamp": index + 1,
            "view_count": 0, "like_count": 0, "comment_count": 0
        });
        writeln!(chunks, "{document}").expect("write chunk");
    }
    File::create(&channels_path).expect("channels");
    build_index(&chunks_path, &channels_path, &index_path).expect("build");
    let engine = RawSearchEngine::open(&index_path).expect("open");

    let response = engine
        .search(&grouped_query("the end of the world"))
        .expect("search");
    let top = response["grouped_hits"][0]["group_key"][0].as_str();
    assert_eq!(top, Some("phrase"), "{response:#}");
}
