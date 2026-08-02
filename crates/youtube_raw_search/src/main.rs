use std::{env, net::SocketAddr, path::PathBuf, process::ExitCode, sync::Arc};
use youtube_raw_search::{RawSearchEngine, build_index, slice_corpus};
use youtube_search_service::ServerConfig;

#[tokio::main]
async fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .json()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "youtube_raw_search=info".into()),
        )
        .init();
    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!(%error, "raw search experiment failed");
            ExitCode::FAILURE
        }
    }
}

async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = env::args().skip(1);
    match args.next().as_deref() {
        Some("build") => {
            let chunks = required_path(args.next(), "chunks JSONL path")?;
            let channels = required_path(args.next(), "channels JSONL path")?;
            let index = required_path(args.next(), "index directory")?;
            let report = build_index(&chunks, &channels, &index)?;
            println!("{}", serde_json::to_string_pretty(&report)?);
        }
        Some("serve") => {
            let index_path = args
                .next()
                .map(PathBuf::from)
                .or_else(|| env::var_os("RAW_SEARCH_INDEX_PATH").map(PathBuf::from))
                .unwrap_or_else(|| PathBuf::from("/data/index"));
            let bind = env::var("RAW_SEARCH_BIND")
                .unwrap_or_else(|_| "0.0.0.0:8108".to_string())
                .parse::<SocketAddr>()?;
            let api_key = env::var("RAW_SEARCH_API_KEY").ok();
            let engine = RawSearchEngine::open(&index_path)?;
            youtube_search_service::serve(
                ServerConfig {
                    bind,
                    api_key,
                    raw_route: true,
                },
                Arc::new(engine),
            )
            .await?;
        }
        Some("slice") => {
            let input = required_path(args.next(), "input JSONL path")?;
            let output = required_path(args.next(), "output JSONL path")?;
            let max_videos = args
                .next()
                .ok_or("missing maximum video count")?
                .parse::<usize>()?;
            let report = slice_corpus(&input, &output, max_videos)?;
            println!("{}", serde_json::to_string_pretty(&report)?);
        }
        _ => {
            return Err("usage: youtube-raw-search build <chunks.jsonl> <channels.jsonl> <index-dir> | serve [index-dir] | slice <input.jsonl> <output.jsonl> <max-videos>".into());
        }
    }
    Ok(())
}

fn required_path(value: Option<String>, name: &str) -> Result<PathBuf, Box<dyn std::error::Error>> {
    value
        .map(PathBuf::from)
        .ok_or_else(|| format!("missing {name}").into())
}
