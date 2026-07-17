//! Reproducible, isolated Qdrant latency/recall benchmark.
//!
//! This intentionally uses deterministic synthetic vectors. It measures the
//! vector index, not embedding quality; semantic quality needs a judged corpus.

use reqwest::Client;
use serde_json::{Value, json};
use std::{env, error::Error, time::Instant};

const COLLECTION: &str = "ytsearch_vector_bench";

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let endpoint = env::var("QDRANT_URL").unwrap_or_else(|_| "http://127.0.0.1:6333".into());
    let count = number("VECTOR_COUNT", 20_000);
    let dimensions = number("VECTOR_DIMENSIONS", 384);
    let query_count = number("QUERY_COUNT", 100);
    let warmups = number("WARMUP_QUERIES", 10);
    let top_k = number("TOP_K", 10);
    let hnsw_ef = number("HNSW_EF", 64);
    let client = Client::new();

    let _ = client
        .delete(format!("{endpoint}/collections/{COLLECTION}"))
        .send()
        .await?;
    checked(
        client
            .put(format!("{endpoint}/collections/{COLLECTION}"))
            .json(&json!({
                "vectors": {"size": dimensions, "distance": "Cosine"},
                "hnsw_config": {"m": 16, "ef_construct": 100, "full_scan_threshold": 1000},
                "optimizers_config": {"indexing_threshold": 1000}
            }))
            .send()
            .await?,
    )
    .await?;

    let ingest_started = Instant::now();
    for start in (0..count).step_by(128) {
        let end = (start + 128).min(count);
        let points = (start..end)
            .map(|id| {
                json!({
                    "id": id,
                    "vector": vector(id as u64, dimensions),
                    "payload": {"creator": format!("creator-{}", id % 100)}
                })
            })
            .collect::<Vec<_>>();
        checked(
            client
                .put(format!(
                    "{endpoint}/collections/{COLLECTION}/points?wait=true"
                ))
                .json(&json!({"points": points}))
                .send()
                .await?,
        )
        .await?;
    }
    checked(
        client
            .put(format!(
                "{endpoint}/collections/{COLLECTION}/index?wait=true"
            ))
            .json(&json!({"field_name": "creator", "field_schema": "keyword"}))
            .send()
            .await?,
    )
    .await?;
    wait_for_index(&client, &endpoint, count).await?;

    println!(
        "engine=qdrant vectors={count} dimensions={dimensions} queries={query_count} top_k={top_k} hnsw_ef={hnsw_ef} ingest_ms={}",
        ingest_started.elapsed().as_millis()
    );
    run_case(
        &client,
        &endpoint,
        count,
        dimensions,
        query_count,
        warmups,
        top_k,
        hnsw_ef,
        false,
    )
    .await?;
    run_case(
        &client,
        &endpoint,
        count,
        dimensions,
        query_count,
        warmups,
        top_k,
        hnsw_ef,
        true,
    )
    .await?;
    Ok(())
}

async fn run_case(
    client: &Client,
    endpoint: &str,
    count: usize,
    dimensions: usize,
    query_count: usize,
    warmups: usize,
    top_k: usize,
    hnsw_ef: usize,
    filtered: bool,
) -> Result<(), Box<dyn Error>> {
    let mut exact_latency = Vec::with_capacity(query_count);
    let mut ann_latency = Vec::with_capacity(query_count);
    let mut recalls = Vec::with_capacity(query_count);

    for query_index in 0..(warmups + query_count) {
        let id = (query_index.wrapping_mul(7_919) + 17) % count;
        let mut query = vector(id as u64, dimensions);
        // A small deterministic perturbation prevents an exact self-match from
        // making every approximate query unrealistically easy.
        for (offset, value) in query.iter_mut().enumerate() {
            *value += (((query_index + offset * 13) % 17) as f32 - 8.0) * 0.0005;
        }
        normalize(&mut query);
        let filter = filtered.then(|| {
            json!({"must": [{"key": "creator", "match": {"value": format!("creator-{}", id % 100)}}]})
        });

        let (exact, exact_ms) = search(
            client,
            endpoint,
            &query,
            top_k,
            hnsw_ef,
            true,
            filter.as_ref(),
        )
        .await?;
        let (ann, ann_ms) = search(
            client,
            endpoint,
            &query,
            top_k,
            hnsw_ef,
            false,
            filter.as_ref(),
        )
        .await?;
        if query_index >= warmups {
            exact_latency.push(exact_ms);
            ann_latency.push(ann_ms);
            let found = ann.iter().filter(|id| exact.contains(id)).count();
            recalls.push(found as f64 / exact.len().max(1) as f64);
        }
    }

    exact_latency.sort_unstable();
    ann_latency.sort_unstable();
    let label = if filtered {
        "creator-filtered"
    } else {
        "unfiltered"
    };
    println!(
        "case={label} exact_p50_us={} exact_p95_us={} ann_p50_us={} ann_p95_us={} recall_at_{top_k}={:.4}",
        percentile(&exact_latency, 50),
        percentile(&exact_latency, 95),
        percentile(&ann_latency, 50),
        percentile(&ann_latency, 95),
        recalls.iter().sum::<f64>() / recalls.len().max(1) as f64,
    );
    Ok(())
}

async fn search(
    client: &Client,
    endpoint: &str,
    query: &[f32],
    limit: usize,
    hnsw_ef: usize,
    exact: bool,
    filter: Option<&Value>,
) -> Result<(Vec<u64>, u128), Box<dyn Error>> {
    let mut body = json!({
        "query": query,
        "limit": limit,
        "params": {"exact": exact, "hnsw_ef": hnsw_ef},
        "with_payload": false,
        "with_vector": false
    });
    if let Some(filter) = filter {
        body["filter"] = filter.clone();
    }
    let started = Instant::now();
    let value = checked(
        client
            .post(format!("{endpoint}/collections/{COLLECTION}/points/query"))
            .json(&body)
            .send()
            .await?,
    )
    .await?;
    let elapsed = started.elapsed().as_micros();
    let ids = value["result"]["points"]
        .as_array()
        .ok_or("Qdrant response did not contain result.points")?
        .iter()
        .filter_map(|point| point["id"].as_u64())
        .collect();
    Ok((ids, elapsed))
}

async fn wait_for_index(
    client: &Client,
    endpoint: &str,
    count: usize,
) -> Result<(), Box<dyn Error>> {
    for _ in 0..120 {
        let value = checked(
            client
                .get(format!("{endpoint}/collections/{COLLECTION}"))
                .send()
                .await?,
        )
        .await?;
        if value["result"]["status"] == "green"
            && value["result"]["indexed_vectors_count"]
                .as_u64()
                .unwrap_or(0)
                >= (count as u64 * 95 / 100)
        {
            return Ok(());
        }
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    }
    Err("Qdrant index did not become ready".into())
}

async fn checked(response: reqwest::Response) -> Result<Value, Box<dyn Error>> {
    let status = response.status();
    let body = response.text().await?;
    if !status.is_success() {
        return Err(format!("Qdrant returned {status}: {body}").into());
    }
    Ok(serde_json::from_str(&body)?)
}

fn vector(id: u64, dimensions: usize) -> Vec<f32> {
    // Nearby IDs are deliberately not nearby vectors. This approximates a
    // shuffled embedding corpus and avoids depending on an ML runtime.
    let mut state = id ^ 0x9e37_79b9_7f4a_7c15;
    let mut values = Vec::with_capacity(dimensions);
    for _ in 0..dimensions {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        values.push(((state >> 40) as f32 / (1_u32 << 24) as f32) * 2.0 - 1.0);
    }
    normalize(&mut values);
    values
}

fn normalize(values: &mut [f32]) {
    let norm = values.iter().map(|value| value * value).sum::<f32>().sqrt();
    for value in values {
        *value /= norm;
    }
}

fn number(name: &str, default: usize) -> usize {
    env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}

fn percentile(sorted: &[u128], percentile: usize) -> u128 {
    sorted[(sorted.len().saturating_sub(1) * percentile) / 100]
}
