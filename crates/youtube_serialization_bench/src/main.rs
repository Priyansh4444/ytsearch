use prost::Message;
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

include!(concat!(env!("OUT_DIR"), "/ytsearch.rs"));

#[derive(Clone, Deserialize, Serialize)]
struct JsonChunk {
    id: String,
    youtuber_id: String,
    youtuber_handle: String,
    youtuber_name: String,
    channel_id: String,
    video_id: String,
    title: String,
    text: String,
    prev_text: Option<String>,
    next_text: Option<String>,
    sequence: u32,
    chapter: Option<String>,
    chapter_index: Option<u32>,
    duration: Option<u32>,
    t_start_ms: u32,
    t_end_ms: u32,
    thumbnail: Option<String>,
    upload_date: Option<String>,
    published_at: Option<String>,
    published_at_timestamp: Option<i64>,
    uploader_id: String,
    embedding: Option<Vec<f32>>,
    extra: serde_json::Map<String, serde_json::Value>,
}

fn chunk(index: usize) -> (JsonChunk, TranscriptChunk) {
    let id = format!("chunk-{index}");
    let title = format!("A representative transcript title {index}");
    let text = "This is a representative transcript segment used for serialization benchmarking."
        .to_string();
    let published_at = Some("2026-01-01T00:00:00Z".to_string());
    let json = JsonChunk {
        id: id.clone(),
        youtuber_id: "t3dotgg".to_string(),
        youtuber_handle: "@t3dotgg".to_string(),
        youtuber_name: "Theo - t3.gg".to_string(),
        channel_id: "UC123456789".to_string(),
        video_id: format!("video-{index}"),
        title: title.clone(),
        text: text.clone(),
        prev_text: Some("Previous segment.".to_string()),
        next_text: Some("Next segment.".to_string()),
        sequence: index as u32,
        chapter: Some("Introduction".to_string()),
        chapter_index: Some(0),
        duration: Some(600_000),
        t_start_ms: (index as u32) * 10_000,
        t_end_ms: (index as u32) * 10_000 + 10_000,
        thumbnail: Some("https://i.ytimg.com/vi/video/maxresdefault.jpg".to_string()),
        upload_date: Some("20260101".to_string()),
        published_at: published_at.clone(),
        published_at_timestamp: Some(1_767_225_600),
        uploader_id: "@t3dotgg".to_string(),
        embedding: None,
        extra: serde_json::Map::new(),
    };
    let proto = TranscriptChunk {
        id,
        youtuber_id: "t3dotgg".to_string(),
        youtuber_handle: "@t3dotgg".to_string(),
        youtuber_name: "Theo - t3.gg".to_string(),
        channel_id: "UC123456789".to_string(),
        video_id: format!("video-{index}"),
        title,
        text,
        prev_text: Some("Previous segment.".to_string()),
        next_text: Some("Next segment.".to_string()),
        sequence: index as u32,
        chapter: Some("Introduction".to_string()),
        chapter_index: Some(0),
        duration: Some(600_000),
        t_start_ms: (index as u32) * 10_000,
        t_end_ms: (index as u32) * 10_000 + 10_000,
        thumbnail: Some("https://i.ytimg.com/vi/video/maxresdefault.jpg".to_string()),
        upload_date: Some("20260101".to_string()),
        published_at,
        published_at_timestamp: Some(1_767_225_600),
        uploader_id: "@t3dotgg".to_string(),
        embedding_f32_le: Vec::new(),
        extra_json: "{}".to_string(),
    };
    (json, proto)
}

fn elapsed_per_round(total: Duration, rounds: usize) -> f64 {
    total.as_secs_f64() * 1_000.0 / rounds as f64
}

fn main() {
    let chunks = std::env::args()
        .nth(1)
        .and_then(|value| value.parse().ok())
        .unwrap_or(10_000usize);
    let rounds = std::env::args()
        .nth(2)
        .and_then(|value| value.parse().ok())
        .unwrap_or(10usize);
    let samples = (0..chunks).map(chunk).collect::<Vec<_>>();

    let mut json_bytes = 0usize;
    let json_encode_start = Instant::now();
    for _ in 0..rounds {
        for (json, _) in &samples {
            json_bytes += serde_json::to_vec(json).expect("encode JSON").len();
        }
    }
    let json_encode = json_encode_start.elapsed();

    let json_encoded = serde_json::to_vec(&samples[0].0).expect("encode JSON sample");
    let json_decode_start = Instant::now();
    for _ in 0..(rounds * chunks) {
        std::hint::black_box(
            serde_json::from_slice::<JsonChunk>(&json_encoded).expect("decode JSON"),
        );
    }
    let json_decode = json_decode_start.elapsed();

    let mut proto_bytes = 0usize;
    let proto_encode_start = Instant::now();
    for _ in 0..rounds {
        for (_, proto) in &samples {
            proto_bytes += proto.encoded_len();
            let mut buffer = Vec::with_capacity(proto.encoded_len());
            proto.encode(&mut buffer).expect("encode protobuf");
            std::hint::black_box(buffer);
        }
    }
    let proto_encode = proto_encode_start.elapsed();

    let encoded = samples[0].1.encode_to_vec();
    let proto_decode_start = Instant::now();
    for _ in 0..(rounds * chunks) {
        std::hint::black_box(TranscriptChunk::decode(encoded.as_slice()).expect("decode protobuf"));
    }
    let proto_decode = proto_decode_start.elapsed();

    println!("chunks={chunks} rounds={rounds}");
    let sample_count = rounds * chunks;
    println!(
        "json_bytes_total={json_bytes} json_bytes_per_chunk={:.1} encode_ms_per_round={:.2} decode_ms_per_round={:.2}",
        json_bytes as f64 / sample_count as f64,
        elapsed_per_round(json_encode, rounds),
        elapsed_per_round(json_decode, rounds)
    );
    println!(
        "protobuf_bytes_total={proto_bytes} protobuf_bytes_per_chunk={:.1} encode_ms_per_round={:.2} decode_ms_per_round={:.2}",
        proto_bytes as f64 / sample_count as f64,
        elapsed_per_round(proto_encode, rounds),
        elapsed_per_round(proto_decode, rounds)
    );
    println!(
        "protobuf_size_ratio={:.3}",
        proto_bytes as f64 / json_bytes as f64
    );
}
